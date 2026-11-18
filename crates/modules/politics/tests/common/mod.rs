//! Shared support for the tests of the politics module: worlds, short
//! helpers for actions and time, and a way to seat officials without an
//! election.

#![allow(dead_code)]

use serde_json::{Value, json};
use unsup_finance::FinancePlugin;
use unsup_kernel::protocol::{ActionReport, ActionStatus, FrameOutput, Notice, SpawnSpec};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityConfig, CityId, ErrorCode, Event, GRANT_FAUCET, Holder, Money, OrgId, OwnerId, Plugin, Pos,
    ResidentId, WorldConfig, ZoneId,
};
use unsup_politics::PoliticsPlugin;
use unsup_society::SocietyPlugin;

static SOCIETY: SocietyPlugin = SocietyPlugin;
static FINANCE: FinancePlugin = FinancePlugin;
static POLITICS: PoliticsPlugin = PoliticsPlugin;

/// Game minutes in one game day.
pub const DAY: u64 = 1_440;
/// The government of the first city.
pub const CITY: OrgId = OrgId::city(CityId(0));
/// The government of the second city of a world with two cities.
pub const CITY_B: OrgId = OrgId::city(CityId(1));
/// A position in the first zone of the first city.
pub const SQUARE: Pos = Pos::new(20, 20);
/// A position in the second city of a world with two cities.
pub const SQUARE_B: Pos = Pos::new(1_020, 20);

/// The fee of a candidate in the data file.
pub const FEE: i64 = 5_000;

/// The plugins of the test world: the society module, which this module
/// needs, and the politics module.
pub fn plugins() -> [&'static dyn Plugin; 2] {
    [&SOCIETY, &POLITICS]
}

/// The plugins of a test world that also has the finance module.
pub fn plugins_with_finance() -> [&'static dyn Plugin; 3] {
    [&SOCIETY, &FINANCE, &POLITICS]
}

/// Returns a data file that sets parameters of this module.
pub fn overrides(values: &[(&str, i64)]) -> String {
    values.iter().map(|(name, value)| format!("[params.\"{name}\"]\nvalue = {value}\n")).collect()
}

/// A configuration with one city and the calendar of the data file.
pub fn default_config(seed: u64) -> WorldConfig {
    WorldConfig::with_seed(seed)
}

/// A configuration whose first elections are far away, for the tests that
/// seat officials by hand. Residents can vote and sign from their first
/// minute.
pub fn quiet_config() -> WorldConfig {
    WorldConfig {
        data: vec![overrides(&[
            ("politics.city_first_vote_day", 300),
            ("politics.nation_first_vote_day", 300),
            ("politics.voter_min_age", 0),
        ])],
        ..WorldConfig::default()
    }
}

/// Adds a second city to a configuration, 1,000 tiles to the east.
pub fn with_two_cities(mut config: WorldConfig) -> WorldConfig {
    config.cities = vec![
        CityConfig::new("Ashford", Pos::new(0, 0), 2),
        CityConfig::new("Brine", Pos::new(1_000, 0), 1),
    ];
    config
}

/// Makes a test world with the calendar of the data file.
pub fn world() -> TestWorld<'static> {
    TestWorld::with_config(&plugins(), default_config(0))
}

/// Makes a test world whose first elections are far away.
pub fn quiet_world() -> TestWorld<'static> {
    TestWorld::with_config(&plugins(), quiet_config())
}

/// Spawns a resident with cash at a position.
pub fn rich_at(w: &mut TestWorld<'_>, name: &str, cents: i64, pos: Pos) -> ResidentId {
    let id = w.spawn_at(name, pos);
    if cents > 0 {
        w.give_money(id, Money(cents));
    }
    id
}

/// Spawns a resident with cash on the square of the first city.
pub fn rich(w: &mut TestWorld<'_>, name: &str, cents: i64) -> ResidentId {
    rich_at(w, name, cents, SQUARE)
}

/// Spawns a resident of a given owner on the square of the first city.
pub fn spawn_owned(w: &mut TestWorld<'_>, name: &str, owner: u64) -> ResidentId {
    let spec =
        SpawnSpec { owner: OwnerId(owner), name: name.into(), model: None, city: Some(CityId(0)) };
    let id = w.world.spawn(spec).expect("the spawn is valid");
    w.set_pos(id, SQUARE);
    id
}

/// Returns the report of an action, or panics.
fn report_of(w: &TestWorld<'_>, r: ResidentId, action_id: u64, verb: &str) -> ActionReport {
    w.reports(r)
        .into_iter()
        .rev()
        .find(|report| report.action_id == action_id)
        .unwrap_or_else(|| panic!("`{verb}` of resident {r} has no report yet"))
}

/// Runs an action to its end and returns the result. A verb of this module
/// answers two frames later, when the global realm has done its part.
/// `frames` is the number of frames to wait.
pub fn run_for(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
    frames: u32,
) -> Result<Value, ErrorCode> {
    let ack = w.act(r, verb, args);
    match ack.status {
        ActionStatus::Done { result } => Ok(result),
        ActionStatus::Rejected { error } => Err(error.code),
        _ => {
            w.step(frames);
            let report = report_of(w, r, ack.action_id, verb);
            match report.error {
                None => Ok(report.result),
                Some(error) => Err(error.code),
            }
        }
    }
}

/// Runs an action that the test expects to fail, and returns the whole
/// error.
pub fn error_of(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> unsup_kernel::ActionError {
    let ack = w.act(r, verb, args);
    match ack.status {
        ActionStatus::Done { result } => panic!("`{verb}` of resident {r} succeeded with {result}"),
        ActionStatus::Rejected { error } => error,
        _ => {
            w.step(2);
            let report = report_of(w, r, ack.action_id, verb);
            report.error.unwrap_or_else(|| {
                panic!("`{verb}` of resident {r} succeeded with {}", report.result)
            })
        }
    }
}

/// Runs an action that answers in two frames.
pub fn run(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> Result<Value, ErrorCode> {
    run_for(w, r, verb, args, 2)
}

/// Runs an action that the test expects to succeed.
pub fn ok(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    match run(w, r, verb, args) {
        Ok(result) => result,
        Err(code) => panic!("`{verb}` of resident {r} failed with `{code}`"),
    }
}

/// Runs an action that the test expects to fail, and returns the code.
pub fn fail(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ErrorCode {
    match run(w, r, verb, args) {
        Ok(result) => panic!("`{verb}` of resident {r} succeeded with {result}"),
        Err(code) => code,
    }
}

/// Runs frames until the world is at `frame`, and returns what they
/// changed. It panics when the world is past the frame.
pub fn step_to(w: &mut TestWorld<'_>, frame: u64) -> Vec<FrameOutput> {
    let now = w.world.frame();
    assert!(now <= frame, "the world is at frame {now}, past frame {frame}");
    w.step(u32::try_from(frame - now).expect("a short test"))
}

/// Returns the frame of a minute of a day. `day` counts from 1, like the
/// clock.
pub fn at(day: u64, minute: u64) -> u64 {
    (day - 1) * DAY + minute
}

/// Returns the public events of a kind in a list of frames.
pub fn events<'a>(frames: &'a [FrameOutput], kind: &str) -> Vec<&'a Event> {
    frames.iter().flat_map(|frame| frame.events.iter()).filter(|e| e.kind == kind).collect()
}

/// Returns the only public event of a kind in a list of frames.
pub fn event<'a>(frames: &'a [FrameOutput], kind: &str) -> &'a Event {
    let found = events(frames, kind);
    assert_eq!(found.len(), 1, "events of the kind `{kind}`");
    found[0]
}

/// Returns the newest published state of a kind and key in a list of
/// frames.
pub fn published<'a>(frames: &'a [FrameOutput], kind: &str, key: &Value) -> Option<&'a Value> {
    events(frames, kind).into_iter().rev().find(|e| &e.data["key"] == key).map(|e| &e.data)
}

/// Returns the notices of a kind of a resident, oldest first.
pub fn notices(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> Vec<Notice> {
    w.notices(r).into_iter().filter(|notice| notice.kind == kind).collect()
}

/// Returns true when the zone of a resident says that it has a permission.
pub fn can(w: &TestWorld<'_>, r: ResidentId, org: OrgId, perm: &str) -> bool {
    w.with_resident(r, |realm, idx| realm.can(idx, org, perm))
}

/// Returns the cents in the treasury of an organization.
pub fn treasury(w: &TestWorld<'_>, org: OrgId) -> i64 {
    w.money(Holder::Org(org)).cents()
}

/// Puts money in the treasury of an organization through the test faucet.
pub fn fund_treasury(w: &mut TestWorld<'_>, org: OrgId, cents: i64) {
    let faucet = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    w.global().ledger_mut().mint(faucet, Holder::Org(org), Money(cents));
}

/// Makes residents the holders of a role of a government organization,
/// with the function of the society module that the politics module uses
/// itself.
///
/// The helper runs one frame before, so that the global realm knows the
/// residents that the test just made, and one frame after, so that the
/// zones have the offices.
pub fn seat(w: &mut TestWorld<'_>, org: OrgId, role: &str, holders: &[ResidentId]) {
    w.step(1);
    unsup_society::offices::set_holders(w.global(), org, role, holders)
        .unwrap_or_else(|error| panic!("cannot seat `{role}`: {error}"));
    w.world.settle();
    w.step(1);
}

/// Returns the residents that hold a role of a government organization.
pub fn holders(w: &TestWorld<'_>, org: OrgId, role: &str) -> Vec<ResidentId> {
    unsup_society::offices::holders(w.global(), org, role)
}

/// Returns the `politics` section of the observation of a resident.
pub fn section(w: &TestWorld<'_>, r: ResidentId) -> Value {
    w.observe(r).sections.get("politics").cloned().unwrap_or(Value::Null)
}

/// Returns the id of the open election for an office that a resident sees
/// in its observation.
pub fn election_of(w: &TestWorld<'_>, r: ResidentId, office: &str) -> u64 {
    let section = section(w, r);
    section["elections"]
        .as_array()
        .and_then(|list| list.iter().find(|entry| entry["office"] == office))
        .and_then(|entry| entry["election"].as_u64())
        .unwrap_or_else(|| panic!("resident {r} sees no open election for `{office}`: {section}"))
}

/// Registers a resident as a candidate with a platform.
pub fn register(w: &mut TestWorld<'_>, r: ResidentId, election: u64, platform: &str) -> Value {
    ok(w, r, "register_candidate", json!({ "election": election, "platform": platform }))
}

/// Casts a vote.
pub fn vote(w: &mut TestWorld<'_>, r: ResidentId, election: u64, candidate: ResidentId) -> Value {
    ok(w, r, "vote", json!({ "election": election, "candidate": candidate }))
}

/// Returns the value of a parameter in a zone.
pub fn zone_param(w: &TestWorld<'_>, zone: u32, name: &str) -> i64 {
    let param = w.world.blueprint().param_id(name).expect("the parameter exists");
    w.zone(ZoneId(zone)).param(param)
}
