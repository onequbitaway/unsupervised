//! Shared support for the tests of the crime module.
//!
//! The tests run a world with every base module, as the server does: the
//! life module makes the map (shops, a jail, a police station), the
//! economy module gives goods and tills, the society module gives the
//! offices of the police and the judges.

#![allow(dead_code)]

use serde_json::{Value, json};
use unsup_crime::CrimePlugin;
use unsup_economy::EconomyPlugin;
use unsup_finance::FinancePlugin;
use unsup_kernel::protocol::{ActionReport, ActionStatus, Event, Notice};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityId, ErrorCode, Holder, Money, OrgId, Plugin, Pos, ResidentId, Site, SiteId, WorldConfig,
    ZoneId,
};
use unsup_life::LifePlugin;
use unsup_society::SocietyPlugin;

static LIFE: LifePlugin = LifePlugin;
static ECONOMY: EconomyPlugin = EconomyPlugin;
static FINANCE: FinancePlugin = FinancePlugin;
static SOCIETY: SocietyPlugin = SocietyPlugin;
static CRIME: CrimePlugin = CrimePlugin;

/// Every module, in the order of the server.
pub static PLUGINS: [&dyn Plugin; 5] = [&LIFE, &ECONOMY, &FINANCE, &SOCIETY, &CRIME];

/// The only city of the test world.
pub const CITY: CityId = CityId(0);

/// The treasury of the city.
pub const TREASURY: Holder = Holder::Org(OrgId::city(CITY));

/// Every crime and every investigation works.
pub const ALWAYS: (&str, i64) = ("crime.chance_min_bp", 10_000);
/// Every crime and every investigation fails.
pub const NEVER: (&str, i64) = ("crime.chance_max_bp", 0);
/// The city watch sees no crime.
pub const NO_WATCH: (&str, i64) = ("crime.watch_detect_bp", 0);
/// The city watch sees every crime.
pub const WATCH_SEES_ALL: (&str, i64) = ("crime.watch_detect_bp", 10_000);

/// Parameters with which no witness recognizes a perpetrator.
pub const BLIND: [(&str, i64); 4] = [
    ("crime.identify_base_bp", 0),
    ("crime.identify_fail_bonus_bp", 0),
    ("crime.identify_force_bonus_bp", 0),
    ("crime.patrol_detect_bp", 0),
];

/// Parameters with which every witness recognizes the perpetrator.
pub const SHARP: [(&str, i64); 5] = [
    ("crime.identify_base_bp", 10_000),
    ("crime.identify_stealth_bp", 0),
    ("crime.identify_night_malus_bp", 0),
    ("crime.identify_distance_bp", 0),
    ("crime.identify_asleep_malus_bp", 0),
];

/// Returns the configuration of a test world with changed parameters. When
/// a parameter is in the list two times, the last value counts.
pub fn config(seed: u64, params: &[(&str, i64)]) -> WorldConfig {
    let mut last: Vec<(&str, i64)> = Vec::new();
    for &(name, value) in params {
        last.retain(|(other, _)| *other != name);
        last.push((name, value));
    }
    let data: String = last
        .iter()
        .map(|(name, value)| format!("[params.\"{name}\"]\nvalue = {value}\n"))
        .collect();
    WorldConfig { data: vec![data], ..WorldConfig::with_seed(seed) }
}

/// Makes a test world with seed 0 and changed parameters. The first frames
/// run, so that every zone knows the law sites of the city.
pub fn town(params: &[(&str, i64)]) -> TestWorld<'static> {
    let mut world = TestWorld::with_config(&PLUGINS, config(0, params));
    world.step(2);
    world
}

/// Joins lists of parameters.
pub fn with(lists: &[&[(&'static str, i64)]]) -> Vec<(&'static str, i64)> {
    lists.iter().flat_map(|list| list.iter().copied()).collect()
}

/// Returns the sites of a kind, by zone and then by id.
pub fn sites(world: &TestWorld<'_>, kind: &str) -> Vec<Site> {
    let mut found = Vec::new();
    for zone in world.world.layout().zones() {
        found.extend(world.zone(zone.id).sites().of_kind(kind).cloned());
    }
    found
}

/// Returns the first site of a kind.
pub fn site(world: &TestWorld<'_>, kind: &str) -> Site {
    sites(world, kind).into_iter().next().unwrap_or_else(|| panic!("the city has no {kind}"))
}

/// Returns a site as it is now.
pub fn site_now(world: &TestWorld<'_>, id: SiteId) -> Site {
    for zone in world.world.layout().zones() {
        if let Some(site) = world.zone(zone.id).sites().get(id) {
            return site.clone();
        }
    }
    panic!("no zone has site {id}");
}

/// Returns the zone of a position.
pub fn zone_at(world: &TestWorld<'_>, pos: Pos) -> ZoneId {
    world.world.layout().zone_at(pos).expect("the position is in the world")
}

/// Spawns a resident with cash in the middle of a site.
pub fn resident_in(world: &mut TestWorld<'_>, name: &str, cash: i64, place: &Site) -> ResidentId {
    resident_at(world, name, cash, place.rect.centre())
}

/// Spawns a resident with cash at a position.
pub fn resident_at(world: &mut TestWorld<'_>, name: &str, cash: i64, pos: Pos) -> ResidentId {
    let resident = world.spawn(name);
    world.give_money(resident, Money(cash));
    world.set_pos(resident, pos);
    resident
}

/// Spawns a resident of another owner than the default owner 1, with cash,
/// at a position. A juror must not have the owner of the defendant.
pub fn stranger_at(
    world: &mut TestWorld<'_>,
    name: &str,
    owner: u64,
    cash: i64,
    pos: Pos,
) -> ResidentId {
    let spec = unsup_kernel::protocol::SpawnSpec {
        owner: unsup_kernel::OwnerId(owner),
        name: name.to_string(),
        model: None,
        city: Some(CITY),
    };
    let resident = world.world.spawn(spec).expect("the spawn works");
    world.give_money(resident, Money(cash));
    world.set_pos(resident, pos);
    resident
}

/// Puts money into the till of a site, as a test fixture.
pub fn fill_till(world: &mut TestWorld<'_>, place: &Site, cents: i64) {
    let faucet = world.world.blueprint().faucet_id("kernel.grant").expect("the kernel faucet");
    let zone = zone_at(world, place.rect.centre());
    world.zone(zone).ledger_mut().mint(faucet, Holder::Site(place.id), Money(cents));
}

/// Puts goods into the stock of a site, as a test fixture.
pub fn fill_stock(world: &mut TestWorld<'_>, place: &Site, item: &str, qty: i64) {
    let blueprint = world.world.blueprint();
    let faucet = blueprint.faucet_id("kernel.grant").expect("the kernel faucet");
    let id = blueprint.item_id(item).unwrap_or_else(|| panic!("no item is named `{item}`"));
    let zone = zone_at(world, place.rect.centre());
    world.zone(zone).ledger_mut().produce(faucet, Holder::Site(place.id), id, qty);
}

/// Gives residents an office of the city government with some
/// permissions, as the politics module does.
pub fn appoint(world: &mut TestWorld<'_>, role: &str, perms: &[&str], holders: &[ResidentId]) {
    // The global realm learns about a new resident in its next tick.
    world.step(1);
    let org = OrgId::city(CITY);
    unsup_society::offices::define_role(world.global(), org, role, perms).expect("a valid role");
    unsup_society::offices::set_holders(world.global(), org, role, holders)
        .expect("the holders are alive");
    world.world.settle();
    world.step(1);
}

/// Returns true when the resident is idle with an empty queue.
pub fn is_idle(world: &TestWorld<'_>, r: ResidentId) -> bool {
    world.with_resident(r, |realm, idx| realm.activity(idx).is_none() && realm.queue_len(idx) == 0)
}

fn find_report(world: &TestWorld<'_>, r: ResidentId, action: u64) -> Option<ActionReport> {
    world.reports(r).into_iter().find(|report| report.action_id == action)
}

/// Sends an action that must be accepted, runs the world until its report
/// is there and returns the report with the public events of those frames.
pub fn run_with_events(
    world: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> (ActionReport, Vec<Event>) {
    let ack = world.act(r, verb, args);
    let mut events = Vec::new();
    match ack.status {
        ActionStatus::Rejected { error } => panic!("`{verb}` was rejected: {error}"),
        ActionStatus::Done { result } => {
            let report = ActionReport {
                seq: 0,
                action_id: ack.action_id,
                request_id: None,
                verb: verb.to_string(),
                tick: unsup_kernel::Tick(0),
                ok: true,
                result,
                error: None,
            };
            return (report, events);
        }
        _ => {}
    }
    for _ in 0..3_000 {
        if let Some(report) = find_report(world, r, ack.action_id) {
            return (report, events);
        }
        events.extend(world.step(1).into_iter().flat_map(|frame| frame.events));
    }
    panic!("`{verb}` of resident {r} wrote no report");
}

/// Like [`run_with_events`], without the events.
pub fn run(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ActionReport {
    run_with_events(world, r, verb, args).0
}

/// Runs an action that must end well and returns its result.
pub fn ok(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    let report = run(world, r, verb, args);
    assert!(report.ok, "`{verb}` failed: {:?}", report.error);
    report.result
}

/// Runs an action that must fail, at once or in its report, and returns
/// the error code.
pub fn fail(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ErrorCode {
    let ack = world.act(r, verb, args);
    match ack.status {
        ActionStatus::Rejected { error } => error.code,
        ActionStatus::Done { result } => panic!("`{verb}` ended well: {result}"),
        _ => {
            for _ in 0..3_000 {
                if let Some(report) = find_report(world, r, ack.action_id) {
                    let error = report
                        .error
                        .unwrap_or_else(|| panic!("`{verb}` ended well: {}", report.result));
                    return error.code;
                }
                world.step(1);
            }
            panic!("`{verb}` of resident {r} wrote no report");
        }
    }
}

/// Runs frames and returns their public events.
pub fn step_events(world: &mut TestWorld<'_>, frames: u32) -> Vec<Event> {
    world.step(frames).into_iter().flat_map(|frame| frame.events).collect()
}

/// Returns the events of a kind.
pub fn of_kind<'a>(events: &'a [Event], kind: &str) -> Vec<&'a Event> {
    events.iter().filter(|event| event.kind == kind).collect()
}

/// Returns the newest notice of a kind in the inbox of a resident.
pub fn notice(world: &TestWorld<'_>, r: ResidentId, kind: &str) -> Option<Notice> {
    world.notices(r).into_iter().rev().find(|notice| notice.kind == kind)
}

/// Returns the notices of a kind in the inbox of a resident, oldest first.
pub fn notices(world: &TestWorld<'_>, r: ResidentId, kind: &str) -> Vec<Notice> {
    world.notices(r).into_iter().filter(|notice| notice.kind == kind).collect()
}

/// Returns the frame of the first event of a kind.
pub fn frame_of(events: &[Event], kind: &str) -> u64 {
    events.iter().find(|event| event.kind == kind).unwrap_or_else(|| panic!("no {kind}")).frame
}

/// Returns the `law` section of the observation of a resident.
pub fn law(world: &TestWorld<'_>, r: ResidentId) -> Option<Value> {
    world.observe(r).sections.get("law").cloned()
}

/// Returns the answer of the free verb `case_view`.
pub fn case_view(world: &mut TestWorld<'_>, reader: ResidentId, case: u64) -> Value {
    ok(world, reader, "case_view", json!({ "case": case }))
}

/// Returns a number of a JSON object.
pub fn num(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or_else(|| panic!("no number `{key}` in {value}"))
}

/// Returns the case number of the result of a crime.
pub fn case_of(result: &Value) -> u64 {
    result.get("case").and_then(Value::as_u64).unwrap_or_else(|| panic!("no case in {result}"))
}

/// Returns the tick of the zone of a resident.
pub fn tick_of(world: &TestWorld<'_>, r: ResidentId) -> u64 {
    world.with_resident(r, |realm, _| realm.tick().0)
}

/// Returns the tick at which the condition `jailed` of a resident ends.
pub fn jailed_until(world: &TestWorld<'_>, r: ResidentId) -> Option<u64> {
    let jailed = world.world.blueprint().condition_id("jailed").expect("the condition");
    world.with_resident(r, |realm, idx| realm.condition_until(idx, jailed).flatten().map(|t| t.0))
}

/// Returns the kind of the site where a resident stands.
pub fn site_kind_of(world: &TestWorld<'_>, r: ResidentId) -> Option<String> {
    world.with_resident(r, |realm, idx| realm.sites().at(realm.pos(idx)).map(|s| s.kind.clone()))
}
