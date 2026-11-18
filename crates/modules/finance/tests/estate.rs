//! The estate of a resident that leaves the world: account, loan, shares
//! and orders go to the heir.

mod common;

use common::{
    AT_EXCHANGE, OPEN_FEE, SUBURB, account_holder, balance, bank_treasury, burned, conserved,
    customer, events, grant, minted, ok, world,
};
use serde_json::json;
use unsup_finance::bank::Bank;
use unsup_finance::exchange::Exchange;
use unsup_finance::shares::Shares;
use unsup_kernel::protocol::{AdminCommand, AdminReply};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, Holder, Money, OrgId, ResidentId, UNCLAIMED, ZoneId};

const ACME: OrgId = OrgId(3_000_000);
const CITY: Holder = Holder::Org(OrgId::city(CityId(0)));

/// Makes Ann: 10,000 on deposit, a loan of 5,000, 10 shares of ACME, a
/// share bid that locks 25, and at the exchange a sell order with 5 wheat
/// and a buy order that locks 100. She carries 900 in cash.
fn rich_ann(w: &mut TestWorld<'_>) -> ResidentId {
    w.set_param("finance.loan_base_limit", 20_000);
    let fay = account_holder(w, "Fay", 0);
    grant(w, fay, ACME, &["treasury.spend"]);
    ok(w, fay, "issue_shares", json!({ "org": ACME, "shares": 100, "price": 10 }));

    let ann = account_holder(w, "Ann", 10_000);
    ok(w, ann, "loan_request", json!({ "amount": 5_000, "term_days": 5 }));
    ok(w, ann, "buy_shares", json!({ "org": ACME, "shares": 10, "price": 10 }));
    ok(w, ann, "buy_shares", json!({ "org": ACME, "shares": 5, "price": 5 }));
    assert_eq!(balance(w, ann), 15_000 - 100 - 25);

    w.set_pos(ann, AT_EXCHANGE);
    w.give_money(ann, Money(1_000));
    w.give_item(ann, "wheat", 5);
    ok(w, ann, "order_place", json!({ "item": "wheat", "side": "sell", "qty": 5, "price": 100 }));
    ok(w, ann, "order_place", json!({ "item": "ore", "side": "buy", "qty": 2, "price": 50 }));
    assert_eq!(w.cash(ann), Money(900));
    ann
}

/// Checks that nothing of a resident is left in the finance module.
fn nothing_left(w: &TestWorld<'_>, r: ResidentId) {
    assert!(!w.global().res::<Bank>().is_open(r));
    assert_eq!(balance(w, r), 0);
    let shares = w.global().res::<Shares>();
    assert_eq!(shares.orders_of(r).count(), 0);
    assert_eq!(shares.portfolio(r).count(), 0);
    assert_eq!(w.zone(ZoneId(0)).res::<Exchange>().orders_of(r).count(), 0);
}

#[test]
fn an_heir_with_an_account_gets_everything() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    let bob = account_holder(&mut w, "Bob", 0);
    let total = conserved(&w);

    w.act_ok(ann, "test_die", json!({ "heir": bob }));
    let frames = w.step(3);
    assert!(!w.is_alive(ann));
    nothing_left(&w, ann);

    // The share bid gave its 25 back, the bank took the 5,000 of the loan
    // from the account, and the rest went to the account of Bob.
    assert_eq!(balance(&w, bob), 14_875 + 25 - 5_000);
    assert_eq!((minted(&w, "bank.loan_issue"), burned(&w, "bank.loan_repay")), (5_000, 5_000));
    let bank = w.global().res::<Bank>();
    assert_eq!((bank.outstanding_principal(), bank.written_off()), (0, 0));
    drop(bank);
    // Bob has the shares.
    let shares = w.global().res::<Shares>();
    let company = shares.company(ACME).expect("ACME has shares");
    assert_eq!((company.holding(bob).held, company.outstanding()), (10, 10));
    drop(shares);
    // The kernel gave Bob the cash of Ann; the exchange gave him what her
    // orders held: 100 in cash and 5 wheat.
    assert_eq!(w.cash(bob), Money(900 + 100));
    assert_eq!(w.qty(Holder::Resident(bob), "wheat"), 5);

    let settled = events(&frames, "finance.estate_settled");
    assert_eq!(settled.len(), 1);
    assert_eq!(
        settled[0].data,
        json!({
            "resident": ann,
            "heir": { "Resident": bob },
            "balance": 9_900,
            "loan_unpaid": 0,
            "shares": 10,
        })
    );
    assert!(events(&frames, "finance.loan_defaulted").is_empty(), "a death is not a default");
    assert_eq!(conserved(&w), total - 5_000);
}

#[test]
fn an_heir_without_an_account_gets_cash_wherever_it_is() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    let bob = customer(&mut w, "Bob", 0);
    w.set_pos(bob, SUBURB);
    let total = conserved(&w);

    w.act_ok(ann, "test_die", json!({ "heir": bob }));
    w.step(4);
    nothing_left(&w, ann);
    // 900 from the kernel, 9,900 from the bank, 100 from the exchange.
    assert_eq!(w.cash(bob), Money(900 + 9_900 + 100));
    assert_eq!(w.qty(Holder::Resident(bob), "wheat"), 5);
    assert_eq!(w.global().res::<Shares>().company(ACME).map(|c| c.holding(bob).held), Some(10));
    assert_eq!(conserved(&w), total - 5_000);
}

#[test]
fn without_an_heir_the_city_gets_the_estate_and_the_shares_end() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    // The account cannot pay the whole loan: Ann sends 12,000 away.
    w.set_param("finance.transfer_fee_bp", 0);
    let friend = account_holder(&mut w, "Friend", 0);
    ok(&mut w, ann, "transfer", json!({ "to": friend, "amount": 12_000 }));
    assert_eq!(balance(&w, ann), 2_875);
    let total = conserved(&w);
    let city_before = w.money(CITY);

    w.act_ok(ann, "test_die", json!({ "heir": null }));
    let frames = w.step(4);
    nothing_left(&w, ann);

    // The account had 2,875 and the 25 of the share bid: the bank took
    // all 2,900 for the loan and gave up 2,100.
    assert_eq!(burned(&w, "bank.loan_repay"), 2_900);
    let bank = w.global().res::<Bank>();
    assert_eq!((bank.outstanding_principal(), bank.written_off()), (0, 2_100));
    drop(bank);
    // The city has what the orders held. The kernel sent the cash of Ann
    // to the unclaimed account, because the test verb names no heir.
    assert_eq!(w.money(CITY) - city_before, Money(100));
    assert_eq!(w.qty(CITY, "wheat"), 5);
    assert_eq!(w.money(UNCLAIMED), Money(900));
    // Nobody holds the shares of Ann: they are cancelled.
    let shares = w.global().res::<Shares>();
    assert_eq!(shares.company(ACME).map(|c| (c.outstanding(), c.on_offer())), Some((0, 90)));
    drop(shares);
    let settled = events(&frames, "finance.estate_settled");
    assert_eq!(
        settled[0].data,
        json!({
            "resident": ann,
            "heir": { "Org": 100 },
            "balance": 0,
            "loan_unpaid": 2_100,
            "shares": 0,
        })
    );
    assert_eq!(conserved(&w), total - 2_900);
}

#[test]
fn a_resident_that_an_administrator_removes_leaves_its_holdings_unclaimed() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    let total = conserved(&w);
    w.act_ok(ann, "test_die", json!({ "heir": null, "silent": true }));
    w.step(4);
    nothing_left(&w, ann);
    // Cash 900, the balance of 9,900 after the loan, and the 100 of the
    // buy order.
    assert_eq!(w.money(UNCLAIMED), Money(900 + 9_900 + 100));
    assert_eq!(w.qty(UNCLAIMED, "wheat"), 5);
    assert_eq!(conserved(&w), total - 5_000);
    // The account fee of Ann stays with the bank.
    assert_eq!(bank_treasury(&w), 2 * OPEN_FEE);
}

#[test]
fn a_resident_without_finance_holdings_leaves_nothing_to_settle() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", 500);
    let total = conserved(&w);
    w.act_ok(ann, "test_die", json!({ "heir": null }));
    let frames = w.step(3);
    assert!(events(&frames, "finance.estate_settled").is_empty());
    assert_eq!(w.money(UNCLAIMED), Money(500));
    assert_eq!(conserved(&w), total);
}

#[test]
fn a_resident_that_names_itself_as_heir_leaves_the_estate_to_the_city() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    let total = conserved(&w);
    let city_before = w.money(CITY);
    w.act_ok(ann, "test_die", json!({ "heir": ann }));
    let frames = w.step(4);
    nothing_left(&w, ann);
    // Nobody inherits from itself: the balance of 9,900 after the loan and
    // the 100 of the buy order go to the city.
    assert_eq!(w.money(CITY) - city_before, Money(9_900 + 100));
    assert_eq!(events(&frames, "finance.estate_settled")[0].data["heir"], json!({ "Org": 100 }));
    assert_eq!(conserved(&w), total - 5_000);
}

#[test]
fn a_resident_that_is_removed_while_a_deposit_is_on_its_way_loses_nothing() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 1_000);
    w.give_money(ann, Money(700));
    let total = conserved(&w);

    // The cash and the request have left the zone when an administrator
    // removes Ann.
    w.act_ok(ann, "deposit", json!({ "amount": 200 }));
    let dropped = w.world.dropped_messages();
    assert_eq!(w.world.admin(AdminCommand::Despawn { resident: ann }), AdminReply::Ok);
    w.step(3);
    assert!(!w.is_alive(ann));
    nothing_left(&w, ann);
    // The kernel sent the 500 in cash to the unclaimed account. The bank
    // counted the deposit and then settled the estate: 1,200 more.
    assert_eq!(w.money(UNCLAIMED), Money(500 + 1_200));
    assert_eq!(w.assert_conserved().money.in_flight, 0);
    assert_eq!(conserved(&w), total);
    // Only the answer of the bank finds nobody. The module sends no notice
    // to a resident that is gone.
    assert_eq!(w.world.dropped_messages() - dropped, 1);
}

#[test]
fn an_heir_without_an_account_gets_one_with_its_first_dividend() {
    let mut w = world();
    let ann = rich_ann(&mut w);
    let bob = customer(&mut w, "Bob", 0);
    w.act_ok(ann, "test_die", json!({ "heir": bob }));
    w.step(4);
    // Bob has the 10 shares and 9,900 in cash, and no account.
    assert!(!w.global().res::<Bank>().is_open(bob));
    let total = conserved(&w);

    // ACME has the 100 that Ann paid for the shares. Bob is the only
    // holder, so the whole dividend is his.
    // Fay, the treasurer, is the first resident that `rich_ann` spawns.
    let fay = ResidentId(1);
    ok(&mut w, fay, "pay_dividend", json!({ "org": ACME, "amount": 60 }));
    assert!(w.global().res::<Bank>().is_open(bob), "the dividend opened an account");
    assert_eq!(balance(&w, bob), 60);
    assert_eq!(w.money(Holder::Org(ACME)), Money(40));
    assert_eq!(w.observe(bob).sections["bank"]["balance"], json!(60));
    // The account was free, and the deposit total of the bank counts it.
    assert_eq!(bank_treasury(&w), 2 * OPEN_FEE);
    assert_eq!(conserved(&w), total);
}
