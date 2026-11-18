//! Shares: issue, trade in the global book, cancel and dividends.

mod common;

use common::{
    SUBURB, account_holder, balance, conserved, customer, err, events, fund_global, grant, notice,
    ok, world,
};
use serde_json::{Value, json};
use unsup_finance::shares::Shares;
use unsup_kernel::protocol::{Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId, ResidentId};

/// The organization of the tests.
const ACME: OrgId = OrgId(3_000_000);
const TREASURY: Holder = Holder::Org(ACME);

/// Spawns a resident with an account and the permission to spend the
/// treasury of ACME.
fn treasurer(w: &mut TestWorld<'_>, name: &str) -> ResidentId {
    let r = account_holder(w, name, 0);
    grant(w, r, ACME, &["treasury.spend"]);
    r
}

fn issue(w: &mut TestWorld<'_>, r: ResidentId, shares: i64, price: i64) -> Value {
    ok(w, r, "issue_shares", json!({ "org": ACME, "shares": shares, "price": price }))
}

fn buy(w: &mut TestWorld<'_>, r: ResidentId, shares: i64, price: i64) -> Value {
    ok(w, r, "buy_shares", json!({ "org": ACME, "shares": shares, "price": price }))
}

fn sell(w: &mut TestWorld<'_>, r: ResidentId, shares: i64, price: i64) -> Value {
    ok(w, r, "sell_shares", json!({ "org": ACME, "shares": shares, "price": price }))
}

/// Returns (held, locked) of a resident in ACME.
fn holding(w: &TestWorld<'_>, r: ResidentId) -> (i64, i64) {
    let shares = w.global().res::<Shares>();
    let holding = shares.company(ACME).map(|company| company.holding(r)).unwrap_or_default();
    (holding.held, holding.locked)
}

/// Checks that the share register adds up: the holdings are the shares
/// outstanding, and nobody locks more than it holds.
fn register_adds_up(w: &TestWorld<'_>) -> i64 {
    let shares = w.global().res::<Shares>();
    let company = shares.company(ACME).expect("ACME has shares");
    let held: i64 = company.holders().map(|(_, holding)| holding.held).sum();
    assert_eq!(held, company.outstanding());
    assert!(company.holders().all(|(_, h)| h.held > 0 && h.locked >= 0 && h.locked <= h.held));
    company.outstanding()
}

#[test]
fn shares_are_issued_bought_and_sold() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let bob = account_holder(&mut w, "Bob", 10_000);
    let cat = account_holder(&mut w, "Cat", 10_000);
    let dan = account_holder(&mut w, "Dan", 10_000);
    let start = conserved(&w);

    // Fay offers 100 new shares at 50. Nobody bids yet, so they wait.
    w.act_ok(fay, "issue_shares", json!({ "org": ACME, "shares": 100, "price": 50 }));
    let frames = w.step(2);
    let report = w.last_report(fay).expect("the issue reports");
    let offer = report.result["order"].as_u64().expect("an order id");
    assert_eq!(report.result, json!({ "order": offer, "sold": 0, "value": 0, "on_offer": 100 }));
    let issued = events(&frames, "finance.shares_issued");
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].data, json!({ "org": ACME, "shares": 100, "price": 50 }));

    // Bob bids 60 for 30. The offer waited, so the price is 50; the money
    // goes to the treasury of ACME and the rest of the lock comes back.
    w.act_ok(bob, "buy_shares", json!({ "org": ACME, "shares": 30, "price": 60 }));
    let frames = w.step(2);
    let report = w.last_report(bob).expect("the order reports");
    assert_eq!(
        report.result,
        json!({
            "order": null, "side": "buy", "traded": 30, "value": 1_500, "waiting": 0,
            "balance": 8_500,
        })
    );
    let traded = events(&frames, "finance.shares_traded");
    assert_eq!(traded.len(), 1);
    assert_eq!(
        traded[0].data,
        json!({ "org": ACME, "shares": 30, "price": 50, "buyer": bob, "seller": null })
    );
    assert_eq!(w.money(TREASURY), Money(1_500));
    assert_eq!(holding(&w, bob), (30, 0));
    assert_eq!(
        notice(&w, fay, "finance.share_order_filled"),
        Some(json!({
            "order": offer, "org": ACME, "side": "sell", "shares": 30, "price": 50,
            "unfilled": 70,
        }))
    );

    // Cat wants 100 at 50: 70 are on offer, the other 30 wait as a bid
    // that locks 1,500.
    let result = buy(&mut w, cat, 100, 50);
    let bid = result["order"].as_u64().expect("the rest waits");
    assert_eq!(
        result,
        json!({
            "order": bid, "side": "buy", "traded": 70, "value": 3_500, "waiting": 30,
            "balance": 5_000,
        })
    );
    assert_eq!(w.money(Holder::Escrow(bid)), Money(1_500));
    assert_eq!(w.money(TREASURY), Money(5_000));
    assert_eq!(register_adds_up(&w), 100);

    // Bob sells 10 at 50 to the bid of Cat, and offers 10 more at 70.
    let result = sell(&mut w, bob, 10, 50);
    assert_eq!(result["traded"], json!(10));
    assert_eq!(result["balance"], json!(9_000));
    assert_eq!((holding(&w, bob), holding(&w, cat)), ((20, 0), (80, 0)));
    let result = sell(&mut w, bob, 10, 70);
    let ask = result["order"].as_u64().expect("the offer waits");
    assert_eq!(holding(&w, bob), (20, 10));

    // The book shows both sides.
    assert_eq!(
        ok(&mut w, dan, "shares_book", json!({ "org": ACME })),
        json!({
            "org": ACME,
            "bids": [{ "price": 50, "qty": 20, "orders": 1 }],
            "asks": [{ "price": 70, "qty": 10, "orders": 1 }],
            "last_price": 50,
            "outstanding": 100,
            "on_offer": 0,
        })
    );

    // Dan bids 80 and pays the 70 of the offer that waited.
    let result = buy(&mut w, dan, 4, 80);
    assert_eq!((result["traded"].clone(), result["value"].clone()), (json!(4), json!(280)));
    assert_eq!(balance(&w, dan), 10_000 - 280);
    assert_eq!(balance(&w, bob), 9_000 + 280);
    assert_eq!((holding(&w, bob), holding(&w, dan)), ((16, 6), (4, 0)));
    assert_eq!(w.global().res::<Shares>().order(ask).map(|o| o.filled), Some(4));
    assert_eq!(register_adds_up(&w), 100);
    assert_eq!(conserved(&w), start, "share trades create no money");

    // The `bank` section lists the shares.
    assert_eq!(
        w.observe(bob).sections["bank"]["shares"],
        json!({ "holdings": [{ "org": ACME, "shares": 16, "locked": 6 }], "open_orders": 1 })
    );
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    assert_eq!(
        w.observe_with(bob, &opts).sections["bank"]["shares"]["orders"],
        json!([{
            "order": ask, "org": ACME, "side": "sell", "issue": false, "price": 70,
            "shares": 10, "filled": 4,
        }])
    );
}

#[test]
fn share_trades_work_from_anywhere() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let bob = account_holder(&mut w, "Bob", 1_000);
    w.set_pos(fay, SUBURB);
    w.set_pos(bob, SUBURB);
    issue(&mut w, fay, 10, 100);
    assert_eq!(buy(&mut w, bob, 10, 100)["traded"], json!(10));
    assert_eq!(holding(&w, bob), (10, 0));
    conserved(&w);
}

#[test]
fn a_treasurer_can_buy_the_shares_that_it_issued_for_the_organization() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    w.give_money(fay, Money(5_000));
    ok(&mut w, fay, "deposit", json!({ "amount": 5_000 }));
    issue(&mut w, fay, 100, 10);
    // The seller is the organization, not Fay: this is not a self-trade.
    let result = buy(&mut w, fay, 40, 10);
    assert_eq!(result["traded"], json!(40));
    assert_eq!(holding(&w, fay), (40, 0));
    assert_eq!(w.money(TREASURY), Money(400));
    assert_eq!(balance(&w, fay), 4_600);
    conserved(&w);
}

#[test]
fn cancel_shares_gives_back_what_the_order_locks() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let bob = account_holder(&mut w, "Bob", 10_000);
    let start = conserved(&w);
    let offer = issue(&mut w, fay, 100, 50)["order"].as_u64().expect("an order id");
    buy(&mut w, bob, 20, 50);

    // A bid: the locked money comes back.
    let bid = buy(&mut w, bob, 10, 40)["order"].as_u64().expect("the bid waits");
    assert_eq!(balance(&w, bob), 10_000 - 1_000 - 400);
    let result = ok(&mut w, bob, "cancel_shares", json!({ "order": bid }));
    assert_eq!(result, json!({ "cancelled": bid, "unfilled": 10 }));
    assert_eq!(balance(&w, bob), 9_000);

    // An offer: the shares are free again.
    let ask = sell(&mut w, bob, 15, 90)["order"].as_u64().expect("the offer waits");
    assert_eq!(holding(&w, bob), (20, 15));
    let error = err(&mut w, bob, "sell_shares", json!({ "org": ACME, "shares": 6, "price": 95 }));
    assert_eq!(error.code, ErrorCode::Insufficient, "locked shares cannot be sold twice");
    assert_eq!(error.details, json!({ "needed": 6, "available": 5 }));
    ok(&mut w, bob, "cancel_shares", json!({ "order": ask }));
    assert_eq!(holding(&w, bob), (20, 0));

    // An issue: the new shares are withdrawn.
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.on_offer()), Some(80));
    assert_eq!(
        err(&mut w, bob, "cancel_shares", json!({ "order": offer })).code,
        ErrorCode::NotFound,
        "only the resident that placed an order cancels it"
    );
    ok(&mut w, fay, "cancel_shares", json!({ "order": offer }));
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.on_offer()), Some(0));
    assert_eq!(
        err(&mut w, fay, "cancel_shares", json!({ "order": offer })).code,
        ErrorCode::NotFound
    );
    assert_eq!(register_adds_up(&w), 20);
    assert_eq!(conserved(&w), start);
}

#[test]
fn share_order_refusals() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let bob = account_holder(&mut w, "Bob", 1_000);
    let cat = customer(&mut w, "Cat", 0);
    let order = |w: &mut _, r, verb: &str, shares: i64, price: i64| {
        err(w, r, verb, json!({ "org": ACME, "shares": shares, "price": price }))
    };

    // Before the first issue the organization has no shares.
    assert_eq!(order(&mut w, bob, "buy_shares", 1, 10).code, ErrorCode::NotFound);
    assert_eq!(err(&mut w, bob, "shares_book", json!({ "org": ACME })).code, ErrorCode::NotFound);

    // Only a treasurer issues.
    assert_eq!(order(&mut w, bob, "issue_shares", 10, 10).code, ErrorCode::NotAllowed);
    assert_eq!(order(&mut w, fay, "issue_shares", 0, 10).code, ErrorCode::InvalidArgs);
    assert_eq!(order(&mut w, fay, "issue_shares", 10, 0).code, ErrorCode::InvalidArgs);
    w.set_param("finance.shares_max_per_org", 150);
    w.step(1);
    issue(&mut w, fay, 100, 10);
    let error = order(&mut w, fay, "issue_shares", 51, 10);
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.details, json!({ "limit": 150, "existing": 100 }));

    assert_eq!(order(&mut w, cat, "buy_shares", 1, 10).code, ErrorCode::NotFound, "no account");
    assert_eq!(order(&mut w, bob, "buy_shares", 0, 10).code, ErrorCode::InvalidArgs);
    assert_eq!(order(&mut w, bob, "buy_shares", 1, 0).code, ErrorCode::InvalidArgs);
    let error = order(&mut w, bob, "buy_shares", 101, 10);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_010, "available": 1_000 }));
    let error = order(&mut w, bob, "sell_shares", 1, 10);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1, "available": 0 }));

    // A resident cannot trade with itself.
    buy(&mut w, bob, 10, 10);
    sell(&mut w, bob, 5, 30);
    assert_eq!(order(&mut w, bob, "buy_shares", 1, 30).code, ErrorCode::Conflict);
    buy(&mut w, bob, 1, 5);
    assert_eq!(order(&mut w, bob, "sell_shares", 1, 5).code, ErrorCode::Conflict);

    // At most two open orders with this setting.
    w.set_param("finance.share_orders_max", 2);
    assert_eq!(order(&mut w, bob, "buy_shares", 1, 4).code, ErrorCode::Conflict);
    // An offer of new shares is an order of the treasurer that placed it:
    // Fay has one, can place a second and not a third.
    issue(&mut w, fay, 10, 20);
    assert_eq!(order(&mut w, fay, "issue_shares", 10, 30).code, ErrorCode::Conflict);
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.on_offer()), Some(90 + 10));
    assert_eq!(register_adds_up(&w), 10);
    conserved(&w);
}

#[test]
fn a_dividend_is_split_by_shares_and_loses_no_cent() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let a = account_holder(&mut w, "A", 10_000);
    let b = account_holder(&mut w, "B", 10_000);
    let c = account_holder(&mut w, "C", 10_000);
    issue(&mut w, fay, 100, 10);
    buy(&mut w, a, 50, 10);
    buy(&mut w, b, 30, 10);
    buy(&mut w, c, 20, 10);
    // Shares that wait in a sell order still earn the dividend.
    sell(&mut w, c, 5, 500);
    assert_eq!(w.money(TREASURY), Money(1_000));
    let start = conserved(&w);
    let before = [balance(&w, a), balance(&w, b), balance(&w, c)];

    // 99 cents over 50, 30 and 20 shares: 49.5, 29.7 and 19.8. Each gets
    // the whole cents, and the two cents that are left go to the largest
    // remainders: C (0.8) and B (0.7).
    w.act_ok(fay, "pay_dividend", json!({ "org": ACME, "amount": 99 }));
    let frames = w.step(2);
    let report = w.last_report(fay).expect("the dividend reports");
    assert_eq!(report.result, json!({ "paid": 99, "shares": 100, "holders": 3 }));
    let after = [balance(&w, a), balance(&w, b), balance(&w, c)];
    assert_eq!([after[0] - before[0], after[1] - before[1], after[2] - before[2]], [49, 30, 20]);
    assert_eq!(w.money(TREASURY), Money(901));
    let paid = events(&frames, "finance.dividend_paid");
    assert_eq!(paid.len(), 1);
    assert_eq!(paid[0].data, json!({ "org": ACME, "amount": 99, "shares": 100, "holders": 3 }));
    assert_eq!(
        notice(&w, b, "finance.dividend_received"),
        Some(json!({ "org": ACME, "amount": 30 }))
    );

    // Equal remainders: the lowest resident id gets the cent. 100 over 50,
    // 30 and 20 has no remainder; 1 cent goes to A, with the largest part.
    ok(&mut w, fay, "pay_dividend", json!({ "org": ACME, "amount": 1 }));
    let last = [balance(&w, a), balance(&w, b), balance(&w, c)];
    assert_eq!([last[0] - after[0], last[1] - after[1], last[2] - after[2]], [1, 0, 0]);
    assert_eq!(w.money(TREASURY), Money(900));
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.dividends_paid()), Some(100));
    assert_eq!(conserved(&w), start, "a dividend is a transfer");
}

#[test]
fn equal_holders_share_a_dividend_that_does_not_divide() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let holders: Vec<ResidentId> =
        (0..3).map(|i| account_holder(&mut w, &format!("H{i}"), 1_000)).collect();
    issue(&mut w, fay, 3, 100);
    for &holder in &holders {
        buy(&mut w, holder, 1, 100);
    }
    let before: Vec<i64> = holders.iter().map(|&h| balance(&w, h)).collect();
    ok(&mut w, fay, "pay_dividend", json!({ "org": ACME, "amount": 200 }));
    let gains: Vec<i64> =
        holders.iter().zip(&before).map(|(&h, &old)| balance(&w, h) - old).collect();
    // 66.67 each: the two cents that are left go to the two lowest ids.
    assert_eq!(gains, [67, 67, 66]);
    assert_eq!(w.money(TREASURY), Money(100));
    conserved(&w);
}

#[test]
fn pay_dividend_refusals() {
    let mut w = world();
    let fay = treasurer(&mut w, "Fay");
    let bob = account_holder(&mut w, "Bob", 1_000);
    let pay = |w: &mut _, r, amount: i64| {
        err(w, r, "pay_dividend", json!({ "org": ACME, "amount": amount }))
    };
    assert_eq!(pay(&mut w, bob, 10).code, ErrorCode::NotAllowed);
    assert_eq!(pay(&mut w, fay, 0).code, ErrorCode::InvalidArgs);
    // No shares yet, then shares on offer that nobody holds.
    fund_global(&mut w, TREASURY, 500);
    assert_eq!(pay(&mut w, fay, 10).code, ErrorCode::NotFound);
    issue(&mut w, fay, 10, 100);
    assert_eq!(pay(&mut w, fay, 10).code, ErrorCode::NotFound);
    buy(&mut w, bob, 1, 100);
    let error = pay(&mut w, fay, 601);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 601, "available": 600 }));
    assert_eq!(w.money(TREASURY), Money(600));
    // A permission in another organization gives nothing here.
    grant(&mut w, bob, OrgId(3_000_001), &["treasury.spend"]);
    assert_eq!(pay(&mut w, bob, 10).code, ErrorCode::NotAllowed);
    conserved(&w);
}

#[test]
fn the_bank_section_shows_a_few_holdings_in_brief_detail() {
    let mut w = world();
    let bob = account_holder(&mut w, "Bob", 10_000);
    let shown = usize::try_from(w.param("finance.observe_list_max")).expect("a small number");
    // Bob buys one share of each of several organizations: two more than
    // a brief observation lists.
    let orgs: Vec<OrgId> = (0..shown as u64 + 2).map(|i| OrgId(3_000_000 + i)).collect();
    for (i, &org) in orgs.iter().enumerate() {
        let fay = account_holder(&mut w, &format!("Fay{i}"), 0);
        grant(&mut w, fay, org, &["treasury.spend"]);
        ok(&mut w, fay, "issue_shares", json!({ "org": org, "shares": 10, "price": 5 }));
        ok(&mut w, bob, "buy_shares", json!({ "org": org, "shares": 1, "price": 5 }));
    }

    // Brief detail: the organizations with the lowest ids, and how many
    // more there are.
    let brief = w.observe(bob).sections["bank"]["shares"].clone();
    let listed: Vec<Value> =
        orgs[..shown].iter().map(|org| json!({ "org": org, "shares": 1, "locked": 0 })).collect();
    assert_eq!(brief, json!({ "holdings": listed, "open_orders": 0, "more_holdings": 2 }));

    // Full detail: every holding and the open orders.
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let full = w.observe_with(bob, &opts).sections["bank"]["shares"].clone();
    assert_eq!(full["holdings"].as_array().map(Vec::len), Some(shown + 2));
    assert!(full.get("more_holdings").is_none());
    assert_eq!(full["orders"], json!([]));
    conserved(&w);
}
