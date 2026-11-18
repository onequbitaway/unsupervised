//! The public statistics: exact values for a small known world.

mod common;

use common::{
    AT_EXCHANGE, SUBURB, account_holder, conserved, events, next_day_frames, ok, trader, world,
};
use serde_json::{Value, json};
use unsup_finance::stats::{Stats, gini_bp};
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{GRANT_FAUCET, Holder, Input, Money, OrgId, Output, ResidentId, SiteId, ZoneId};

/// Sells `qty` units of an item from `seller` to `buyer` at the exchange.
fn trade(
    w: &mut TestWorld<'_>,
    seller: ResidentId,
    buyer: ResidentId,
    item: &str,
    qty: i64,
    price: i64,
) {
    w.give_item(seller, item, qty);
    for (who, side) in [(seller, "sell"), (buyer, "buy")] {
        ok(
            w,
            who,
            "order_place",
            json!({ "item": item, "side": side, "qty": qty, "price": price }),
        );
    }
}

/// Returns the data of the one statistics event in a list of frames.
fn published(frames: &[FrameOutput]) -> Value {
    let found = events(frames, "finance.stats_published");
    assert_eq!(found.len(), 1, "one publication per day");
    assert!(found[0].public);
    found[0].data.clone()
}

#[test]
fn gini_of_known_lists() {
    assert_eq!(gini_bp(&mut [500, 500, 500, 500]), 0);
    // One owner among n has (n - 1) / n.
    assert_eq!(gini_bp(&mut [0, 0, 0, 1_000]), 7_500);
    assert_eq!(gini_bp(&mut [1_000, 0]), 5_000);
    // Sum 400, ranks times values 0 + 200 + 900: (2,200 - 1,600) / 1,200.
    assert_eq!(gini_bp(&mut [300, 0, 100]), 5_000);
    // Sum 10, ranks times values 30: (60 - 50) / 40.
    assert_eq!(gini_bp(&mut [4, 1, 3, 2]), 2_500);
    assert_eq!(gini_bp(&mut []), 0);
    assert_eq!(gini_bp(&mut [0, 0]), 0);
    // Large values do not overflow.
    assert_eq!(gini_bp(&mut [i64::MAX / 4, i64::MAX / 4]), 0);
}

#[test]
fn the_statistics_of_a_small_world_are_exact() {
    let mut w = world();
    w.set_param("finance.deposit_spread_bp", 10_000);
    w.set_param("finance.loan_base_limit", 20_000);
    // Ann has 30,000 in the bank. Bob has 10,000 in cash. Cid has goods.
    let ann = account_holder(&mut w, "Ann", 30_000);
    let bob = trader(&mut w, "Bob", 10_000);
    let cid = trader(&mut w, "Cid", 0);

    // Day 1: 10 wheat at 100 and 2 ore at 500. The fee is 5 each time.
    trade(&mut w, cid, bob, "wheat", 10, 100);
    trade(&mut w, cid, bob, "ore", 2, 500);
    ok(&mut w, bob, "market_claim", Value::Null);
    ok(&mut w, cid, "market_claim", Value::Null);
    assert_eq!((w.cash(bob), w.cash(cid)), (Money(8_000), Money(1_990)));
    assert!(w.global().res::<Stats>().latest().is_none());

    let day1 = published(&next_day_frames(&mut w));
    assert_eq!(
        day1,
        json!({
            "day": 1,
            // Everything came from `kernel.grant`: 30,100 and 10,000.
            "money_supply": 40_100,
            "money": {
                "cash": 9_990,
                "deposits": 30_000,
                // The bank has the account fee of 100, the city the fees.
                "treasuries": 110,
                "tills": 0,
                "escrow": 0,
                "other": 0,
                "in_transit": 0,
            },
            "price_index_bp": 10_000,
            "prices": {
                "ore": { "last": 500, "reference": 500, "basket": false },
                "wheat": { "last": 100, "reference": 100, "basket": true },
            },
            // Wealth 1,990, 8,000 and 30,000: sum 39,990, ranks times
            // values 107,990, (215,980 - 159,960) / 119,970 = 0.4669.
            "gini_bp": 4_669,
            "residents": 3,
            // A faucet or a sink that was never used is absent.
            "faucets": { "kernel.grant": { "total": 40_100, "day": 40_100 } },
            "sinks": {},
            // Cid got 10 wheat and 2 ore to sell.
            "goods_faucets": { "kernel.grant": { "total": 12, "day": 12 } },
            "goods_sinks": {},
            "loans": { "count": 0, "outstanding": 0, "written_off": 0 },
            "base_rate_bp": 500,
        })
    );
    assert_eq!(conserved(&w), 40_100);

    // Day 2. Wheat: 5 at 120 at the exchange and 5 at 150 in a shop, mean
    // 135. Fish is new at 40. Ore at 900 is not food. Ann borrows 10,000.
    // Cid leaves what it earned in the escrows.
    trade(&mut w, cid, bob, "wheat", 5, 120);
    ok(&mut w, cid, "test_shop_sale", json!({ "item": "wheat", "qty": 5, "unit_price": 150 }));
    trade(&mut w, cid, bob, "fish", 4, 40);
    trade(&mut w, cid, bob, "ore", 1, 900);
    ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    // Bob eats 3 of the wheat and 1 of the ore that he collected on day 1.
    ok(&mut w, bob, "test_eat", json!({ "item": "wheat", "qty": 3 }));
    ok(&mut w, bob, "test_eat", json!({ "item": "ore", "qty": 1 }));
    // The global realm has a ledger too: something there makes 7 fish.
    let grant = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    let fish = w.world.blueprint().item_id("fish").expect("fish is an item");
    w.global().ledger_mut().produce(grant, Holder::Org(OrgId::NATION), fish, 7);
    let day2 = published(&next_day_frames(&mut w));
    assert_eq!(day2["day"], json!(2));
    assert_eq!(day2["money_supply"], json!(50_100));
    assert_eq!(
        day2["money"],
        json!({
            // Bob paid 600, 160 and 900.
            "cash": 6_340 + 1_990,
            "deposits": 40_000,
            // City fees: 10, and 3, 0 and 4 today.
            "treasuries": 100 + 17,
            "tills": 0,
            // What Cid earned today: 597, 160 and 896.
            "escrow": 1_653,
            "other": 0,
            "in_transit": 0,
        })
    );
    assert_eq!(
        day2["prices"],
        json!({
            "fish": { "last": 40, "reference": 40, "basket": true },
            "ore": { "last": 900, "reference": 500, "basket": false },
            "wheat": { "last": 135, "reference": 100, "basket": true },
        })
    );
    // The mean of 135 / 100 and 40 / 40. Ore is not in the basket.
    assert_eq!(day2["price_index_bp"], json!(11_750));
    // Wealth 1,990, 6,340 and 40,000: sum 48,330, ranks times values
    // 134,670, (269,340 - 193,320) / 144,990 = 0.5243.
    assert_eq!(day2["gini_bp"], json!(5_243));
    assert_eq!(
        day2["faucets"],
        json!({
            "bank.loan_issue": { "total": 10_000, "day": 10_000 },
            "kernel.grant": { "total": 40_100, "day": 0 },
        })
    );
    assert_eq!(day2["sinks"], json!({}));
    // Goods count in units, all items together: 5 wheat, 4 fish and 1 ore
    // for Cid in zone 0, and the 7 fish of the global realm.
    assert_eq!(day2["goods_faucets"], json!({ "kernel.grant": { "total": 29, "day": 17 } }));
    assert_eq!(day2["goods_sinks"], json!({ "test.eaten": { "total": 4, "day": 4 } }));
    assert_eq!(day2["loans"], json!({ "count": 1, "outstanding": 10_000, "written_off": 0 }));

    // Day 3: no sale, so the prices and the index stay. At midnight the
    // bank takes the first instalment: 2,500 of principal are destroyed.
    let day3 = published(&next_day_frames(&mut w));
    assert_eq!(day3["price_index_bp"], json!(11_750));
    assert_eq!(day3["prices"], day2["prices"]);
    assert_eq!(day3["money_supply"], json!(47_600));
    assert_eq!(day3["faucets"]["bank.loan_issue"], json!({ "total": 10_000, "day": 0 }));
    assert_eq!(day3["sinks"], json!({ "bank.loan_repay": { "total": 2_500, "day": 2_500 } }));
    assert_eq!(day3["goods_faucets"], json!({ "kernel.grant": { "total": 29, "day": 0 } }));
    assert_eq!(day3["goods_sinks"], json!({ "test.eaten": { "total": 4, "day": 0 } }));
    assert_eq!(day3["loans"], json!({ "count": 1, "outstanding": 7_500, "written_off": 0 }));
    // The interest of 28 moved from Ann to the bank: 800 basis points a
    // year on 10,000 for one of 28 days.
    assert_eq!(day3["money"]["deposits"], json!(40_000 - 2_528));
    assert_eq!(day3["money"]["treasuries"], json!(117 + 28));
    assert_eq!(day3["money"]["in_transit"], json!(0));
    assert_eq!(conserved(&w), 47_600);

    // The state keeps the last publication.
    let stats = w.global().res::<Stats>();
    let latest = stats.latest().expect("published");
    assert_eq!(
        (latest.day, latest.money_supply, latest.gini_bp),
        (3, 47_600, day3["gini_bp"].as_i64().unwrap())
    );
    assert_eq!(stats.price("wheat").map(|track| (track.reference, track.last)), Some((100, 135)));
}

#[test]
fn the_money_supply_counts_money_between_realms() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 5_000);
    // Step to the last frame of the day, then send a deposit and a
    // withdrawal on their way.
    let frame = w.world.frame();
    w.step(u32::try_from(1_439 - frame).expect("the first day"));
    w.give_money(ann, Money(700));
    w.act_ok(ann, "withdraw", json!({ "amount": 2_000 }));
    let frames = w.step(1);
    let stats = published(&frames);
    // The 2,000 left the account in this frame and reach Ann in the next.
    assert_eq!(stats["money_supply"], json!(5_800));
    assert_eq!(stats["money"]["deposits"], json!(3_000));
    assert_eq!(stats["money"]["cash"], json!(700));
    assert_eq!(stats["money"]["in_transit"], json!(2_000));
    assert_eq!(w.assert_conserved().money.in_flight, 2_000);
    w.step(1);
    assert_eq!(w.cash(ann), Money(2_700));
}

#[test]
fn every_zone_reports_at_the_day_boundary_whatever_the_period_is() {
    let mut w = world();
    // With this period the zones report at frame 1,000 and not again
    // before the end of the day.
    w.set_param("finance.stats_report_period", 1_000);
    let yan = w.spawn_at("Yan", AT_EXCHANGE);
    w.give_money(yan, Money(400));
    w.step(1_100);
    assert_eq!(
        w.global().res::<Stats>().zone_report(0).map(|report| (report.frame, report.cash)),
        Some((1_000, 400))
    );
    w.give_money(yan, Money(350));
    let frame = w.world.frame();
    let stats = published(&w.step(u32::try_from(1_440 - frame).expect("the first day")));
    // The publication reads the report of frame 1,440, not of frame 1,000.
    assert_eq!(stats["money"]["cash"], json!(750));
    assert_eq!(stats["money"]["in_transit"], json!(0));
    assert_eq!(stats["money_supply"], json!(750));
    assert_eq!(w.global().res::<Stats>().zone_report(0).map(|report| report.frame), Some(1_440));
    conserved(&w);
}

#[test]
fn a_zone_that_misses_the_day_boundary_is_read_from_its_last_report() {
    let mut w = world();
    // Yan stays in zone 0 with 750. Xia stays in zone 1 with 100. Zed is
    // in zone 1 with 500 when zone 1 reports at frame 1,380. Then Zed
    // walks to zone 0 and gets 250 more.
    let yan = w.spawn_at("Yan", AT_EXCHANGE);
    w.give_money(yan, Money(750));
    let xia = w.spawn_at("Xia", SUBURB);
    w.give_money(xia, Money(100));
    let zed = w.spawn_at("Zed", SUBURB);
    w.give_money(zed, Money(500));
    w.step(1_390);
    assert_eq!(
        w.global().res::<Stats>().zone_report(1).map(|report| (report.frame, report.cash)),
        Some((1_380, 600))
    );
    w.set_pos(zed, AT_EXCHANGE);
    w.give_money(zed, Money(250));
    w.step(49);
    // Zone 1 skips the frame of the day boundary (time dilation).
    let Output::Frame(frame) = w.world.apply(Input::Step { skip: vec![ZoneId(1)] }) else {
        panic!("a step gives a frame");
    };
    let stats = published(&[frame]);
    // The Gini coefficient counts the residents of the zones that reported
    // in this frame: Yan and Zed in zone 0, with 750 each. Xia is not
    // counted today, and Zed is counted once.
    assert_eq!(stats["residents"], json!(2));
    assert_eq!(stats["gini_bp"], json!(0));
    // The money supply is exact: it comes from the faucets and sinks.
    assert_eq!(stats["money_supply"], json!(1_600));
    // The parts are not: the 500 of Zed are in the old report of zone 1
    // and in the new report of zone 0. The error shows as negative money
    // in transit.
    assert_eq!(stats["money"]["cash"], json!(1_500 + 600));
    assert_eq!(stats["money"]["in_transit"], json!(-500));
    // The lists of residents are not kept.
    let kept = w.global().res::<Stats>();
    assert_eq!(kept.zone_report(0).map(|report| report.residents.len()), Some(0));
    drop(kept);

    // The next day every zone reports again, and everybody counts.
    let next = published(&next_day_frames(&mut w));
    assert_eq!(next["residents"], json!(3));
    assert_eq!(next["money"]["in_transit"], json!(0));
    // Wealth 100, 750 and 750: sum 1,600, ranks times values 3,850,
    // (7,700 - 6,400) / 4,800 = 0.2708.
    assert_eq!(next["gini_bp"], json!(2_708));
    conserved(&w);
}

#[test]
fn tills_and_unclaimed_money_have_their_own_parts() {
    let mut w = world();
    let yan = w.spawn_at("Yan", AT_EXCHANGE);
    w.give_money(yan, Money(400));
    // The till of a shop in zone 0 holds 300.
    let faucet = w.world.blueprint().faucet_id(GRANT_FAUCET).expect("the kernel faucet");
    w.zone(ZoneId(0)).ledger_mut().mint(faucet, Holder::Site(SiteId(77)), Money(300));
    // A resident that left the world without an heir leaves 250 in the
    // unclaimed account of the global realm.
    let gone = w.spawn_at("Gone", SUBURB);
    w.give_money(gone, Money(250));
    w.act_ok(gone, "test_die", json!({ "heir": null, "silent": true }));

    let stats = published(&next_day_frames(&mut w));
    assert_eq!(stats["money_supply"], json!(950));
    assert_eq!(
        stats["money"],
        json!({
            "cash": 400,
            // The unclaimed account is not a bank deposit.
            "deposits": 0,
            "treasuries": 0,
            "tills": 300,
            "escrow": 0,
            "other": 250,
            "in_transit": 0,
        })
    );
    // Only Yan is a resident now.
    assert_eq!((stats["residents"].clone(), stats["gini_bp"].clone()), (json!(1), json!(0)));
    conserved(&w);
}

#[test]
fn a_wire_that_leaves_a_zone_after_its_report_is_counted_twice_that_day() {
    let mut w = world();
    // Deposits earn nothing here.
    w.set_param("finance.deposit_spread_bp", 10_000);
    let ann = account_holder(&mut w, "Ann", 5_000);
    w.give_money(ann, Money(700));
    let frame = w.world.frame();
    w.step(u32::try_from(1_439 - frame).expect("the first day"));
    // A routine starts its action after the systems of the zone, so after
    // the report of the zone. The cash of this deposit is in the report,
    // and the global realm has it in the account in the same frame.
    let routines = json!({ "routines": [{
        "name": "save",
        "when": { "type": "always" },
        "then": { "verb": "deposit", "args": { "amount": 700 } },
    }] });
    w.act_ok(ann, "set_routines", routines);
    let stats = published(&w.step(1));
    assert_eq!(w.cash(ann), Money(0), "the routine sent the deposit in the frame of the midnight");
    // The money supply is exact: it comes from the faucets and the sinks.
    assert_eq!(stats["money_supply"], json!(5_800));
    // The parts are not: the 700 are counted as cash and as a deposit. The
    // error shows as negative money in transit.
    assert_eq!(stats["money"]["cash"], json!(700));
    assert_eq!(stats["money"]["deposits"], json!(5_700));
    assert_eq!(stats["money"]["in_transit"], json!(-700));
    assert_eq!(conserved(&w), 5_800);

    // The next publication is exact again.
    w.act_ok(ann, "set_routines", json!({ "routines": [] }));
    let next = published(&next_day_frames(&mut w));
    assert_eq!(next["money"]["cash"], json!(0));
    assert_eq!(next["money"]["deposits"], json!(5_700));
    assert_eq!(next["money"]["in_transit"], json!(0));
}
