//! The exchange: orders, escrow, matching by price and time, the fee,
//! cancel and claim.

mod common;

use common::{AT_EXCHANGE, STREET, SUBURB, conserved, err, events, notice, ok, trader, world};
use serde_json::{Value, json};
use unsup_finance::exchange::{Exchange, Side};
use unsup_kernel::protocol::{AdminCommand, AdminReply, Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ErrorCode, Holder, Money, OrgId, ResidentId, ZoneId};

const CITY: Holder = Holder::Org(OrgId::city(CityId(0)));

/// Places an order and returns its id.
fn place(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    side: &str,
    item: &str,
    qty: i64,
    price: i64,
) -> u64 {
    let args = json!({ "item": item, "side": side, "qty": qty, "price": price });
    ok(w, r, "order_place", args)["order"].as_u64().expect("an order id")
}

fn place_err(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    side: &str,
    item: &str,
    qty: i64,
    price: i64,
) -> unsup_kernel::ActionError {
    err(w, r, "order_place", json!({ "item": item, "side": side, "qty": qty, "price": price }))
}

/// Spawns a trader at the exchange with goods and no cash.
fn seller(w: &mut TestWorld<'_>, name: &str, item: &str, qty: i64) -> ResidentId {
    let r = trader(w, name, 0);
    w.give_item(r, item, qty);
    r
}

#[test]
fn an_order_locks_escrow_and_a_crossing_order_trades() {
    let mut w = world();
    let sam = seller(&mut w, "Sam", "wheat", 10);
    let bea = trader(&mut w, "Bea", 5_000);
    let start = conserved(&w);

    // `order_place` takes one tick.
    let args = json!({ "item": "wheat", "side": "sell", "qty": 10, "price": 100 });
    let status = w.act_ok(sam, "order_place", args.clone());
    assert!(matches!(status, unsup_kernel::protocol::ActionStatus::Started { ticks: 1, .. }));
    assert_eq!(w.qty(Holder::Resident(sam), "wheat"), 10, "nothing is locked before the end");
    w.step(1);
    let report = w.last_report(sam).expect("the order reports");
    let sell = report.result["order"].as_u64().expect("an order id");
    assert_eq!(
        report.result,
        json!({ "order": sell, "item": "wheat", "side": "sell", "qty": 10, "price": 100 })
    );
    assert_eq!(w.qty(Holder::Resident(sam), "wheat"), 0);
    assert_eq!(w.qty(Holder::Escrow(sell), "wheat"), 10);

    // Bea bids 120 for 4. The order of Sam was first, so the price is 100.
    let args = json!({ "item": "wheat", "side": "buy", "qty": 4, "price": 120 });
    w.act_ok(bea, "order_place", args);
    assert_eq!(w.cash(bea), Money(5_000));
    let frames = w.step(1);
    let buy = w.last_report(bea).expect("the order reports").result["order"]
        .as_u64()
        .expect("an order id");
    let fills = events(&frames, "finance.order_filled");
    assert_eq!(fills.len(), 1);
    assert_eq!(
        fills[0].data,
        json!({ "city": 0, "item": "wheat", "qty": 4, "price": 100, "buyer": bea, "seller": sam })
    );
    assert!(fills[0].public);
    assert_eq!(fills[0].realm, unsup_kernel::RealmId::Zone(ZoneId(0)));

    // Bea locked 480 and paid 400: 80 and the wheat wait in the escrow.
    assert_eq!(w.cash(bea), Money(5_000 - 480));
    assert_eq!(w.money(Holder::Escrow(buy)), Money(80));
    assert_eq!(w.qty(Holder::Escrow(buy), "wheat"), 4);
    // Sam earned 400 less the fee of 50 basis points; the city has the fee.
    assert_eq!(w.money(Holder::Escrow(sell)), Money(398));
    assert_eq!(w.qty(Holder::Escrow(sell), "wheat"), 6);
    assert_eq!(w.money(CITY), Money(2));
    assert_eq!(conserved(&w), start);

    // Both traders hear about it.
    assert_eq!(
        notice(&w, bea, "finance.order_filled"),
        Some(json!({
            "order": buy, "item": "wheat", "side": "buy", "qty": 4, "value": 400, "unfilled": 0,
        }))
    );
    assert_eq!(
        notice(&w, sam, "finance.order_filled"),
        Some(json!({
            "order": sell, "item": "wheat", "side": "sell", "qty": 4, "value": 400, "unfilled": 6,
        }))
    );

    // The `market` section shows the orders and what waits.
    assert_eq!(
        w.observe(bea).sections["market"],
        json!({
            "orders": [{
                "order": buy, "item": "wheat", "side": "buy", "price": 120, "qty": 4,
                "filled": 4, "open": false,
            }],
            "claimable": { "cash": 80, "items": { "wheat": 4 } },
        })
    );
    assert_eq!(w.observe(sam).sections["market"]["claimable"], json!({ "cash": 398, "items": {} }));

    // `market_claim` hands it over. A filled order is then gone; an open
    // one stays.
    let result = ok(&mut w, bea, "market_claim", Value::Null);
    assert_eq!(
        result,
        json!({ "claimed": { "cash": 80, "items": { "wheat": 4 } }, "open_orders": 0 })
    );
    assert_eq!(w.cash(bea), Money(4_600));
    assert_eq!(w.qty(Holder::Resident(bea), "wheat"), 4);
    assert!(!w.observe(bea).sections.contains_key("market"));
    let result = ok(&mut w, sam, "market_claim", Value::Null);
    assert_eq!(result, json!({ "claimed": { "cash": 398, "items": {} }, "open_orders": 1 }));
    assert_eq!(w.cash(sam), Money(398));
    assert_eq!(w.qty(Holder::Escrow(sell), "wheat"), 6);
    assert_eq!(conserved(&w), start);
}

#[test]
fn matching_is_by_price_then_time() {
    let mut w = world();
    w.set_param("finance.exchange_fee_bp", 0);
    let s1 = seller(&mut w, "S1", "wheat", 5);
    let s2 = seller(&mut w, "S2", "wheat", 5);
    let s3 = seller(&mut w, "S3", "wheat", 5);
    let bea = trader(&mut w, "Bea", 10_000);
    let start = conserved(&w);
    let o1 = place(&mut w, s1, "sell", "wheat", 5, 100);
    let o2 = place(&mut w, s2, "sell", "wheat", 5, 90);
    let o3 = place(&mut w, s3, "sell", "wheat", 5, 100);

    // One buy order for 12 at up to 110 takes the cheapest ask first, then
    // the older of the two asks at 100, each at the price of the ask.
    w.act_ok(
        bea,
        "order_place",
        json!({ "item": "wheat", "side": "buy", "qty": 12, "price": 110 }),
    );
    let frames = w.step(1);
    let fills: Vec<(u64, i64, i64)> = events(&frames, "finance.order_filled")
        .iter()
        .map(|e| {
            (
                e.data["seller"].as_u64().expect("a seller"),
                e.data["qty"].as_i64().expect("a quantity"),
                e.data["price"].as_i64().expect("a price"),
            )
        })
        .collect();
    assert_eq!(fills, [(s2.0, 5, 90), (s1.0, 5, 100), (s3.0, 2, 100)]);
    {
        let exchange = w.zone(ZoneId(0)).res::<Exchange>();
        let state = |id: u64| exchange.order(id).map(|o| (o.filled, o.open));
        assert_eq!(state(o1), Some((5, false)));
        assert_eq!(state(o2), Some((5, false)));
        assert_eq!(state(o3), Some((2, true)));
    }
    // Bea locked 1,320 and paid 450 + 500 + 200.
    assert_eq!(w.cash(bea), Money(10_000 - 1_320));
    let result = ok(&mut w, bea, "market_claim", Value::Null);
    assert_eq!(result["claimed"], json!({ "cash": 170, "items": { "wheat": 12 } }));
    assert_eq!(conserved(&w), start);
}

#[test]
fn the_older_order_sets_the_price_and_bids_queue_by_price_then_time() {
    let mut w = world();
    w.set_param("finance.exchange_fee_bp", 0);
    let b1 = trader(&mut w, "B1", 1_000);
    let b2 = trader(&mut w, "B2", 1_000);
    let b3 = trader(&mut w, "B3", 1_000);
    let sam = seller(&mut w, "Sam", "ore", 4);
    let o1 = place(&mut w, b1, "buy", "ore", 2, 50);
    let o2 = place(&mut w, b2, "buy", "ore", 2, 60);
    let o3 = place(&mut w, b3, "buy", "ore", 2, 60);

    // Sam sells 3 for at least 40. The bids were first, so they set the
    // price: the higher bids first, the older of them before the newer.
    w.act_ok(sam, "order_place", json!({ "item": "ore", "side": "sell", "qty": 3, "price": 40 }));
    let frames = w.step(1);
    let fills: Vec<(u64, i64, i64)> = events(&frames, "finance.order_filled")
        .iter()
        .map(|e| {
            (
                e.data["buyer"].as_u64().expect("a buyer"),
                e.data["qty"].as_i64().expect("a quantity"),
                e.data["price"].as_i64().expect("a price"),
            )
        })
        .collect();
    assert_eq!(fills, [(b2.0, 2, 60), (b3.0, 1, 60)]);
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert_eq!(exchange.order(o1).map(|o| o.filled), Some(0));
    assert_eq!(exchange.order(o2).map(|o| o.filled), Some(2));
    assert_eq!(exchange.order(o3).map(|o| o.filled), Some(1));
    let ore = w.world.blueprint().item_id("ore").expect("ore is an item");
    assert_eq!(exchange.book(ore).map(|book| book.last_price()), Some(60));
    drop(exchange);
    assert_eq!(w.observe(sam).sections["market"]["claimable"]["cash"], json!(180));
    conserved(&w);
}

#[test]
fn orders_that_do_not_cross_wait() {
    let mut w = world();
    let sam = seller(&mut w, "Sam", "wheat", 3);
    let bea = trader(&mut w, "Bea", 1_000);
    let sell = place(&mut w, sam, "sell", "wheat", 3, 100);
    let buy = place(&mut w, bea, "buy", "wheat", 3, 99);
    w.step(5);
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert_eq!(exchange.order(sell).map(|o| (o.filled, o.open)), Some((0, true)));
    assert_eq!(exchange.order(buy).map(|o| (o.filled, o.open)), Some((0, true)));
    assert_eq!(exchange.orders_of(bea).map(|o| o.side).collect::<Vec<_>>(), [Side::Buy]);
    drop(exchange);
    assert_eq!(w.money(Holder::Escrow(buy)), Money(297));
    assert_eq!(w.money(CITY), Money(0));
    conserved(&w);
}

#[test]
fn order_cancel_returns_the_locked_part_and_the_fills() {
    let mut w = world();
    w.set_param("finance.exchange_fee_bp", 100);
    let sam = seller(&mut w, "Sam", "wheat", 10);
    let bea = trader(&mut w, "Bea", 2_000);
    let start = conserved(&w);
    let sell = place(&mut w, sam, "sell", "wheat", 10, 100);
    place(&mut w, bea, "buy", "wheat", 4, 100);
    let waiting = place(&mut w, bea, "buy", "wheat", 5, 80);
    assert_eq!(w.cash(bea), Money(2_000 - 400 - 400));

    // Sam cancels after 4 of 10 were sold: 6 wheat and 396 come back.
    let result = ok(&mut w, sam, "order_cancel", json!({ "order": sell }));
    assert_eq!(
        result,
        json!({
            "cancelled": sell,
            "filled": 4,
            "unfilled": 6,
            "returned": { "cash": 396, "items": { "wheat": 6 } },
        })
    );
    assert_eq!((w.cash(sam), w.qty(Holder::Resident(sam), "wheat")), (Money(396), 6));
    assert_eq!(w.money(CITY), Money(4));

    // Bea cancels the order that nobody took: the 400 come back.
    let result = ok(&mut w, bea, "order_cancel", json!({ "order": waiting }));
    assert_eq!(result["returned"], json!({ "cash": 400, "items": {} }));
    assert_eq!(w.cash(bea), Money(1_600));
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert!(exchange.order(sell).is_none() && exchange.order(waiting).is_none());
    let wheat = w.world.blueprint().item_id("wheat").expect("wheat is an item");
    assert!(exchange.levels(wheat, Side::Buy, 5).is_empty());
    assert!(exchange.levels(wheat, Side::Sell, 5).is_empty());
    drop(exchange);

    // A cancelled order no longer trades.
    let late = seller(&mut w, "Late", "wheat", 5);
    place(&mut w, late, "sell", "wheat", 5, 80);
    w.step(3);
    assert_eq!(w.cash(bea), Money(1_600));
    assert_eq!(conserved(&w), start);
}

#[test]
fn a_trader_cannot_trade_with_itself() {
    let mut w = world();
    let tom = trader(&mut w, "Tom", 5_000);
    w.give_item(tom, "wheat", 10);
    place(&mut w, tom, "sell", "wheat", 5, 100);
    // A bid at or above the own ask would match it.
    assert_eq!(place_err(&mut w, tom, "buy", "wheat", 1, 100).code, ErrorCode::Conflict);
    assert_eq!(place_err(&mut w, tom, "buy", "wheat", 1, 150).code, ErrorCode::Conflict);
    // A lower bid is fine, and then an ask at or below it is refused.
    place(&mut w, tom, "buy", "wheat", 2, 90);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 1, 90).code, ErrorCode::Conflict);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 1, 80).code, ErrorCode::Conflict);
    place(&mut w, tom, "sell", "wheat", 1, 95);
    // Another item has its own book.
    place(&mut w, tom, "buy", "ore", 1, 100);
    w.step(3);
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert_eq!(exchange.orders_of(tom).count(), 4);
    assert!(exchange.orders_of(tom).all(|o| o.filled == 0), "no order of Tom traded");
    drop(exchange);
    assert!(events(&w.step(1), "finance.order_filled").is_empty());
    conserved(&w);
}

#[test]
fn order_place_refusals() {
    let mut w = world();
    let tom = trader(&mut w, "Tom", 1_000);
    w.give_item(tom, "wheat", 3);
    w.give_item(tom, "powder", 3);

    assert_eq!(place_err(&mut w, tom, "sell", "gold", 1, 10).code, ErrorCode::InvalidArgs);
    assert_eq!(place_err(&mut w, tom, "sell", "powder", 1, 10).code, ErrorCode::NotAllowed);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 0, 10).code, ErrorCode::InvalidArgs);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 100_001, 10).code, ErrorCode::InvalidArgs);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 1, 0).code, ErrorCode::InvalidArgs);
    assert_eq!(
        place_err(&mut w, tom, "sell", "wheat", 1, 100_000_001).code,
        ErrorCode::InvalidArgs
    );
    assert_eq!(place_err(&mut w, tom, "hold", "wheat", 1, 10).code, ErrorCode::InvalidArgs);

    let error = place_err(&mut w, tom, "sell", "wheat", 4, 10);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 4, "available": 3 }));
    let error = place_err(&mut w, tom, "buy", "wheat", 11, 100);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_100, "available": 1_000 }));

    w.set_pos(tom, STREET);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 1, 10).code, ErrorCode::NotHere);
    w.set_pos(tom, AT_EXCHANGE);

    // At most two orders with this setting, open or not yet claimed.
    w.set_param("finance.orders_max_per_trader", 2);
    w.step(1);
    place(&mut w, tom, "sell", "wheat", 1, 500);
    place(&mut w, tom, "sell", "wheat", 1, 600);
    assert_eq!(place_err(&mut w, tom, "sell", "wheat", 1, 700).code, ErrorCode::Conflict);

    // Nothing of all this moved value.
    assert_eq!(w.cash(tom), Money(1_000));
    assert_eq!(w.qty(Holder::Resident(tom), "wheat"), 1);
    assert_eq!(w.qty(Holder::Resident(tom), "powder"), 3);
    conserved(&w);
}

#[test]
fn an_order_is_checked_again_when_it_is_placed() {
    let mut w = world();
    let tom = trader(&mut w, "Tom", 1_000);
    let args = json!({ "item": "ore", "side": "buy", "qty": 10, "price": 100 });

    // The check passes, then Tom's cash goes away before the tick ends.
    w.act_ok(tom, "order_place", args.clone());
    w.with_resident(tom, |realm, _| {
        realm
            .ledger_mut()
            .transfer_money(Holder::Resident(tom), CITY, Money(1), "test")
            .expect("Tom has the cent");
    });
    w.step(1);
    let report = w.last_report(tom).expect("the order reports");
    assert_eq!(report.error.map(|e| e.code), Some(ErrorCode::Insufficient));
    assert_eq!(w.cash(tom), Money(999));

    // The check passes, then something moves Tom out of the exchange.
    w.give_money(tom, Money(1));
    w.act_ok(tom, "order_place", args);
    w.set_pos(tom, STREET);
    w.step(1);
    let report = w.last_report(tom).expect("the order reports");
    assert_eq!(report.error.map(|e| e.code), Some(ErrorCode::NotHere));
    assert_eq!(w.cash(tom), Money(1_000), "nothing is locked");
    assert_eq!(w.zone(ZoneId(0)).res::<Exchange>().orders_of(tom).count(), 0);
    conserved(&w);
}

#[test]
fn order_cancel_and_market_claim_refusals() {
    let mut w = world();
    let sam = seller(&mut w, "Sam", "wheat", 5);
    let tom = trader(&mut w, "Tom", 1_000);
    let sell = place(&mut w, sam, "sell", "wheat", 5, 100);

    let cancel = |w: &mut _, r, order: u64| err(w, r, "order_cancel", json!({ "order": order }));
    assert_eq!(cancel(&mut w, tom, sell).code, ErrorCode::NotFound, "not the order of Tom");
    assert_eq!(cancel(&mut w, sam, sell + 1).code, ErrorCode::NotFound);
    assert_eq!(err(&mut w, tom, "market_claim", Value::Null).code, ErrorCode::NotFound);
    assert_eq!(
        err(&mut w, sam, "market_claim", Value::Null).code,
        ErrorCode::NotFound,
        "an open order without a fill holds nothing to collect"
    );

    w.set_pos(sam, STREET);
    assert_eq!(cancel(&mut w, sam, sell).code, ErrorCode::NotHere);
    assert_eq!(err(&mut w, sam, "market_claim", Value::Null).code, ErrorCode::NotHere);
    assert!(!w.observe(sam).sections.contains_key("market"), "only at the exchange");
    assert_eq!(w.qty(Holder::Escrow(sell), "wheat"), 5);
    conserved(&w);
}

#[test]
fn an_order_trades_while_its_trader_is_away() {
    let mut w = world();
    w.set_param("finance.exchange_fee_bp", 0);
    let sam = seller(&mut w, "Sam", "wheat", 5);
    let sell = place(&mut w, sam, "sell", "wheat", 5, 100);
    w.set_pos(sam, SUBURB);
    let bea = trader(&mut w, "Bea", 1_000);
    place(&mut w, bea, "buy", "wheat", 5, 100);
    w.step(1);
    // The notice follows Sam to the other zone.
    assert_eq!(
        notice(&w, sam, "finance.order_filled").map(|data| data["value"].clone()),
        Some(json!(500))
    );
    assert_eq!(w.cash(sam), Money(0), "the cash waits at the exchange");
    assert_eq!(w.money(Holder::Escrow(sell)), Money(500));
    w.set_pos(sam, AT_EXCHANGE);
    ok(&mut w, sam, "market_claim", Value::Null);
    assert_eq!(w.cash(sam), Money(500));
    conserved(&w);
}

#[test]
fn market_book_shows_the_best_levels() {
    let mut w = world();
    let sam = seller(&mut w, "Sam", "wheat", 50);
    let bea = trader(&mut w, "Bea", 100_000);
    let reader = w.spawn_at("Reader", STREET);
    let book = |w: &mut TestWorld<'_>, r| ok(w, r, "market_book", json!({ "item": "wheat" }));

    // The book of an item that nobody trades is empty.
    assert_eq!(
        book(&mut w, reader),
        json!({ "item": "wheat", "bids": [], "asks": [], "last_price": 0 })
    );
    for (qty, price) in [(5, 110), (3, 105), (2, 105), (1, 120)] {
        place(&mut w, sam, "sell", "wheat", qty, price);
    }
    for (qty, price) in [(4, 90), (6, 95), (1, 90)] {
        place(&mut w, bea, "buy", "wheat", qty, price);
    }
    // The verb is free and works anywhere in the district of the exchange.
    assert_eq!(
        book(&mut w, reader),
        json!({
            "item": "wheat",
            "bids": [
                { "price": 95, "qty": 6, "orders": 1 },
                { "price": 90, "qty": 5, "orders": 2 },
            ],
            "asks": [
                { "price": 105, "qty": 5, "orders": 2 },
                { "price": 110, "qty": 5, "orders": 1 },
                { "price": 120, "qty": 1, "orders": 1 },
            ],
            "last_price": 0,
        })
    );
    // `book_depth` limits the levels of each side.
    w.set_param("finance.book_depth", 1);
    w.step(1);
    let short = book(&mut w, reader);
    assert_eq!(short["bids"], json!([{ "price": 95, "qty": 6, "orders": 1 }]));
    assert_eq!(short["asks"], json!([{ "price": 105, "qty": 5, "orders": 2 }]));

    // After a trade the book shows the last price.
    place(&mut w, bea, "buy", "wheat", 4, 105);
    let after = book(&mut w, reader);
    assert_eq!(after["last_price"], json!(105));
    assert_eq!(after["asks"], json!([{ "price": 105, "qty": 1, "orders": 1 }]));

    assert_eq!(
        err(&mut w, reader, "market_book", json!({ "item": "gold" })).code,
        ErrorCode::InvalidArgs
    );
    w.set_pos(reader, SUBURB);
    assert_eq!(
        err(&mut w, reader, "market_book", json!({ "item": "wheat" })).code,
        ErrorCode::NotHere
    );
    conserved(&w);
}

#[test]
fn the_fee_goes_to_the_city_and_is_rounded_down() {
    let mut w = world();
    // 2.5 percent.
    w.set_param("finance.exchange_fee_bp", 250);
    let sam = seller(&mut w, "Sam", "fish", 10);
    let bea = trader(&mut w, "Bea", 1_000);
    let start = conserved(&w);
    let sell = place(&mut w, sam, "sell", "fish", 10, 33);
    // 3 * 33 = 99, the fee is 2.475: 2.
    place(&mut w, bea, "buy", "fish", 3, 33);
    assert_eq!(w.money(CITY), Money(2));
    assert_eq!(w.money(Holder::Escrow(sell)), Money(97));
    // 7 * 33 = 231, the fee is 5.775: 5.
    place(&mut w, bea, "buy", "fish", 7, 40);
    assert_eq!(w.money(CITY), Money(7));
    assert_eq!(w.money(Holder::Escrow(sell)), Money(97 + 226));
    assert_eq!(conserved(&w), start);
}

#[test]
fn market_claim_leaves_what_an_open_order_still_locks() {
    let mut w = world();
    w.set_param("finance.exchange_fee_bp", 0);
    let sam = seller(&mut w, "Sam", "wheat", 10);
    let bea = trader(&mut w, "Bea", 5_000);
    let start = conserved(&w);
    // Sam offers 4 at 100. Bea bids 120 for 10: 4 trade at 100, and the
    // bid for the other 6 stays open with 720 locked.
    place(&mut w, sam, "sell", "wheat", 4, 100);
    let buy = place(&mut w, bea, "buy", "wheat", 10, 120);
    assert_eq!(w.money(Holder::Escrow(buy)), Money(1_200 - 400));
    assert_eq!(
        w.observe(bea).sections["market"],
        json!({
            "orders": [{
                "order": buy, "item": "wheat", "side": "buy", "price": 120, "qty": 10,
                "filled": 4, "open": true,
            }],
            // 20 less than the limit on each of the 4 units.
            "claimable": { "cash": 80, "items": { "wheat": 4 } },
        })
    );

    // The claim hands over the 4 wheat and the 80, not the 720.
    let result = ok(&mut w, bea, "market_claim", Value::Null);
    assert_eq!(
        result,
        json!({ "claimed": { "cash": 80, "items": { "wheat": 4 } }, "open_orders": 1 })
    );
    assert_eq!(w.cash(bea), Money(5_000 - 1_200 + 80));
    assert_eq!(w.money(Holder::Escrow(buy)), Money(720));
    assert_eq!(
        err(&mut w, bea, "market_claim", Value::Null).code,
        ErrorCode::NotFound,
        "the locked cash is not there to collect"
    );

    // The open part still trades, at the price of the bid, which is older.
    place(&mut w, sam, "sell", "wheat", 6, 110);
    let exchange = w.zone(ZoneId(0)).res::<Exchange>();
    assert_eq!(exchange.order(buy).map(|o| (o.filled, o.open)), Some((10, false)));
    drop(exchange);
    assert_eq!(w.money(Holder::Escrow(buy)), Money(0));
    let result = ok(&mut w, bea, "market_claim", Value::Null);
    assert_eq!(result["claimed"], json!({ "cash": 0, "items": { "wheat": 6 } }));
    assert_eq!(w.qty(Holder::Resident(bea), "wheat"), 10);
    assert_eq!(w.cash(bea), Money(5_000 - 400 - 720));
    assert_eq!(conserved(&w), start);
}

#[test]
fn the_market_section_shows_a_few_orders_in_brief_detail() {
    let mut w = world();
    let tom = trader(&mut w, "Tom", 10_000);
    let shown = usize::try_from(w.param("finance.observe_list_max")).expect("a small number");
    let ids: Vec<u64> =
        (0..shown as i64 + 2).map(|i| place(&mut w, tom, "buy", "ore", 1, 10 + i)).collect();

    // Brief detail: the oldest orders, and how many more there are.
    let brief = w.observe(tom).sections["market"].clone();
    let listed: Vec<u64> = brief["orders"]
        .as_array()
        .expect("a list of orders")
        .iter()
        .map(|order| order["order"].as_u64().expect("an order id"))
        .collect();
    assert_eq!(listed, ids[..shown]);
    assert_eq!(brief["more_orders"], json!(2));
    assert_eq!(brief["claimable"], json!({ "cash": 0, "items": {} }));

    // Full detail: every order, and no count of the rest.
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let full = w.observe_with(tom, &opts).sections["market"].clone();
    assert_eq!(full["orders"].as_array().map(Vec::len), Some(shown + 2));
    assert!(full.get("more_orders").is_none());
    conserved(&w);
}

#[test]
fn a_city_can_have_its_own_fee() {
    let mut w = world();
    // The nation keeps 50 basis points. The city of the exchange takes 2
    // percent.
    let reply = w.world.admin(AdminCommand::SetParam {
        name: "finance.exchange_fee_bp".to_string(),
        value: 200,
        city: Some(CityId(0)),
    });
    assert_eq!(reply, AdminReply::Ok);
    assert_eq!(w.param("finance.exchange_fee_bp"), 50);
    w.step(1);
    let sam = seller(&mut w, "Sam", "wheat", 10);
    let bea = trader(&mut w, "Bea", 1_000);
    let sell = place(&mut w, sam, "sell", "wheat", 10, 100);
    place(&mut w, bea, "buy", "wheat", 10, 100);
    assert_eq!(w.money(CITY), Money(20));
    assert_eq!(w.money(Holder::Escrow(sell)), Money(980));
    conserved(&w);
}
