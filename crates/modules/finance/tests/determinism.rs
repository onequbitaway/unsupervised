//! The same seed and the same inputs give the same state, also when the
//! world is restored from a snapshot on the way.

mod common;

use common::{AT_BANK, AT_EXCHANGE, SUBURB, conserved, grant, plugins};
use serde_json::{Value, json};
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Money, OrgId, ResidentId, WorldConfig};

const ACME: OrgId = OrgId(3_000_000);

/// The residents of the scenario, in spawn order. Ids start at 1.
const ANN: ResidentId = ResidentId(1);
const BOB: ResidentId = ResidentId(2);
const CID: ResidentId = ResidentId(3);
const GOV: ResidentId = ResidentId(4);

/// Sends an action and steps two frames, so that an answer of the global
/// realm is back. The test does not look at the answer: a refused action
/// is a valid input too.
fn act(w: &mut TestWorld<'_>, r: ResidentId, verb: &str, args: Value) {
    w.act(r, verb, args);
    w.step(2);
}

/// The first minutes: accounts, a loan, orders that wait, an issue of
/// shares.
fn setup(w: &mut TestWorld<'_>) {
    w.set_param("finance.loan_base_limit", 30_000);
    for (name, pos, cash) in [
        ("Ann", AT_BANK, 60_100),
        ("Bob", AT_BANK, 20_100),
        ("Cid", AT_EXCHANGE, 5_000),
        ("Gov", AT_BANK, 0),
    ] {
        let r = w.spawn_at(name, pos);
        w.give_money(r, Money(cash));
    }
    grant(w, ANN, ACME, &["treasury.spend"]);
    grant(w, GOV, OrgId::CENTRAL_BANK, &["bank.central"]);
    for r in [ANN, BOB] {
        act(w, r, "open_account", Value::Null);
    }
    act(w, ANN, "deposit", json!({ "amount": 60_000 }));
    act(w, BOB, "deposit", json!({ "amount": 20_000 }));
    act(w, BOB, "loan_request", json!({ "amount": 12_000, "term_days": 3 }));
    act(w, ANN, "issue_shares", json!({ "org": ACME, "shares": 500, "price": 20 }));
    act(w, BOB, "buy_shares", json!({ "org": ACME, "shares": 120, "price": 25 }));
    act(w, BOB, "sell_shares", json!({ "org": ACME, "shares": 20, "price": 40 }));
    act(w, GOV, "central_bank_issue", json!({ "amount": 70_000 }));
    w.give_item(CID, "wheat", 40);
    w.give_item(CID, "fish", 10);
    act(w, CID, "order_place", json!({ "item": "wheat", "side": "sell", "qty": 25, "price": 110 }));
    act(w, CID, "order_place", json!({ "item": "fish", "side": "sell", "qty": 10, "price": 45 }));
    act(w, CID, "order_place", json!({ "item": "ore", "side": "buy", "qty": 3, "price": 300 }));
}

/// The next five days: trades, a rate change, a dividend, a default and a
/// death.
fn run(w: &mut TestWorld<'_>) {
    // Bob takes his money out and walks to the exchange, so that the loan
    // defaults later.
    act(w, BOB, "withdraw", json!({ "amount": 29_000 }));
    w.set_pos(BOB, AT_EXCHANGE);
    act(w, BOB, "order_place", json!({ "item": "wheat", "side": "buy", "qty": 30, "price": 115 }));
    act(w, BOB, "order_place", json!({ "item": "fish", "side": "buy", "qty": 4, "price": 50 }));
    act(w, BOB, "market_claim", Value::Null);
    act(w, CID, "order_cancel", json!({ "order": 0 }));
    act(w, GOV, "central_bank_set_rate", json!({ "rate_bp": 900 }));
    act(w, ANN, "pay_dividend", json!({ "org": ACME, "amount": 1_001 }));
    act(w, ANN, "transfer", json!({ "to": BOB, "amount": 777 }));
    w.step(1_440);
    w.give_item(BOB, "ore", 5);
    act(w, BOB, "order_place", json!({ "item": "ore", "side": "sell", "qty": 5, "price": 250 }));
    act(w, GOV, "central_bank_retire", json!({ "amount": 9_999 }));
    w.set_pos(CID, SUBURB);
    w.step(1_440);
    act(w, ANN, "loan_request", json!({ "amount": 25_000, "term_days": 10 }));
    act(w, ANN, "loan_repay", json!({ "amount": 4_321 }));
    w.step(2 * 1_440);
    act(w, BOB, "test_die", json!({ "heir": ANN }));
    w.step(1_440);
    conserved(w);
}

#[test]
fn the_same_inputs_give_the_same_state() {
    let hash = assert_deterministic(&plugins(), &WorldConfig::with_seed(7), setup, run);
    assert_eq!(hash.len(), 64);
}

#[test]
fn a_restore_at_any_point_changes_nothing() {
    let straight = {
        let mut w = TestWorld::with_config(&plugins(), WorldConfig::with_seed(7));
        setup(&mut w);
        run(&mut w);
        w.hash()
    };

    // The same scenario, with a snapshot and a restore in the middle of
    // each phase: with orders in the books, with a loan in arrears, with
    // messages on the way.
    let mut w = TestWorld::with_config(&plugins(), WorldConfig::with_seed(7));
    setup(&mut w);
    w.snapshot_restore();
    act(&mut w, BOB, "withdraw", json!({ "amount": 29_000 }));
    w.set_pos(BOB, AT_EXCHANGE);
    // A message is on its way to the global realm when the snapshot is
    // made.
    w.act(BOB, "order_place", json!({ "item": "wheat", "side": "buy", "qty": 30, "price": 115 }));
    w.snapshot_restore();
    w.step(1);
    w.snapshot_restore();
    w.step(1);
    act(
        &mut w,
        BOB,
        "order_place",
        json!({ "item": "fish", "side": "buy", "qty": 4, "price": 50 }),
    );
    w.snapshot_restore();
    act(&mut w, BOB, "market_claim", Value::Null);
    act(&mut w, CID, "order_cancel", json!({ "order": 0 }));
    act(&mut w, GOV, "central_bank_set_rate", json!({ "rate_bp": 900 }));
    w.act(ANN, "pay_dividend", json!({ "org": ACME, "amount": 1_001 }));
    w.snapshot_restore();
    w.step(2);
    act(&mut w, ANN, "transfer", json!({ "to": BOB, "amount": 777 }));
    w.step(700);
    w.snapshot_restore();
    w.step(740);
    w.snapshot_restore();
    w.give_item(BOB, "ore", 5);
    act(
        &mut w,
        BOB,
        "order_place",
        json!({ "item": "ore", "side": "sell", "qty": 5, "price": 250 }),
    );
    act(&mut w, GOV, "central_bank_retire", json!({ "amount": 9_999 }));
    w.set_pos(CID, SUBURB);
    w.step(1_440);
    w.snapshot_restore();
    act(&mut w, ANN, "loan_request", json!({ "amount": 25_000, "term_days": 10 }));
    act(&mut w, ANN, "loan_repay", json!({ "amount": 4_321 }));
    w.step(1_440);
    w.snapshot_restore();
    w.step(1_440);
    w.act(BOB, "test_die", json!({ "heir": ANN }));
    w.snapshot_restore();
    w.step(2);
    w.snapshot_restore();
    w.step(1_440);
    conserved(&w);
    assert_eq!(w.hash(), straight);
}

#[test]
fn the_scenario_does_what_it_says() {
    // The determinism tests only compare hashes. This test proves that the
    // scenario is not empty: the loan defaults, orders trade, the estate
    // is settled.
    use unsup_finance::bank::Bank;
    use unsup_finance::shares::Shares;
    use unsup_finance::stats::Stats;
    let mut w = TestWorld::with_config(&plugins(), WorldConfig::with_seed(7));
    setup(&mut w);
    run(&mut w);
    let bank = w.global().res::<Bank>();
    // Only a default or an estate writes principal off, and the estate of
    // Bob had no loan left. At the default Bob had 2,378 and 7 of deposit
    // interest: 5 on the second midnight, and 2 of 5 on the third, when
    // the bank treasury had 69 for 132 of interest.
    assert_eq!(bank.written_off(), 12_000 - 2_385);
    assert!(bank.account(ANN).is_some_and(|a| a.loan.is_some()));
    assert!(!bank.is_open(BOB), "the account of Bob is closed");
    let shares = w.global().res::<Shares>();
    let acme = shares.company(ACME).expect("ACME has shares");
    assert_eq!((acme.outstanding(), acme.holding(ANN).held), (120, 120));
    assert_eq!(shares.orders_of(BOB).count(), 0);
    let stats = w.global().res::<Stats>();
    let latest = stats.latest().expect("five days were published");
    assert_eq!(latest.day, 5);
    assert_eq!(latest.prices.get("wheat").map(|p| p.last), Some(110));
    assert_eq!(latest.faucets["central_bank.issue"].total, 70_000);
    assert_eq!(latest.sinks["central_bank.retire"].total, 9_999);
    assert_eq!(latest.loans.written_off, 12_000 - 2_385);
    assert!(!w.is_alive(BOB));
    assert_eq!(w.param("finance.base_rate_bp"), 900);
    // Ann inherited the wheat that Bob bought at the exchange.
    assert_eq!(w.qty(unsup_kernel::Holder::Resident(ANN), "wheat"), 25);
}
