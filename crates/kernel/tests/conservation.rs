//! Money and goods are conserved: nothing is created or lost outside
//! faucets and sinks, in every frame.

mod common;

use common::{BANK, world};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    Dest, Holder, ItemId, LedgerError, Money, Pos, RealmId, Rng, UNCLAIMED, ZoneId,
};

/// Audits the world and checks the conservation equation for money and for
/// every item. Returns (money created minus destroyed, money in flight).
fn check(w: &TestWorld<'_>) -> (i64, i64) {
    let totals = w.assert_conserved();
    let m = totals.money;
    assert_eq!(m.held + m.in_flight + m.external, m.minted - m.burned, "money: {m:?}");
    assert_eq!(m.external, 0, "the test world hosts every realm");
    for (item, t) in &totals.items {
        assert_eq!(
            t.held + t.in_flight + t.external,
            t.produced - t.consumed,
            "item {item}: {t:?}"
        );
    }
    (m.minted - m.burned, m.in_flight)
}

#[test]
fn totals_hold_in_every_frame_of_a_busy_world() {
    let mut w = world();
    let mut rng = Rng::seeded(11);
    let mut residents = Vec::new();
    for i in 0..24 {
        let pos = Pos::new(rng.range(40, 90) as i32, rng.range(40, 90) as i32);
        let id = w.spawn_at(&format!("r{i}"), pos);
        w.give_item(id, "bread", 3);
        w.give_item(id, "coal", i64::from(i % 3));
        residents.push(id);
    }
    let (start, _) = check(&w);
    assert_eq!(start, 24 * 1_000, "every resident starts with 10.00 from the faucet `demo.start`");

    let mut seen_in_flight = false;
    let mut alive = residents.clone();
    for frame in 0..300u32 {
        for _ in 0..6 {
            let who = alive[rng.below(alive.len() as u64) as usize];
            let other = alive[rng.below(alive.len() as u64) as usize];
            match rng.below(9) {
                0 => {
                    // A local transfer or a wire between zones. It may fail.
                    let amount = rng.range(1, 700);
                    w.act(who, "pay", json!({ "to": other, "amount": amount }));
                }
                1 => {
                    w.act(who, "work", json!({ "ticks": rng.range(1, 6) }));
                }
                2 => {
                    w.act(who, "step", json!({ "dx": rng.range(-1, 2), "dy": rng.range(-1, 2) }));
                }
                3 => {
                    w.act(who, "pay_tax", json!({ "amount": 10 }));
                }
                4 => {
                    w.act(who, "eat", Value::Null);
                }
                5 => {
                    w.act(who, "deposit", json!({ "amount": rng.range(1, 300) }));
                }
                6 => {
                    w.act(who, "withdraw", json!({ "amount": rng.range(1, 200) }));
                }
                7 => {
                    let zone = rng.below(4) as u32;
                    let (x, y) = (zone % 2 * 64 + 10, zone / 2 * 64 + 10);
                    w.act(who, "jump", json!({ "zone": zone, "x": x, "y": y }));
                }
                _ => {
                    if alive.len() > 12 && frame % 40 == 7 {
                        let heir =
                            if rng.chance(1, 2) { json!({ "heir": other }) } else { Value::Null };
                        if w.act(who, "leave", heir).status.is_accepted() {
                            alive.retain(|&r| r != who);
                        }
                    }
                }
            }
        }
        // Between frames, with wires and messages on the way.
        let (_, in_flight) = check(&w);
        seen_in_flight |= in_flight > 0;
        w.step(1);
        check(&w);
    }
    assert!(seen_in_flight, "the scenario must have value in transit, or it proves little");
    assert!(alive.len() < residents.len(), "some residents left the world");

    // The totals of each faucet and sink explain the whole change. The
    // number of finished `work` actions comes from another path: the event
    // handler of each zone counted them.
    let bp = w.world.blueprint();
    let wages = bp.faucet_id("demo.wages").expect("faucet");
    let start_faucet = bp.faucet_id("demo.start").expect("faucet");
    let tax = bp.sink_id("demo.tax").expect("sink");
    let eaten = bp.sink_id("demo.eaten").expect("sink");
    let bread = bp.item_id("bread").expect("item");
    let coal = bp.item_id("coal").expect("item");
    let (mut minted_wages, mut minted_start, mut burned_tax, mut eaten_bread, mut worked) =
        (0, 0, 0, 0, 0);
    for zone in 0..4 {
        let realm = w.zone(ZoneId(zone));
        let ledger = realm.ledger();
        minted_wages += ledger.minted_by(wages).cents();
        minted_start += ledger.minted_by(start_faucet).cents();
        burned_tax += ledger.burned_by(tax).cents();
        eaten_bread += ledger.consumed_by(eaten, bread);
        worked += i64::from(realm.res::<common::ZoneLog>().worked);
    }
    assert!(worked > 20, "{worked}");
    assert_eq!(minted_wages, worked * 500);
    assert_eq!(minted_start, 24 * 1_000);
    assert!(burned_tax > 0 && burned_tax % 10 == 0, "{burned_tax}");
    assert!(eaten_bread > 0, "{eaten_bread}");
    let totals = w.assert_conserved();
    assert_eq!(totals.money.minted, minted_wages + minted_start);
    assert_eq!(totals.money.burned, burned_tax);
    assert_eq!(totals.money.held + totals.money.in_flight, 24 * 1_000 + minted_wages - burned_tax);
    assert_eq!(totals.items[&bread].produced, 24 * 3);
    assert_eq!(totals.items[&bread].consumed, eaten_bread);
    assert_eq!(totals.items[&bread].held + totals.items[&bread].in_flight, 24 * 3 - eaten_bread);
    assert_eq!(totals.items[&coal].held + totals.items[&coal].in_flight, 24, "nobody burns coal");
    // The holdings of residents that left without an heir are unclaimed in
    // the global realm, not lost.
    let unclaimed = w.global().ledger().money(UNCLAIMED).cents();
    let banked = w.global().ledger().money(BANK).cents();
    let in_zones: i64 = (0..4).map(|z| w.zone(ZoneId(z)).ledger().totals().money.held).sum();
    assert_eq!(
        unclaimed + banked + in_zones + totals.money.in_flight,
        24 * 1_000 + minted_wages - burned_tax
    );
}

#[test]
fn a_failed_transfer_changes_nothing() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 100));
    w.give_item(ada, "bread", 1);
    let before = w.assert_conserved();

    // Not enough money, locally and by wire.
    let error = w.act_err(ada, "pay", json!({ "to": bob, "amount": 1_001 }));
    assert_eq!(error.code, unsup_kernel::ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_001, "available": 1_000 }));
    assert_eq!(
        w.act_err(ada, "pay", json!({ "to": cyd, "amount": 1_001 })).code,
        unsup_kernel::ErrorCode::Insufficient
    );
    assert_eq!(
        w.act_err(ada, "pay", json!({ "to": bob, "amount": -5 })).code,
        unsup_kernel::ErrorCode::InvalidArgs
    );
    assert_eq!(
        w.act_err(ada, "deposit", json!({ "amount": 5_000 })).code,
        unsup_kernel::ErrorCode::Insufficient
    );
    assert_eq!(
        w.act_err(ada, "pay_tax", json!({ "amount": 5_000 })).code,
        unsup_kernel::ErrorCode::Insufficient
    );
    assert_eq!(w.act_err(bob, "eat", Value::Null).code, unsup_kernel::ErrorCode::Insufficient);
    w.step(2);

    assert_eq!((w.cash(ada), w.cash(bob), w.cash(cyd)), (Money(1_000), Money(1_000), Money(1_000)));
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 1);
    assert!(w.notices(bob).is_empty(), "a failed payment tells nobody");
    assert_eq!(w.assert_conserved(), before);

    // The ledger and the wire API refuse the same things directly.
    let zone = w.zone(ZoneId(0));
    let a = Holder::Resident(ada);
    let b = Holder::Resident(bob);
    assert_eq!(
        zone.ledger_mut().transfer_money(a, b, Money(1_001), "gift"),
        Err(LedgerError::Insufficient { holder: a, needed: 1_001, available: 1_000 })
    );
    assert_eq!(
        zone.wire_money(a, Dest::AllZones, b, Money(10), "gift"),
        Err(LedgerError::BadDestination)
    );
    assert_eq!(
        zone.wire_money(a, Dest::Zone(ZoneId(40)), b, Money(10), "gift"),
        Err(LedgerError::BadDestination)
    );
    assert!(zone.wire_item(a, Dest::Global, BANK, ItemId(0), 9, "ship").is_err());
    assert_eq!(
        zone.wire_money(a, Dest::Global, BANK, Money(-1), "gift"),
        Err(LedgerError::InvalidAmount)
    );
    assert_eq!(w.assert_conserved(), before);
}

#[test]
fn wires_between_zones_and_the_global_realm_arrive_whole() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 100));
    w.give_item(ada, "coal", 5);

    // Zone to global: the money leaves at once and arrives in the frame.
    w.act_ok(ada, "deposit", json!({ "amount": 400 }));
    assert_eq!(w.cash(ada).cents(), 600);
    assert_eq!(w.money(BANK).cents(), 0);
    assert_eq!(check(&w), (2_000, 400));
    w.step(1);
    assert_eq!(w.global().ledger().money(BANK), Money(400));
    assert_eq!(check(&w), (2_000, 0));

    // Global to a resident: asked in frame 2, in the hand in frame 3.
    w.act_ok(ada, "withdraw", json!({ "amount": 150 }));
    w.step(1);
    assert_eq!(w.global().ledger().money(BANK), Money(250));
    assert_eq!(w.cash(ada).cents(), 600);
    assert_eq!(check(&w), (2_000, 150));
    w.step(1);
    assert_eq!(w.cash(ada).cents(), 750);
    // A withdrawal that the bank cannot pay fails in the global realm.
    w.act_ok(ada, "withdraw", json!({ "amount": 9_999 }));
    w.step(2);
    let report = w.last_report(ada).expect("a report");
    assert_eq!(report.error.map(|e| e.code), Some(unsup_kernel::ErrorCode::Insufficient));
    assert_eq!(w.global().ledger().money(BANK), Money(250));

    // Zone to zone, money and goods.
    w.act_ok(ada, "pay", json!({ "to": cyd, "amount": 50 }));
    let coal = w.world.blueprint().item_id("coal").expect("item");
    let zone = w.zone(ZoneId(0));
    zone.wire_item(
        Holder::Resident(ada),
        Dest::Resident(cyd),
        Holder::Resident(cyd),
        coal,
        2,
        "ship",
    )
    .expect("enough coal");
    w.world.settle();
    let totals = w.assert_conserved();
    assert_eq!((totals.money.in_flight, totals.items[&coal].in_flight), (50, 2));
    w.step(1);
    assert_eq!(w.cash(cyd).cents(), 1_050);
    assert_eq!(w.qty(Holder::Resident(cyd), "coal"), 2);
    assert_eq!(w.qty(Holder::Resident(ada), "coal"), 3);
    assert_eq!(check(&w), (2_000, 0));

    // A wire to a resident that no longer exists goes to the unclaimed
    // account. Nothing is lost.
    w.act_ok(ada, "pay", json!({ "to": cyd, "amount": 70 }));
    w.despawn(cyd);
    w.step(2);
    assert_eq!(w.global().ledger().money(UNCLAIMED), Money(1_050 + 70));
    assert_eq!(w.global().ledger().qty(UNCLAIMED, coal), 2);
    assert_eq!(check(&w), (2_000, 0));
    // The flows are recorded under their reason.
    let ledger = w.zone(ZoneId(0)).ledger();
    let flows: Vec<_> = ledger
        .money_flows()
        .map(|(reason, flow)| (reason.to_string(), flow.count, flow.amount))
        .collect();
    assert_eq!(flows, [("deposit".to_string(), 1, 400), ("gift".to_string(), 2, 120)]);
}

#[test]
fn audit_fails_when_a_balance_is_corrupted() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let cyd = w.spawn_at("Cyd", Pos::new(100, 100));
    w.give_item(ada, "bread", 2);
    w.step(3);
    assert!(w.world.audit().is_ok());

    // One cent too many in zone 3.
    w.corrupt_money(RealmId::Zone(ZoneId(3)), Holder::Resident(cyd), 1);
    let error = w.world.audit().expect_err("the audit must see the extra cent");
    assert_eq!(
        (error.realm, error.item, error.held, error.expected),
        (Some(RealmId::Zone(ZoneId(3))), None, 1_001, 1_000)
    );
    w.corrupt_money(RealmId::Zone(ZoneId(3)), Holder::Resident(cyd), -1);
    assert!(w.world.audit().is_ok());

    // One cent missing in the global realm.
    w.act_ok(ada, "deposit", json!({ "amount": 10 }));
    w.step(1);
    w.corrupt_money(RealmId::Global, BANK, -1);
    let error = w.world.audit().expect_err("the audit must see the missing cent");
    assert_eq!((error.realm, error.held, error.expected), (Some(RealmId::Global), 9, 10));
    w.corrupt_money(RealmId::Global, BANK, 1);

    // A loaf of bread from nowhere.
    w.corrupt_item(RealmId::Zone(ZoneId(0)), Holder::Resident(ada), "bread", 1);
    let error = w.world.audit().expect_err("the audit must see the extra bread");
    let bread = w.world.blueprint().item_id("bread").expect("item");
    assert_eq!(
        (error.realm, error.item, error.held, error.expected),
        (Some(RealmId::Zone(ZoneId(0))), Some(bread), 3, 2)
    );
    w.corrupt_item(RealmId::Zone(ZoneId(0)), Holder::Resident(ada), "bread", -1);
    assert!(w.world.audit().is_ok());
}

#[test]
#[should_panic(expected = "ledger audit failed in zone 0")]
fn a_realm_audits_itself_every_tick() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.corrupt_money(RealmId::Zone(ZoneId(0)), Holder::Resident(ada), 5);
    w.step(1);
}
