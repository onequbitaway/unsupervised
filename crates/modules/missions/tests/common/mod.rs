//! Shared support for the tests of the missions module: a small fixture
//! plugin that stands in for the other modules, and short helpers.
//!
//! The fixture gives the world what the missions module reads from others:
//! sites with a value, memberships, and the events
//! `economy.goods_produced`, `economy.shift_completed`, `crime.arrest_made`,
//! `life.resident_died`, `society.org_left` and `society.said`.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionReport, ActionStatus, FrameOutput, Notice, SpawnSpec};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, CityId, CondDef, Dest, Envelope, ErrorCode, Event, EventMeta, GRANT_FAUCET, Holder,
    Money, NoArgs, OrgId, OwnerId, Plugin, Pos, RealmId, Rect, ResidentId, Scope, Site, SiteId,
    VerbSpec, ZoneId,
};
use unsup_missions::{MissionSpec, MissionsPlugin, SponsorMission};

/// A home in zone 0.
pub const HOME: Rect = Rect::new(10, 10, 14, 14);
/// The value of the home in cents.
pub const HOME_VALUE: i64 = 30_000;
/// A shop in zone 0.
pub const SHOP: Rect = Rect::new(50, 10, 54, 14);
/// The value of the shop in cents.
pub const SHOP_VALUE: i64 = 50_000;
/// A farm in zone 3.
pub const FARM: Rect = Rect::new(100, 100, 104, 104);
/// The value of the farm in cents.
pub const FARM_VALUE: i64 = 20_000;
/// A position in zone 3, far from everything in zone 0.
pub const FAR: Pos = Pos::new(110, 110);

/// The treasury of the only city of the test world.
pub const CITY: Holder = Holder::Org(OrgId::city(CityId(0)));

/// Arguments of `produce_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct ProduceArgs {
    /// Name of the good.
    item: String,
    /// Number of units.
    qty: i64,
}

/// Arguments of `arrest_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct ArrestArgs {
    /// The resident that is arrested.
    target: ResidentId,
}

/// Arguments of `die_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct DieArgs {
    /// The resident that inherits. The city treasury when absent.
    heir: Option<ResidentId>,
}

/// Arguments of `join_test` and `leave_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct OrgArgs {
    /// The organization.
    org: OrgId,
}

fn spec(verb: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "fixture", summary: "A test verb.", free: true, allowed_in: &[] }
}

/// The fixture plugin.
pub struct Fixture;

impl Plugin for Fixture {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn build(&self, b: &mut Builder) {
        let jailed = b.condition(CondDef::new("jailed").blocking().summary("In jail."));
        if b.scope() == Scope::Zone {
            b.on_worldgen(0, |realm, _gen| {
                let zone = realm.zone_info().expect("zone hooks run in zones").id;
                let sites: &[(&str, Rect, i64)] = match zone.0 {
                    0 => &[("home", HOME, HOME_VALUE), ("shop", SHOP, SHOP_VALUE)],
                    3 => &[("farm", FARM, FARM_VALUE)],
                    _ => &[],
                };
                for (kind, rect, value) in sites {
                    let id = SiteId(realm.alloc_id());
                    let site = Site::new(id, *kind, *kind, *rect)
                        .with_owner(CITY)
                        .with_attr("value", *value);
                    realm.sites_mut().insert(site);
                }
            });
        }
        // A good comes out of a recipe, as the economy module reports it.
        b.instant_verb::<ProduceArgs>(spec("produce_test"), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("economy.goods_produced", 2)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "resident": ctx.resident, "site": 7, "item": args.item, "qty": args.qty }),
            );
            Ok(Value::Null)
        });
        b.instant_verb::<NoArgs>(spec("shift_test"), |realm, ctx, _args| {
            realm.emit(
                EventMeta::public("economy.shift_completed", 2).actor(ctx.resident),
                &json!({ "resident": ctx.resident, "site": 7, "wage": 500, "tax": 50, "skill": "labor" }),
            );
            Ok(Value::Null)
        });
        b.instant_verb::<ArrestArgs>(spec("arrest_test"), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("crime.arrest_made", 50).actor(ctx.resident),
                // The data of the crime module: an arrest by an officer for
                // one case.
                &json!({
                    "officer": ctx.resident,
                    "target": args.target,
                    "case": 7,
                    "cases": [7],
                    "city": 0,
                    "watch": false,
                }),
            );
            Ok(Value::Null)
        });
        // Speech: an event that no mission can count.
        b.instant_verb::<NoArgs>(spec("say_test"), |realm, ctx, _args| {
            realm.emit(
                EventMeta::public("society.said", 1).actor(ctx.resident),
                &json!({ "resident": ctx.resident }),
            );
            Ok(Value::Null)
        });
        // A death as the life module reports it: the event names the heir,
        // then the resident leaves the world and the heir gets its holdings.
        b.instant_verb::<DieArgs>(spec("die_test"), |realm, ctx, args| {
            let heir = args.heir.map_or(CITY, Holder::Resident);
            realm.emit(
                EventMeta::public("life.resident_died", 80).actor(ctx.resident),
                &json!({ "resident": ctx.resident, "cause": "test", "age": 70, "heir": heir, "estate": 0 }),
            );
            realm.despawn(ctx.idx, heir, "test");
            Ok(Value::Null)
        });
        // The society module is the writer of memberships. The fixture
        // stands in for it.
        b.instant_verb::<OrgArgs>(spec("join_test"), |realm, ctx, args| {
            realm.grant(ctx.resident, args.org, "member", &[]);
            Ok(Value::Null)
        });
        b.instant_verb::<OrgArgs>(spec("leave_test"), |realm, ctx, args| {
            realm.revoke(ctx.resident, args.org);
            realm.emit(
                EventMeta::public("society.org_left", 5).actor(ctx.resident),
                &json!({ "org": args.org, "resident": ctx.resident, "cause": "left" }),
            );
            Ok(Value::Null)
        });
        b.instant_verb::<NoArgs>(spec("jail_me"), move |realm, ctx, _args| {
            realm.set_condition(ctx.idx, jailed, None);
            Ok(Value::Null)
        });
    }
}

static MISSIONS: MissionsPlugin = MissionsPlugin;
static FIXTURE: Fixture = Fixture;

/// The plugins of the test world.
pub fn plugins() -> [&'static dyn Plugin; 2] {
    [&MISSIONS, &FIXTURE]
}

/// Makes a test world with the missions module and the fixture. Residents
/// can use many free test verbs in one tick.
pub fn world() -> TestWorld<'static> {
    let mut w = TestWorld::new(&plugins());
    w.set_param("kernel.free_actions_per_tick", 100);
    w
}

/// Spawns a resident of a given owner, with a declared model or without.
pub fn spawn_as(w: &mut TestWorld<'_>, name: &str, owner: u64, model: Option<&str>) -> ResidentId {
    let spec = SpawnSpec {
        owner: OwnerId(owner),
        name: name.into(),
        model: model.map(str::to_string),
        city: Some(CityId(0)),
    };
    w.world.spawn(spec).expect("the spawn is valid")
}

/// Spawns a resident with its own owner and with cash.
pub fn rich(w: &mut TestWorld<'_>, name: &str, owner: u64, cents: i64) -> ResidentId {
    let id = spawn_as(w, name, owner, None);
    w.give_money(id, Money(cents));
    id
}

/// Puts money on the bank account of a resident, as the finance module
/// keeps it: `Holder::Account(resident id)` in the global realm.
pub fn give_deposit(w: &TestWorld<'_>, r: ResidentId, cents: i64) {
    let faucet = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet exists");
    w.global().ledger_mut().mint(faucet, Holder::Account(r.0), Money(cents));
}

/// Returns the id of the first site of a kind in a zone.
pub fn site(w: &TestWorld<'_>, zone: u32, kind: &str) -> SiteId {
    w.zone(ZoneId(zone)).sites().of_kind(kind).next().expect("the fixture made the site").id
}

/// Makes a resident the owner of a site, as the economy module would.
pub fn give_site(w: &TestWorld<'_>, zone: u32, id: SiteId, owner: ResidentId) {
    let realm = w.zone(ZoneId(zone));
    realm.sites_mut().get_mut(id).expect("the site exists").owner = Some(Holder::Resident(owner));
}

/// Makes a resident a member of an organization. The zone knows it at once.
pub fn join(w: &mut TestWorld<'_>, r: ResidentId, org: u64) {
    w.act_ok(r, "join_test", json!({ "org": org }));
}

/// Returns the report of an action, or panics.
fn report_of(w: &TestWorld<'_>, r: ResidentId, action_id: u64, verb: &str) -> ActionReport {
    w.reports(r)
        .into_iter()
        .rev()
        .find(|report| report.action_id == action_id)
        .unwrap_or_else(|| panic!("`{verb}` of resident {r} has no report yet"))
}

/// Runs an action to its end and returns the result or the whole error. A
/// verb that finishes at once answers in the acknowledgement; every other
/// verb answers two frames later, when the global realm has done its part.
pub fn run_full(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> Result<Value, unsup_kernel::ActionError> {
    let ack = w.act(r, verb, args);
    match ack.status {
        ActionStatus::Done { result } => Ok(result),
        ActionStatus::Rejected { error } => Err(error),
        _ => {
            w.step(2);
            let report = report_of(w, r, ack.action_id, verb);
            match report.error {
                None => Ok(report.result),
                Some(error) => Err(error),
            }
        }
    }
}

/// Runs an action that the test expects to succeed.
pub fn ok(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    match run_full(w, r, verb, args) {
        Ok(result) => result,
        Err(error) => panic!("`{verb}` of resident {r} failed: {error}"),
    }
}

/// Runs an action that the test expects to fail, and returns the code.
pub fn fail(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ErrorCode {
    match run_full(w, r, verb, args) {
        Ok(result) => panic!("`{verb}` of resident {r} succeeded with {result}"),
        Err(error) => error.code,
    }
}

/// Runs an action that the test expects to fail, and returns the message.
pub fn fail_message(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> String {
    match run_full(w, r, verb, args) {
        Ok(result) => panic!("`{verb}` of resident {r} succeeded with {result}"),
        Err(error) => error.message,
    }
}

/// The definition of a small solo mission: produce `count` loaves of bread.
pub fn bread_mission(reward: i64, count: u32) -> Value {
    json!({
        "title": "Bake bread",
        "text": "The town needs bread.",
        "objectives": [
            { "event": "economy.goods_produced", "fields": { "item": "bread" }, "count": count }
        ],
        "reward": reward,
        "deadline_in": 600,
    })
}

/// Publishes a mission and returns its id.
pub fn post(w: &mut TestWorld<'_>, poster: ResidentId, definition: Value) -> u64 {
    let result = ok(w, poster, "mission_post", definition);
    result["mission"].as_u64().expect("the result has the id")
}

/// Lets a resident produce one loaf of bread.
pub fn bake(w: &mut TestWorld<'_>, r: ResidentId) {
    w.act_ok(r, "produce_test", json!({ "item": "bread", "qty": 1 }));
}

/// Sends the mission of a sponsor to the global realm, as the server does
/// with `SimHandle::send_global`: one `Input::Deliver`.
pub fn sponsor(w: &mut TestWorld<'_>, request: u64, name: &str, definition: Value) {
    let mission: MissionSpec = serde_json::from_value(definition).expect("a mission definition");
    let message = SponsorMission { request, sponsor: name.into(), mission };
    let envelope = Envelope::module(RealmId::Global, Dest::Global, &message);
    // The envelope crosses a process border as bytes.
    let envelope = Envelope::from_bytes(&envelope.to_bytes()).expect("the envelope reads back");
    w.world.deliver_external(vec![envelope]);
}

/// Returns the public events of a kind in a list of frames.
pub fn events<'a>(frames: &'a [FrameOutput], kind: &str) -> Vec<&'a Event> {
    frames.iter().flat_map(|frame| frame.events.iter()).filter(|e| e.kind == kind).collect()
}

/// Returns the data of the one event of a kind in a list of frames.
pub fn one_event(frames: &[FrameOutput], kind: &str) -> Value {
    let found = events(frames, kind);
    assert_eq!(found.len(), 1, "one `{kind}` event");
    assert!(found[0].public);
    found[0].data.clone()
}

/// Returns the newest notice of a kind.
pub fn notice(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> Notice {
    w.notices(r)
        .into_iter()
        .rev()
        .find(|notice| notice.kind == kind)
        .unwrap_or_else(|| panic!("resident {r} has no notice `{kind}`"))
}

/// Returns how many notices of a kind a resident has.
pub fn count_notices(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> usize {
    w.notices(r).iter().filter(|notice| notice.kind == kind).count()
}

/// Returns the money in the escrow of a mission.
pub fn escrow(w: &TestWorld<'_>, mission: u64) -> i64 {
    w.money(Holder::Escrow(mission)).cents()
}

/// Returns the cents that a sink of the missions module has destroyed, in
/// all realms.
pub fn burned(w: &TestWorld<'_>, sink: &str) -> i64 {
    let id = w.world.blueprint().sink_id(sink).expect("the sink exists");
    let zones = w.world.layout().zones().iter().map(|zone| RealmId::Zone(zone.id));
    std::iter::once(RealmId::Global)
        .chain(zones)
        .filter_map(|realm| w.world.realm(realm))
        .map(|realm| realm.ledger().burned_by(id).cents())
        .sum()
}

/// Returns the cents that a faucet of the missions module has created.
pub fn minted(w: &TestWorld<'_>, faucet: &str) -> i64 {
    let id = w.world.blueprint().faucet_id(faucet).expect("the faucet exists");
    w.global().ledger().minted_by(id).cents()
}

/// Returns the observation section `missions` of a resident, if it has one.
pub fn section(w: &TestWorld<'_>, r: ResidentId) -> Option<Value> {
    w.observe(r).sections.get("missions").cloned()
}
