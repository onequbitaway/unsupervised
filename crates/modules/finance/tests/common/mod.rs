//! A small town for the finance tests: one bank, one exchange and a few
//! test goods. The finance module defines no goods and no sites, so the
//! tests bring their own.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_finance::FinancePlugin;
use unsup_finance::bank::Bank;
use unsup_kernel::protocol::{ActionReport, ActionStatus};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    ActionError, Builder, CondDef, EventMeta, GRANT_FAUCET, Holder, Money, OrgId, Plugin, Pos,
    Rect, ResidentId, Scope, Site, SiteId, VerbSpec, ZoneId,
};

/// The goods of the test town.
pub const GOODS: &str = r#"
[[items]]
name = "wheat"
weight = 1
tags = ["food"]
summary = "A sack of wheat."

[[items]]
name = "fish"
weight = 1
tags = ["food"]
summary = "A fish."

[[items]]
name = "ore"
weight = 5
summary = "A lump of ore."

[[items]]
name = "powder"
weight = 1
tags = ["contraband"]
summary = "An illegal powder."
"#;

/// The bank of the town, in zone 0.
pub const BANK: Rect = Rect::new(10, 10, 14, 14);
/// The exchange of the town, in zone 0.
pub const EXCHANGE: Rect = Rect::new(20, 10, 24, 14);
/// A tile inside the bank.
pub const AT_BANK: Pos = Pos::new(11, 11);
/// A tile inside the exchange.
pub const AT_EXCHANGE: Pos = Pos::new(21, 11);
/// A tile of zone 0 that is in no site.
pub const STREET: Pos = Pos::new(40, 40);
/// A tile of zone 1, which has no bank and no exchange.
pub const SUBURB: Pos = Pos::new(100, 30);

/// The fee of `open_account` in the data file.
pub const OPEN_FEE: i64 = 100;

/// Arguments of the test verb `test_shop_sale`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ShopSaleArgs {
    /// Name of the item.
    pub item: String,
    /// Units sold.
    pub qty: i64,
    /// Price of one unit in cents.
    pub unit_price: i64,
}

/// Arguments of the test verb `test_eat`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct EatArgs {
    /// Name of the item.
    pub item: String,
    /// Units that the resident eats.
    pub qty: i64,
}

/// The goods sink of the test verb `test_eat`.
pub const EATEN: &str = "test.eaten";

/// Arguments of the test verb `test_die`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct DieArgs {
    /// The resident that inherits.
    pub heir: Option<ResidentId>,
    /// True: leave the world without the event `life.resident_died`.
    #[serde(default)]
    pub silent: bool,
}

/// The plugin that builds the test town.
pub struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        b.load_data(GOODS);
        // The condition of the life module, with its definition from the
        // game design.
        b.condition(CondDef::new("hospitalized").blocking().summary("In hospital until healed."));
        // Stands in for the economy module: reports a shop sale. It moves
        // nothing; the statistics only read the event.
        b.instant_verb::<ShopSaleArgs>(
            VerbSpec {
                verb: "test_shop_sale",
                module: "town",
                summary: "Report a shop sale.",
                free: true,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                realm.emit(
                    EventMeta::public("economy.goods_sold", 1).actor(ctx.resident),
                    &json!({
                        "seller": Holder::Resident(ctx.resident),
                        "buyer": ctx.resident,
                        "site": 0,
                        "city": 0,
                        "item": args.item,
                        "qty": args.qty,
                        "unit_price": args.unit_price,
                        "tax": 0,
                    }),
                );
                Ok(Value::Null)
            },
        );
        // Stands in for the life module: the resident eats goods that it
        // carries. They leave the world through a goods sink.
        let eaten = b.sink(EATEN);
        b.instant_verb::<EatArgs>(
            VerbSpec {
                verb: "test_eat",
                module: "town",
                summary: "Eat goods.",
                free: true,
                allowed_in: &[],
            },
            move |realm, ctx, args| {
                let Some(item) = realm.defs().item_id(&args.item) else {
                    return Err(ActionError::invalid_args("No item has this name."));
                };
                realm.ledger_mut().consume(
                    Holder::Resident(ctx.resident),
                    eaten,
                    item,
                    args.qty,
                )?;
                Ok(Value::Null)
            },
        );
        // Stands in for the life module: the resident dies. `heir` is the
        // resident that inherits; without one the event names no heir.
        // With `silent` the resident leaves without the event of the life
        // module, like a resident that an administrator removed.
        b.instant_verb::<DieArgs>(
            VerbSpec {
                verb: "test_die",
                module: "town",
                summary: "Die.",
                free: false,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                let heir = args.heir.map(Holder::Resident);
                if !args.silent {
                    realm.emit(
                        EventMeta::public("life.resident_died", 80).actor(ctx.resident),
                        &json!({
                            "resident": ctx.resident,
                            "cause": "test",
                            "age": 70,
                            "heir": heir,
                            "estate": 0,
                        }),
                    );
                }
                realm.despawn(ctx.idx, heir.unwrap_or(unsup_kernel::UNCLAIMED), "test");
                Ok(Value::Null)
            },
        );
        if b.scope() == Scope::Zone {
            b.on_worldgen(0, |realm, _gen| {
                if realm.zone_info().map(|zone| zone.id) != Some(ZoneId(0)) {
                    return;
                }
                let bank = SiteId(realm.alloc_id());
                realm.sites_mut().insert(Site::new(bank, "bank", "First Bank", BANK));
                let exchange = SiteId(realm.alloc_id());
                realm.sites_mut().insert(Site::new(exchange, "exchange", "Exchange", EXCHANGE));
            });
        }
    }
}

/// Makes the test town with the finance module.
pub fn world() -> TestWorld<'static> {
    TestWorld::new(&[&Town, &FinancePlugin])
}

/// The plugins of the test town, for `assert_deterministic`.
pub fn plugins() -> [&'static dyn Plugin; 2] {
    [&Town, &FinancePlugin]
}

/// Sends an action and returns how it ended: the result, or the error. A
/// request that the zone rejects ends at once. Any other request ends with
/// a report; the world steps until the report is there.
pub fn outcome(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    verb: &str,
    args: Value,
) -> Result<Value, ActionError> {
    let ack = w.act(r, verb, args);
    match ack.status {
        ActionStatus::Done { result } => return Ok(result),
        ActionStatus::Rejected { error } => return Err(error),
        _ => {}
    }
    for _ in 0..20 {
        w.step(1);
        let found: Option<ActionReport> =
            w.reports(r).into_iter().find(|report| report.action_id == ack.action_id);
        if let Some(report) = found {
            return match report.error {
                Some(error) => Err(error),
                None => Ok(report.result),
            };
        }
    }
    panic!("`{verb}` of resident {r} did not end in 20 frames");
}

/// Sends an action that the test expects to succeed. Returns its result.
pub fn ok(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> Value {
    match outcome(w, r, verb, args) {
        Ok(result) => result,
        Err(error) => panic!("`{verb}` of resident {r} failed: {error}"),
    }
}

/// Sends an action that the test expects to fail. Returns its error.
pub fn err(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) -> ActionError {
    match outcome(w, r, verb, args) {
        Ok(result) => panic!("`{verb}` of resident {r} succeeded: {result}"),
        Err(error) => error,
    }
}

/// Spawns a resident inside the bank with `cash` cents.
pub fn customer(w: &mut TestWorld<'_>, name: &str, cash: i64) -> ResidentId {
    let r = w.spawn_at(name, AT_BANK);
    w.give_money(r, Money(cash));
    r
}

/// Spawns a resident inside the bank, opens its account and deposits
/// `deposit` cents. The resident has no cash afterwards.
pub fn account_holder(w: &mut TestWorld<'_>, name: &str, deposit: i64) -> ResidentId {
    let r = customer(w, name, OPEN_FEE + deposit);
    ok(w, r, "open_account", Value::Null);
    if deposit > 0 {
        ok(w, r, "deposit", json!({ "amount": deposit }));
    }
    r
}

/// Spawns a resident inside the exchange with cash.
pub fn trader(w: &mut TestWorld<'_>, name: &str, cash: i64) -> ResidentId {
    let r = w.spawn_at(name, AT_EXCHANGE);
    w.give_money(r, Money(cash));
    r
}

/// Returns the balance of the bank account of a resident.
pub fn balance(w: &TestWorld<'_>, r: ResidentId) -> i64 {
    w.money(Bank::holder(r)).0
}

/// Returns the money of the bank treasury: fees and loan interest.
pub fn bank_treasury(w: &TestWorld<'_>) -> i64 {
    w.money(Holder::Org(OrgId::CENTRAL_BANK)).0
}

/// Gives a resident a role with permissions in an organization.
pub fn grant(w: &mut TestWorld<'_>, r: ResidentId, org: OrgId, perms: &[&str]) {
    w.with_resident(r, |realm, _| realm.grant(r, org, "officer", perms));
}

/// Mints money for a holder of the global realm through `kernel.grant`.
pub fn fund_global(w: &mut TestWorld<'_>, holder: Holder, cents: i64) {
    let faucet = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    w.global().ledger_mut().mint(faucet, holder, Money(cents));
}

/// Returns the money that a faucet created, summed over all realms.
pub fn minted(w: &TestWorld<'_>, faucet: &str) -> i64 {
    let id = w.world.blueprint().faucet_id(faucet).expect("the faucet exists");
    let zones = w.world.layout().zones().iter().map(|zone| w.zone(zone.id));
    std::iter::once(w.global()).chain(zones).map(|realm| realm.ledger().minted_by(id).0).sum()
}

/// Returns the money that a sink destroyed, summed over all realms.
pub fn burned(w: &TestWorld<'_>, sink: &str) -> i64 {
    let id = w.world.blueprint().sink_id(sink).expect("the sink exists");
    let zones = w.world.layout().zones().iter().map(|zone| w.zone(zone.id));
    std::iter::once(w.global()).chain(zones).map(|realm| realm.ledger().burned_by(id).0).sum()
}

/// Audits the world and checks the conservation equation for money and
/// for every item. Returns the money that exists: created minus destroyed.
pub fn conserved(w: &TestWorld<'_>) -> i64 {
    let totals = w.assert_conserved();
    let m = totals.money;
    assert_eq!(m.held + m.in_flight + m.external, m.minted - m.burned, "money: {m:?}");
    for (item, t) in &totals.items {
        assert_eq!(
            t.held + t.in_flight + t.external,
            t.produced - t.consumed,
            "item {item}: {t:?}"
        );
    }
    // The deposit total of the bank is the sum of the account balances.
    let bank = w.global().res::<Bank>();
    let sum: i64 = bank.accounts().map(|(r, _)| Bank::balance(w.global(), r)).sum();
    assert_eq!(bank.deposits(), sum, "the deposit total of the bank");
    m.minted - m.burned
}

/// Steps the world to the start of the next game day and one frame more,
/// so that the daily work of the bank and its notices are done.
pub fn next_day(w: &mut TestWorld<'_>) {
    let frame = w.world.frame();
    let target = (frame / 1_440 + 1) * 1_440 + 1;
    w.step(u32::try_from(target - frame).expect("less than a day"));
}

/// Returns the public events of one kind that a list of frames made.
pub fn events(
    frames: &[unsup_kernel::protocol::FrameOutput],
    kind: &str,
) -> Vec<unsup_kernel::Event> {
    frames
        .iter()
        .flat_map(|frame| frame.events.iter())
        .filter(|event| event.kind == kind)
        .cloned()
        .collect()
}

/// Steps the world to the start of the next game day and one frame more,
/// like [`next_day`], and returns the frames.
pub fn next_day_frames(w: &mut TestWorld<'_>) -> Vec<unsup_kernel::protocol::FrameOutput> {
    let frame = w.world.frame();
    let target = (frame / 1_440 + 1) * 1_440 + 1;
    w.step(u32::try_from(target - frame).expect("less than a day"))
}

/// Returns the newest notice of a kind in the inbox of a resident.
pub fn notice(w: &TestWorld<'_>, r: ResidentId, kind: &str) -> Option<Value> {
    w.notices(r).into_iter().rev().find(|notice| notice.kind == kind).map(|notice| notice.data)
}
