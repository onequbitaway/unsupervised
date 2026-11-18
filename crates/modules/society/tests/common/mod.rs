//! Shared support for the tests of the society module: a small fixture
//! plugin that stands in for the other modules, and short helpers.
//!
//! The fixture gives the world what the society module reads from others:
//! goods, sites, a blocking condition, and the events `crime.arrest_made`,
//! `economy.tenant_evicted`, `economy.lease_ended`, `economy.property_sold`
//! and `economy.property_foreclosed`, with the data of the real modules.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionReport, ActionStatus, SpawnSpec};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Builder, CityId, CondDef, ErrorCode, EventMeta, Holder, Kin, Money, NoArgs, OrgId, OwnerId,
    Plugin, Pos, Rect, ResidentId, Scope, Site, SiteId, VerbSpec, ZoneId,
};
use unsup_society::SocietyPlugin;

/// The data file of the fixture.
const DATA: &str = r#"
[[items]]
name = "bread"
weight = 1
tags = ["food"]
summary = "A loaf of bread."

[[items]]
name = "coal"
weight = 5
summary = "A lump of coal."
"#;

/// A home in zone 0.
pub const HOME: Rect = Rect::new(10, 10, 14, 14);
/// A warehouse in zone 0.
pub const DEPOT: Rect = Rect::new(40, 40, 44, 44);
/// An apartment in zone 3.
pub const FLAT: Rect = Rect::new(100, 100, 104, 104);
/// A shop in zone 0. Nobody can live in it.
pub const SHOP: Rect = Rect::new(50, 10, 54, 14);

/// The treasury of the only city of the test world.
pub const CITY: Holder = Holder::Org(OrgId::city(CityId(0)));

/// Arguments of `arrest_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct ArrestArgs {
    /// The resident that is arrested.
    target: ResidentId,
}

/// Arguments of `evict_test` and `end_lease_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct EvictArgs {
    /// The site.
    site: SiteId,
    /// The tenant that loses the site.
    tenant: ResidentId,
}

/// Arguments of `sell_test` and `foreclose_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct SellArgs {
    /// The site that the resident sells.
    site: SiteId,
}

/// Arguments of `die_test`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct DieArgs {
    /// The resident that inherits. The city treasury when absent.
    heir: Option<ResidentId>,
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
        b.load_data(DATA);
        let jailed = b.condition(CondDef::new("jailed").blocking().summary("In jail."));
        if b.scope() == Scope::Zone {
            b.on_worldgen(0, |realm, _gen| {
                let zone = realm.zone_info().expect("zone hooks run in zones").id;
                let sites: &[(&str, &str, Rect)] = match zone.0 {
                    0 => &[
                        ("home", "Home", HOME),
                        ("warehouse", "Depot", DEPOT),
                        ("shop", "Shop", SHOP),
                    ],
                    3 => &[("apartment", "Flat", FLAT)],
                    _ => &[],
                };
                for (kind, name, rect) in sites {
                    let id = SiteId(realm.alloc_id());
                    realm.sites_mut().insert(Site::new(id, *kind, *name, *rect));
                }
            });
        }
        b.instant_verb::<ArrestArgs>(spec("arrest_test"), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("crime.arrest_made", 50)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
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
        b.instant_verb::<EvictArgs>(spec("evict_test"), |realm, _ctx, args| {
            realm.emit(
                EventMeta::public("economy.tenant_evicted", 40),
                &json!({ "site": args.site, "tenant": args.tenant }),
            );
            Ok(Value::Null)
        });
        // A lease that ends without an eviction, as the economy module
        // reports it: the tenant left by itself.
        b.instant_verb::<EvictArgs>(spec("end_lease_test"), |realm, _ctx, args| {
            realm.emit(
                EventMeta::public("economy.lease_ended", 12),
                &json!({ "site": args.site, "tenant": args.tenant, "by": "tenant" }),
            );
            Ok(Value::Null)
        });
        // A foreclosure, as the economy module reports it: the city takes
        // the site from the resident.
        b.instant_verb::<SellArgs>(spec("foreclose_test"), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("economy.property_foreclosed", 55),
                &json!({
                    "site": args.site,
                    "city": 0,
                    "from": Holder::Resident(ctx.resident),
                    "reason": "loan_default",
                }),
            );
            Ok(Value::Null)
        });
        b.instant_verb::<SellArgs>(spec("sell_test"), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("economy.property_sold", 20),
                &json!({
                    "site": args.site,
                    "city": 0,
                    "from": Holder::Resident(ctx.resident),
                    "to": CITY,
                    "price": 1,
                }),
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
        b.instant_verb::<NoArgs>(spec("jail_me"), move |realm, ctx, _args| {
            realm.set_condition(ctx.idx, jailed, None);
            Ok(Value::Null)
        });
    }
}

static SOCIETY: SocietyPlugin = SocietyPlugin;
static FIXTURE: Fixture = Fixture;

/// The plugins of the test world.
pub fn plugins() -> [&'static dyn Plugin; 2] {
    [&SOCIETY, &FIXTURE]
}

/// Makes a test world with the society module and the fixture.
pub fn world() -> TestWorld<'static> {
    TestWorld::new(&plugins())
}

/// Spawns a resident of a given owner.
pub fn spawn_owned(w: &mut TestWorld<'_>, name: &str, owner: u64) -> ResidentId {
    let spec =
        SpawnSpec { owner: OwnerId(owner), name: name.into(), model: None, city: Some(CityId(0)) };
    w.world.spawn(spec).expect("the spawn is valid")
}

/// Spawns a resident with cash.
pub fn rich(w: &mut TestWorld<'_>, name: &str, cents: i64) -> ResidentId {
    let id = w.spawn(name);
    w.give_money(id, Money(cents));
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

/// Runs an action to its end and returns the result. A verb that finishes
/// at once answers in the acknowledgement; every other verb answers two
/// frames later, when the global realm has done its part.
pub fn run(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> Result<Value, ErrorCode> {
    let ack = w.act(r, verb, args);
    match ack.status {
        ActionStatus::Done { result } => Ok(result),
        ActionStatus::Rejected { error } => Err(error.code),
        _ => {
            w.step(2);
            let report = report_of(w, r, ack.action_id, verb);
            match report.error {
                None => Ok(report.result),
                Some(error) => Err(error.code),
            }
        }
    }
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

/// Founds an organization and returns its id. The founder needs the fee.
pub fn found(w: &mut TestWorld<'_>, founder: ResidentId, name: &str) -> OrgId {
    let result = ok(w, founder, "org_create", json!({ "name": name }));
    OrgId(result["org"].as_u64().expect("the result has the id"))
}

/// Invites a resident and lets it join.
pub fn enlist(w: &mut TestWorld<'_>, org: OrgId, by: ResidentId, who: ResidentId) {
    ok(w, by, "org_invite", json!({ "org": org, "resident": who }));
    ok(w, who, "org_join", json!({ "org": org }));
    w.step(1);
}

/// Returns true when the zone of a resident says that it has a permission.
pub fn can(w: &TestWorld<'_>, r: ResidentId, org: OrgId, perm: &str) -> bool {
    w.with_resident(r, |realm, idx| realm.can(idx, org, perm))
}

/// Returns the family links of a resident.
pub fn kin(w: &TestWorld<'_>, r: ResidentId) -> Kin {
    w.with_resident(r, |realm, idx| realm.col::<Kin>()[idx].clone())
}

/// Returns the id of the first site of a kind in a zone.
pub fn site(w: &TestWorld<'_>, zone: u32, kind: &str) -> SiteId {
    w.zone(ZoneId(zone)).sites().of_kind(kind).next().expect("the fixture made the site").id
}

/// Returns the tenants of a site.
pub fn tenants(w: &TestWorld<'_>, zone: u32, id: SiteId) -> Vec<ResidentId> {
    w.zone(ZoneId(zone)).sites().get(id).expect("the site exists").tenants.clone()
}

/// Makes a resident the owner of a site, as the economy module would.
pub fn give_site(w: &TestWorld<'_>, zone: u32, id: SiteId, owner: ResidentId) {
    let realm = w.zone(ZoneId(zone));
    realm.sites_mut().get_mut(id).expect("the site exists").owner = Some(Holder::Resident(owner));
}

/// Returns the kinds of the notices of a resident, oldest first.
pub fn notice_kinds(w: &TestWorld<'_>, r: ResidentId) -> Vec<String> {
    w.notices(r).into_iter().map(|notice| notice.kind).collect()
}

/// Returns how many notices of a kind a resident has.
pub fn count_notices(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> usize {
    w.notices(r).iter().filter(|notice| notice.kind == kind).count()
}

/// Returns the newest notice of a kind.
pub fn notice(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> unsup_kernel::protocol::Notice {
    w.notices(r)
        .into_iter()
        .rev()
        .find(|notice| notice.kind == kind)
        .unwrap_or_else(|| panic!("resident {r} has no notice `{kind}`"))
}

/// Returns the public events of a kind in a list of frames.
pub fn events<'a>(
    frames: &'a [unsup_kernel::protocol::FrameOutput],
    kind: &str,
) -> Vec<&'a unsup_kernel::Event> {
    frames.iter().flat_map(|frame| frame.events.iter()).filter(|e| e.kind == kind).collect()
}

/// A position in zone 3, far from everything in zone 0.
pub const FAR: Pos = Pos::new(110, 110);
