//! Bank accounts: `open_account`, `deposit`, `withdraw` and `transfer`.

mod common;

use common::{
    AT_BANK, OPEN_FEE, STREET, SUBURB, account_holder, balance, bank_treasury, conserved, customer,
    err, events, ok, world,
};
use serde_json::{Value, json};
use unsup_finance::bank::Bank;
use unsup_kernel::{ErrorCode, Money};

#[test]
fn open_account_takes_the_fee_in_cash() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", 250);
    let result = ok(&mut w, ann, "open_account", Value::Null);
    assert_eq!(result, json!({ "account": ann, "fee": OPEN_FEE, "balance": 0 }));
    assert_eq!(w.cash(ann), Money(150));
    assert_eq!(bank_treasury(&w), OPEN_FEE);
    assert!(w.global().res::<Bank>().is_open(ann));
    assert_eq!(conserved(&w), 250);
}

#[test]
fn open_account_refusals() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", 250);

    w.set_pos(ann, STREET);
    assert_eq!(err(&mut w, ann, "open_account", Value::Null).code, ErrorCode::NotHere);

    w.set_pos(ann, AT_BANK);
    let poor = customer(&mut w, "Poor", OPEN_FEE - 1);
    let error = err(&mut w, poor, "open_account", Value::Null);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": OPEN_FEE, "available": OPEN_FEE - 1 }));
    assert_eq!(w.cash(poor), Money(OPEN_FEE - 1));
    assert!(!w.global().res::<Bank>().is_open(poor));

    // The second request is refused by the global realm, and the fee comes
    // back.
    ok(&mut w, ann, "open_account", Value::Null);
    assert_eq!(err(&mut w, ann, "open_account", Value::Null).code, ErrorCode::Conflict);
    w.step(2);
    assert_eq!(w.cash(ann), Money(150));
    assert_eq!(bank_treasury(&w), OPEN_FEE);
    assert_eq!(conserved(&w), 250 + OPEN_FEE - 1);
}

#[test]
fn deposit_and_withdraw_move_cash_between_realms() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", OPEN_FEE + 1_000);
    ok(&mut w, ann, "open_account", Value::Null);
    let total = conserved(&w);
    assert_eq!(total, OPEN_FEE + 1_000);

    // The cash leaves the zone at once and is in flight until the global
    // realm has it.
    let ack = w.act_ok(ann, "deposit", json!({ "amount": 700 }));
    assert_eq!(ack, unsup_kernel::protocol::ActionStatus::Pending);
    assert_eq!(w.cash(ann), Money(300));
    let in_flight = w.assert_conserved().money.in_flight;
    assert_eq!(in_flight, 700);
    assert_eq!(conserved(&w), total);
    w.step(1);
    assert_eq!(balance(&w, ann), 700);
    assert_eq!(conserved(&w), total);
    w.step(1);
    let report = w.last_report(ann).expect("the deposit reports");
    assert_eq!(report.result, json!({ "deposited": 700, "balance": 700 }));
    assert_eq!(w.global().res::<Bank>().deposits(), 700);

    let result = ok(&mut w, ann, "withdraw", json!({ "amount": 250 }));
    assert_eq!(result, json!({ "withdrawn": 250, "fee": 0, "balance": 450 }));
    assert_eq!(w.cash(ann), Money(550));
    assert_eq!(balance(&w, ann), 450);
    assert_eq!(conserved(&w), total);
}

#[test]
fn withdraw_charges_the_fee_parameter() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 10_000);
    w.set_param("finance.withdraw_fee_bp", 150);
    let result = ok(&mut w, ann, "withdraw", json!({ "amount": 4_000 }));
    // 1.5 percent of 4,000 is 60.
    assert_eq!(result, json!({ "withdrawn": 4_000, "fee": 60, "balance": 5_940 }));
    assert_eq!(w.cash(ann), Money(4_000));
    assert_eq!(bank_treasury(&w), OPEN_FEE + 60);
    // The fee counts against the balance.
    let error = err(&mut w, ann, "withdraw", json!({ "amount": 5_900 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 5_988, "available": 5_940 }));
    assert_eq!(balance(&w, ann), 5_940);
    assert_eq!(conserved(&w), OPEN_FEE + 10_000);
}

#[test]
fn deposit_refusals() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", 500);
    assert_eq!(err(&mut w, ann, "deposit", json!({ "amount": 0 })).code, ErrorCode::InvalidArgs);
    assert_eq!(err(&mut w, ann, "deposit", json!({ "amount": -5 })).code, ErrorCode::InvalidArgs);
    assert_eq!(err(&mut w, ann, "deposit", json!({ "amount": 501 })).code, ErrorCode::Insufficient);
    assert_eq!(err(&mut w, ann, "deposit", json!({})).code, ErrorCode::InvalidArgs);

    // Without an account the global realm refuses, and the cash comes back.
    let error = err(&mut w, ann, "deposit", json!({ "amount": 200 }));
    assert_eq!(error.code, ErrorCode::NotFound);
    w.step(2);
    assert_eq!(w.cash(ann), Money(500));
    assert_eq!(balance(&w, ann), 0);
    assert_eq!(w.global().res::<Bank>().deposits(), 0);

    w.set_pos(ann, SUBURB);
    assert_eq!(err(&mut w, ann, "deposit", json!({ "amount": 200 })).code, ErrorCode::NotHere);
    assert_eq!(w.cash(ann), Money(500));
    assert_eq!(conserved(&w), 500);
}

#[test]
fn withdraw_refusals() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", 500);
    assert_eq!(err(&mut w, ann, "withdraw", json!({ "amount": 0 })).code, ErrorCode::InvalidArgs);
    assert_eq!(err(&mut w, ann, "withdraw", json!({ "amount": 10 })).code, ErrorCode::NotFound);

    let bob = account_holder(&mut w, "Bob", 300);
    let error = err(&mut w, bob, "withdraw", json!({ "amount": 301 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 301, "available": 300 }));
    w.set_pos(bob, STREET);
    assert_eq!(err(&mut w, bob, "withdraw", json!({ "amount": 10 })).code, ErrorCode::NotHere);
    assert_eq!(balance(&w, bob), 300);
    assert_eq!(w.cash(bob), Money(0));
    assert_eq!(conserved(&w), 500 + OPEN_FEE + 300);
}

#[test]
fn transfer_moves_money_between_accounts_from_anywhere() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 50_000);
    let bob = account_holder(&mut w, "Bob", 0);
    w.set_pos(ann, SUBURB);
    let total = conserved(&w);

    // The fee is 10 basis points of 20,000: 20.
    let result = ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 20_000 }));
    assert_eq!(result, json!({ "sent": 20_000, "fee": 20, "balance": 29_980 }));
    assert_eq!(balance(&w, ann), 29_980);
    assert_eq!(balance(&w, bob), 20_000);
    assert_eq!(bank_treasury(&w), 2 * OPEN_FEE + 20);
    assert_eq!(conserved(&w), total);

    // The receiver gets a notice.
    let notice = w
        .notices(bob)
        .into_iter()
        .find(|notice| notice.kind == "finance.transfer_received")
        .expect("Bob hears about the transfer");
    assert_eq!(notice.from, Some(ann));
    assert_eq!(notice.data, json!({ "from": ann, "amount": 20_000 }));

    // `transfer` is free: it does not occupy the resident.
    w.act_ok(ann, "wait", json!({ "ticks": 30 }));
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 80 }));
    assert_eq!(w.activity(ann), Some("wait"));
    assert_eq!(balance(&w, bob), 20_080);
}

#[test]
fn transfer_refusals() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 1_000);
    let bob = account_holder(&mut w, "Bob", 0);
    let cat = customer(&mut w, "Cat", 0);

    let code = |w: &mut _, from, to, amount| {
        err(w, from, "transfer", json!({ "to": to, "amount": amount })).code
    };
    assert_eq!(code(&mut w, ann, bob, 0), ErrorCode::InvalidArgs);
    assert_eq!(code(&mut w, ann, ann, 10), ErrorCode::InvalidArgs);
    assert_eq!(code(&mut w, cat, ann, 10), ErrorCode::NotFound);
    assert_eq!(code(&mut w, ann, cat, 10), ErrorCode::NotFound);
    // 1,000 and the fee of 1 are more than the balance.
    let error = err(&mut w, ann, "transfer", json!({ "to": bob, "amount": 1_000 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_001, "available": 1_000 }));
    assert_eq!((balance(&w, ann), balance(&w, bob)), (1_000, 0));
    assert_eq!(conserved(&w), 2 * OPEN_FEE + 1_000);
}

#[test]
fn the_bank_section_shows_the_balance() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", OPEN_FEE + 900);
    assert!(!w.observe(ann).sections.contains_key("bank"), "no account, no section");
    ok(&mut w, ann, "open_account", Value::Null);
    ok(&mut w, ann, "deposit", json!({ "amount": 900 }));
    assert_eq!(w.observe(ann).sections["bank"], json!({ "balance": 900 }));
    conserved(&w);
}

#[test]
fn a_resident_in_hospital_can_use_its_account_but_not_the_counter() {
    let mut w = world();
    let ann = account_holder(&mut w, "Ann", 5_000);
    let bob = account_holder(&mut w, "Bob", 0);
    w.give_money(ann, Money(300));
    w.set_condition(ann, "hospitalized", None);
    // The counter verbs need the resident on its feet.
    for (verb, args) in [
        ("deposit", json!({ "amount": 100 })),
        ("withdraw", json!({ "amount": 100 })),
        ("open_account", Value::Null),
        ("loan_request", json!({ "amount": 1_000, "term_days": 2 })),
    ] {
        assert_eq!(err(&mut w, ann, verb, args).code, ErrorCode::Blocked, "{verb}");
    }
    // The verbs that only use the account work from the bed.
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 1_000 }));
    assert_eq!(balance(&w, bob), 1_000);
    assert_eq!(
        err(&mut w, ann, "loan_repay", json!({ "amount": 10 })).code,
        ErrorCode::NotFound,
        "the verb runs: Ann has no loan"
    );
    conserved(&w);
}

#[test]
fn a_new_account_is_a_public_event() {
    let mut w = world();
    let ann = customer(&mut w, "Ann", OPEN_FEE);
    w.act_ok(ann, "open_account", Value::Null);
    let opened = events(&w.step(2), "finance.account_opened");
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].data, json!({ "resident": ann }));
    assert_eq!(opened[0].actor, Some(ann));
    assert!(opened[0].public);
    // A request that the bank refuses is not an event.
    let ben = customer(&mut w, "Ben", 2 * OPEN_FEE);
    ok(&mut w, ben, "open_account", Value::Null);
    w.act_ok(ben, "open_account", Value::Null);
    assert!(events(&w.step(2), "finance.account_opened").is_empty());
    conserved(&w);
}
