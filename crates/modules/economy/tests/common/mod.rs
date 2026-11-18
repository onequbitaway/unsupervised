//! A small town for the tests of the economy module.
//!
//! The life module makes the sites of a real world. The tests make their
//! own through the kernel, with the contract of
//! `docs/design/game-design.md`, section 9: the city owns every site, the
//! attribute `value` is the price, and a public building has the tag
//! `public`.

#![allow(dead_code)]

use std::collections::BTreeMap;

use serde_json::{Value, json};
use unsup_economy::EconomyPlugin;
use unsup_kernel::protocol::{ActionReport, ActionStatus, Event, FrameOutput};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    ActionError, CityId, ErrorCode, Holder, Money, OrgId, Pos, RealmId, Rect, ResidentId, Site,
    SiteId, ZoneId,
};

/// The plugin under test.
pub static ECONOMY: EconomyPlugin = EconomyPlugin;

/// The only city of the test world.
pub const CITY: CityId = CityId(0);

/// Ticks in one game day.
pub const DAY: u32 = 1_440;

/// The treasury of the city.
pub fn city() -> Holder {
    Holder::Org(OrgId::city(CITY))
}

/// The sites of the town.
#[derive(Clone, Copy, Debug)]
pub struct Sites {
    pub farm: SiteId,
    pub mine: SiteId,
    pub forest: SiteId,
    pub workshop: SiteId,
    pub factory: SiteId,
    pub restaurant: SiteId,
    pub shop: SiteId,
    pub home: SiteId,
    pub apartment: SiteId,
    pub city_hall: SiteId,
    pub depot: SiteId,
    pub office: SiteId,
    pub park: SiteId,
    /// A home in the second zone of the city.
    pub far_home: SiteId,
    /// A shop in the second zone of the city.
    pub far_shop: SiteId,
    /// A city hall in the second zone of the city.
    pub far_hall: SiteId,
}

/// A test world with one site of every kind that the economy uses.
pub struct Town {
    pub world: TestWorld<'static>,
    pub at: Sites,
}

/// Values of the sites of the town, in cents.
pub const HOME_VALUE: i64 = 200_000;
pub const APARTMENT_VALUE: i64 = 800_000;
pub const SHOP_VALUE: i64 = 500_000;
pub const FARM_VALUE: i64 = 300_000;
pub const WORKSHOP_VALUE: i64 = 400_000;

/// Adds a site that the first city owns. Returns its id.
pub fn add_site(
    world: &mut TestWorld<'static>,
    kind: &str,
    corner: (i32, i32),
    value: i64,
    public: bool,
) -> SiteId {
    add_site_of(world, city(), kind, corner, value, public)
}

/// Adds a site with an owner, as the life module does for the city of the
/// site. Returns its id.
pub fn add_site_of(
    world: &mut TestWorld<'static>,
    owner: Holder,
    kind: &str,
    corner: (i32, i32),
    value: i64,
    public: bool,
) -> SiteId {
    let rect = Rect::sized(corner.0, corner.1, 4, 4);
    let zone = world.world.layout().zone_at(Pos::new(corner.0, corner.1)).expect("in the world");
    let realm = world.zone(zone);
    let id = SiteId(realm.alloc_id());
    let mut site = Site::new(id, kind, format!("Test {kind}"), rect)
        .with_owner(owner)
        .with_attr("value", value);
    if public {
        site = site.with_tag("public");
    }
    realm.sites_mut().insert(site);
    id
}

impl Town {
    /// Makes the town with seed 0.
    pub fn new() -> Town {
        Town::with_seed(0)
    }

    /// Makes the town with a seed.
    pub fn with_seed(seed: u64) -> Town {
        let mut world = TestWorld::with_seed(&[&ECONOMY], seed);
        let at = build(&mut world);
        Town { world, at }
    }
}

/// Adds the sites of the town to a new world. The ids are the same in
/// every new world.
pub fn build(w: &mut TestWorld<'static>) -> Sites {
    Sites {
        farm: add_site(w, "farm", (2, 2), FARM_VALUE, false),
        mine: add_site(w, "mine", (8, 2), 600_000, false),
        forest: add_site(w, "forest", (14, 2), 250_000, false),
        workshop: add_site(w, "workshop", (20, 2), WORKSHOP_VALUE, false),
        factory: add_site(w, "factory", (26, 2), 900_000, false),
        restaurant: add_site(w, "restaurant", (32, 2), 500_000, false),
        shop: add_site(w, "shop", (38, 2), SHOP_VALUE, false),
        home: add_site(w, "home", (44, 2), HOME_VALUE, false),
        apartment: add_site(w, "apartment", (50, 2), APARTMENT_VALUE, false),
        city_hall: add_site(w, "city_hall", (2, 10), 1_000_000, true),
        depot: add_site(w, "depot", (8, 10), 1_000_000, true),
        office: add_site(w, "office", (14, 10), 400_000, false),
        park: add_site(w, "park", (20, 10), 100_000, true),
        far_home: add_site(w, "home", (66, 2), HOME_VALUE, false),
        far_shop: add_site(w, "shop", (72, 2), SHOP_VALUE, false),
        far_hall: add_site(w, "city_hall", (78, 2), 1_000_000, true),
    }
}

/// Returns a copy of a site.
pub fn site(world: &TestWorld<'_>, id: SiteId) -> Site {
    for zone in world.world.layout().zones() {
        if let Some(site) = world.zone(zone.id).sites().get(id) {
            return site.clone();
        }
    }
    panic!("no zone has site {id}");
}

/// Returns the zone of a site.
pub fn zone_of_site(world: &TestWorld<'_>, id: SiteId) -> ZoneId {
    let pos = site(world, id).rect.centre();
    world.world.layout().zone_at(pos).expect("the site is in the world")
}

/// Puts a resident in the middle of a site.
pub fn go(world: &mut TestWorld<'_>, r: ResidentId, to: SiteId) {
    let pos = site(world, to).rect.centre();
    world.set_pos(r, pos);
}

/// Spawns a resident with cash at a site.
pub fn resident_at(world: &mut TestWorld<'_>, name: &str, cash: i64, at: SiteId) -> ResidentId {
    let r = world.spawn(name);
    world.give_money(r, Money(cash));
    go(world, r, at);
    r
}

/// Returns true when the resident is idle with an empty queue.
pub fn is_idle(world: &TestWorld<'_>, r: ResidentId) -> bool {
    world.with_resident(r, |realm, idx| realm.activity(idx).is_none() && realm.queue_len(idx) == 0)
}

/// Steps the world until the resident is idle. Returns the public events
/// of those frames.
pub fn settle(world: &mut TestWorld<'_>, r: ResidentId) -> Vec<Event> {
    let mut events = Vec::new();
    for _ in 0..2_000 {
        if is_idle(world, r) {
            return events;
        }
        events.extend(world.step(1).into_iter().flat_map(|frame| frame.events));
    }
    panic!("resident {r} is still busy");
}

/// Sends an action that must be accepted, waits for its end and returns the
/// report with the public events of the frames in between.
pub fn run(
    world: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> (ActionReport, Vec<Event>) {
    let ack = world.act(r, verb, args);
    if let ActionStatus::Rejected { error } = &ack.status {
        panic!("`{verb}` was rejected: {error}");
    }
    let mut events = settle(world, r);
    // The answer of a global verb comes one frame after the request.
    let mut report = find_report(world, r, ack.action_id);
    for _ in 0..3 {
        if report.is_some() {
            break;
        }
        events.extend(world.step(1).into_iter().flat_map(|frame| frame.events));
        report = find_report(world, r, ack.action_id);
    }
    (report.unwrap_or_else(|| panic!("`{verb}` wrote no report")), events)
}

fn find_report(world: &TestWorld<'_>, r: ResidentId, action: u64) -> Option<ActionReport> {
    world.reports(r).into_iter().find(|report| report.action_id == action)
}

/// Runs an action that must succeed and returns its result.
pub fn ok(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    let (report, _) = run(world, r, verb, args);
    assert!(report.ok, "`{verb}` failed: {:?}", report.error);
    report.result
}

/// Runs an action that must succeed and returns its result with the public
/// events of the frames in between.
pub fn ok_with_events(
    world: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> (Value, Vec<Event>) {
    let (report, events) = run(world, r, verb, args);
    assert!(report.ok, "`{verb}` failed: {:?}", report.error);
    (report.result, events)
}

/// Runs an action that starts and then must fail. Returns the error of the
/// report.
pub fn fails(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ActionError {
    let (report, _) = run(world, r, verb, args);
    assert!(!report.ok, "`{verb}` succeeded: {}", report.result);
    report.error.expect("a failed report has an error")
}

/// Sends an action that must be refused at once. Returns the error code.
pub fn refused(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ErrorCode {
    world.act_err(r, verb, args).code
}

/// Asks a board of the global realm with a free verb and returns the
/// answer. The answer comes as a report, one frame after the request.
pub fn ask_board(world: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    let busy_with = world.activity(r);
    let ack = world.act(r, verb, args);
    assert_eq!(ack.status, ActionStatus::Pending, "a board is in the global realm");
    assert_eq!(world.activity(r), busy_with, "a free verb does not occupy the resident");
    world.step(2);
    let report = find_report(world, r, ack.action_id).expect("the answer of the board");
    assert!(report.ok, "{:?}", report.error);
    report.result
}

/// Returns the data of the events of one kind.
pub fn of_kind(events: &[Event], kind: &str) -> Vec<Value> {
    events.iter().filter(|e| e.kind == kind).map(|e| e.data.clone()).collect()
}

/// Returns the public events of some frames.
pub fn events_of(frames: Vec<FrameOutput>) -> Vec<Event> {
    frames.into_iter().flat_map(|frame| frame.events).collect()
}

/// Makes a resident the owner of a site of the city by a real purchase.
/// The resident gets the price first and ends at the site.
pub fn buy_site(world: &mut TestWorld<'_>, r: ResidentId, id: SiteId) {
    let price = site(world, id).attr("value");
    world.give_money(r, Money(price));
    go(world, r, id);
    ok(world, r, "buy_property", json!({}));
    assert_eq!(site(world, id).owner, Some(Holder::Resident(r)));
}

/// Buys a business site and opens it. The resident gets the price and the
/// permit fee first.
pub fn open_site(world: &mut TestWorld<'_>, r: ResidentId, id: SiteId) {
    buy_site(world, r, id);
    world.give_money(r, Money(world.param("economy.permit_fee")));
    ok(world, r, "open_business", json!({}));
    assert!(site(world, id).has_tag("open"));
}

fn realms<'a>(world: &'a TestWorld<'_>) -> Vec<&'a unsup_kernel::Realm> {
    let mut list = vec![world.global()];
    list.extend(world.world.layout().zones().iter().map(|zone| world.zone(zone.id)));
    list
}

/// Every faucet and sink that moved something, summed over all realms.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flows {
    /// Money that each faucet created, by faucet name.
    pub minted: BTreeMap<String, i64>,
    /// Money that each sink destroyed, by sink name.
    pub burned: BTreeMap<String, i64>,
    /// Goods that each faucet created, by `faucet:item`.
    pub produced: BTreeMap<String, i64>,
    /// Goods that each sink destroyed, by `sink:item`.
    pub consumed: BTreeMap<String, i64>,
}

/// Audits the whole world and returns what every faucet and sink did.
pub fn flows(world: &TestWorld<'_>) -> Flows {
    let totals = world.assert_conserved();
    let bp = world.world.blueprint();
    let mut out = Flows::default();
    for realm in realms(world) {
        let ledger = realm.ledger();
        for (id, name) in bp.faucets() {
            let money = ledger.minted_by(id).0;
            if money != 0 {
                *out.minted.entry(name.to_string()).or_default() += money;
            }
            for (item, def) in bp.items() {
                let qty = ledger.produced_by(id, item);
                if qty != 0 {
                    *out.produced.entry(format!("{name}:{}", def.name)).or_default() += qty;
                }
            }
        }
        for (id, name) in bp.sinks() {
            let money = ledger.burned_by(id).0;
            if money != 0 {
                *out.burned.entry(name.to_string()).or_default() += money;
            }
            for (item, def) in bp.items() {
                let qty = ledger.consumed_by(id, item);
                if qty != 0 {
                    *out.consumed.entry(format!("{name}:{}", def.name)).or_default() += qty;
                }
            }
        }
    }
    // The named flows explain every cent that exists.
    assert_eq!(out.minted.values().sum::<i64>(), totals.money.minted);
    assert_eq!(out.burned.values().sum::<i64>(), totals.money.burned);
    assert_eq!(
        totals.money.held + totals.money.in_flight,
        totals.money.minted - totals.money.burned
    );
    out
}

/// The faucet through which the tests give money and goods to residents.
const GRANT: &str = "kernel.grant";

impl Flows {
    /// Returns what the faucets and the sinks of the module did: the flows
    /// without the money and the goods that the test gave to residents.
    pub fn of_module(mut self) -> Flows {
        self.minted.retain(|name, _| name != GRANT);
        self.produced.retain(|name, _| !name.starts_with("kernel.grant:"));
        self
    }
}

/// Audits the whole world and asserts that the scenario was made of
/// transfers only: no faucet and no sink of the module moved anything.
pub fn assert_only_transfers(world: &TestWorld<'_>) {
    assert_eq!(flows(world).of_module(), Flows::default());
}

/// Builds the flows of the module that a scenario must have, from pairs:
/// money created and destroyed by faucet and sink name, goods created and
/// destroyed by `faucet:item` and `sink:item`.
pub fn module_flows<const A: usize, const B: usize, const C: usize, const D: usize>(
    minted: [(&str, i64); A],
    burned: [(&str, i64); B],
    produced: [(&str, i64); C],
    consumed: [(&str, i64); D],
) -> Flows {
    Flows {
        minted: map(minted),
        burned: map(burned),
        produced: map(produced),
        consumed: map(consumed),
    }
}

/// Builds a map of flows from pairs.
pub fn map<const N: usize>(pairs: [(&str, i64); N]) -> BTreeMap<String, i64> {
    pairs.into_iter().map(|(name, amount)| (name.to_string(), amount)).collect()
}

/// Returns the money of a holder in one realm.
pub fn money_in(world: &TestWorld<'_>, realm: RealmId, holder: Holder) -> i64 {
    world.world.realm(realm).expect("the realm exists").ledger().money(holder).0
}

/// Returns the till of a site.
pub fn till(world: &TestWorld<'_>, id: SiteId) -> i64 {
    world.money(Holder::Site(id)).0
}

/// Returns the cash of a resident in cents.
pub fn cash(world: &TestWorld<'_>, r: ResidentId) -> i64 {
    world.cash(r).0
}

/// Returns the money of the treasury of the first city.
pub fn treasury(world: &TestWorld<'_>) -> i64 {
    world.money(city()).0
}

/// Returns the money of the treasury of a city.
pub fn treasury_of(world: &TestWorld<'_>, city: CityId) -> i64 {
    world.money(Holder::Org(OrgId::city(city))).0
}

/// Returns one section of the brief observation of a resident.
pub fn section(world: &TestWorld<'_>, r: ResidentId, name: &str) -> Option<Value> {
    world.observe(r).sections.get(name).cloned()
}

/// Steps the world into the next game day, and three frames more so that
/// the daily work and its messages are done. Returns the public events.
pub fn next_day(world: &mut TestWorld<'_>) -> Vec<Event> {
    let mut events = Vec::new();
    loop {
        events.extend(events_of(world.step(1)));
        if world.world.frame().is_multiple_of(u64::from(DAY)) {
            break;
        }
    }
    events.extend(events_of(world.step(3)));
    events
}

/// Emits an event of another module in the global realm, as that module
/// would.
pub fn emit_global(world: &mut TestWorld<'_>, kind: &'static str, data: Value) {
    world.global().emit(unsup_kernel::EventMeta::public(kind, 50), &data);
    world.world.settle();
}

/// Returns the drama scores of the events of one kind.
pub fn drama_of(events: &[Event], kind: &str) -> Vec<u8> {
    events.iter().filter(|e| e.kind == kind).map(|e| e.drama).collect()
}
