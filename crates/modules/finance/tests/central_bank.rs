//! The central bank: the base rate, issuing and retiring money, the
//! permission and the yearly limits.

mod common;

use common::{
    account_holder, burned, conserved, customer, err, events, fund_global, grant, minted, next_day,
    ok, world,
};
use serde_json::json;
use unsup_finance::central::CentralBank;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId, ResidentId};

const NATION: Holder = Holder::Org(OrgId::NATION);

/// Spawns a resident with the permission `bank.central` in the central
/// bank organization.
fn banker(w: &mut TestWorld<'_>) -> ResidentId {
    let r = customer(w, "Governor", 0);
    grant(w, r, OrgId::CENTRAL_BANK, &["bank.central"]);
    r
}

#[test]
fn the_central_banker_sets_the_base_rate() {
    let mut w = world();
    let gov = banker(&mut w);
    assert_eq!(w.param("finance.base_rate_bp"), 500);

    w.act_ok(gov, "central_bank_set_rate", json!({ "rate_bp": 700 }));
    let frames = w.step(2);
    let report = w.last_report(gov).expect("the verb reports");
    assert_eq!(report.result, json!({ "rate_bp": 700, "old_bp": 500 }));
    assert_eq!(w.param("finance.base_rate_bp"), 700);
    let changed = events(&frames, "finance.rate_changed");
    assert_eq!(changed.len(), 1);
    assert_eq!(changed[0].data, json!({ "resident": gov, "old_bp": 500, "rate_bp": 700 }));

    // A new loan costs the base rate plus the spread of 300.
    w.set_param("finance.loan_base_limit", 10_000);
    let ann = account_holder(&mut w, "Ann", 50_000);
    let loan = ok(&mut w, ann, "loan_request", json!({ "amount": 5_000, "term_days": 5 }));
    assert_eq!(loan["rate_bp"], json!(1_000));

    // The law sets the band. Its ends are allowed, the outside is not.
    w.set_param("finance.base_rate_floor_bp", 200);
    w.set_param("finance.base_rate_ceiling_bp", 900);
    for rate in [199, 901, -1] {
        let error = err(&mut w, gov, "central_bank_set_rate", json!({ "rate_bp": rate }));
        assert_eq!(error.code, ErrorCode::NotAllowed, "rate {rate}");
        assert_eq!(error.details, json!({ "min": 200, "max": 900 }));
    }
    assert_eq!(w.param("finance.base_rate_bp"), 700);
    ok(&mut w, gov, "central_bank_set_rate", json!({ "rate_bp": 200 }));
    ok(&mut w, gov, "central_bank_set_rate", json!({ "rate_bp": 900 }));
    assert_eq!(w.param("finance.base_rate_bp"), 900);
    // The loan that exists keeps its rate.
    assert_eq!(w.observe(ann).sections["bank"]["loan"]["rate_bp"], json!(1_000));
    conserved(&w);
}

#[test]
fn only_a_central_banker_can_use_the_verbs() {
    let mut w = world();
    let bob = customer(&mut w, "Bob", 0);
    fund_global(&mut w, NATION, 1_000);
    let all = [
        ("central_bank_set_rate", json!({ "rate_bp": 100 })),
        ("central_bank_issue", json!({ "amount": 100 })),
        ("central_bank_retire", json!({ "amount": 100 })),
    ];
    for (verb, args) in &all {
        assert_eq!(err(&mut w, bob, verb, args.clone()).code, ErrorCode::NotAllowed, "{verb}");
    }
    // The permission in a private organization gives nothing, and another
    // permission in the central bank gives nothing.
    grant(&mut w, bob, OrgId(3_000_000), &["bank.central"]);
    grant(&mut w, bob, OrgId::CENTRAL_BANK, &["treasury.spend"]);
    for (verb, args) in &all {
        assert_eq!(err(&mut w, bob, verb, args.clone()).code, ErrorCode::NotAllowed, "{verb}");
    }
    assert_eq!(w.param("finance.base_rate_bp"), 500);
    assert_eq!(w.money(NATION), Money(1_000));
    assert_eq!(minted(&w, "central_bank.issue") + burned(&w, "central_bank.retire"), 0);

    // The permission in the nation works too.
    grant(&mut w, bob, OrgId::NATION, &["bank.central"]);
    ok(&mut w, bob, "central_bank_issue", json!({ "amount": 100 }));
    assert_eq!(w.money(NATION), Money(1_100));
    conserved(&w);
}

#[test]
fn issue_creates_treasury_money_up_to_the_yearly_limit() {
    let mut w = world();
    let gov = banker(&mut w);
    w.set_param("finance.issue_limit_per_year", 1_000_000);
    let start = conserved(&w);

    w.act_ok(gov, "central_bank_issue", json!({ "amount": 600_000 }));
    let frames = w.step(2);
    let report = w.last_report(gov).expect("the verb reports");
    assert_eq!(
        report.result,
        json!({ "issued": 600_000, "issued_this_year": 600_000, "treasury": 600_000 })
    );
    let issued = events(&frames, "finance.money_issued");
    assert_eq!(issued.len(), 1);
    assert_eq!(issued[0].data, json!({ "resident": gov, "amount": 600_000 }));
    assert_eq!(w.money(NATION), Money(600_000));
    assert_eq!(minted(&w, "central_bank.issue"), 600_000);
    assert_eq!(conserved(&w), start + 600_000);

    // 400,000 are left this year: one cent more is refused, the exact
    // rest is allowed.
    let error = err(&mut w, gov, "central_bank_issue", json!({ "amount": 400_001 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.details, json!({ "limit": 1_000_000, "used": 600_000, "left": 400_000 }));
    ok(&mut w, gov, "central_bank_issue", json!({ "amount": 400_000 }));
    assert_eq!(
        err(&mut w, gov, "central_bank_issue", json!({ "amount": 1 })).code,
        ErrorCode::NotAllowed
    );
    assert_eq!(
        err(&mut w, gov, "central_bank_issue", json!({ "amount": 0 })).code,
        ErrorCode::InvalidArgs
    );
    assert_eq!(minted(&w, "central_bank.issue"), 1_000_000);
    assert_eq!(w.global().res::<CentralBank>().issued_in(1), 1_000_000);

    // A new game year starts a new limit.
    for _ in 0..28 {
        next_day(&mut w);
    }
    assert_eq!(w.world.realm(unsup_kernel::RealmId::Global).map(|r| r.clock().year), Some(2));
    ok(&mut w, gov, "central_bank_issue", json!({ "amount": 250_000 }));
    assert_eq!(minted(&w, "central_bank.issue"), 1_250_000);
    assert_eq!(w.global().res::<CentralBank>().issued_in(2), 250_000);
    assert_eq!(conserved(&w), start + 1_250_000);
}

#[test]
fn retire_destroys_treasury_money() {
    let mut w = world();
    let gov = banker(&mut w);
    fund_global(&mut w, NATION, 5_000);
    w.set_param("finance.retire_limit_per_year", 4_000);
    let start = conserved(&w);

    w.act_ok(gov, "central_bank_retire", json!({ "amount": 1_500 }));
    let frames = w.step(2);
    let report = w.last_report(gov).expect("the verb reports");
    assert_eq!(
        report.result,
        json!({ "retired": 1_500, "retired_this_year": 1_500, "treasury": 3_500 })
    );
    assert_eq!(events(&frames, "finance.money_retired").len(), 1);
    assert_eq!(burned(&w, "central_bank.retire"), 1_500);
    assert_eq!(conserved(&w), start - 1_500);

    let error = err(&mut w, gov, "central_bank_retire", json!({ "amount": 2_501 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.details, json!({ "limit": 4_000, "used": 1_500, "left": 2_500 }));
    assert_eq!(
        err(&mut w, gov, "central_bank_retire", json!({ "amount": 0 })).code,
        ErrorCode::InvalidArgs
    );

    // The exact rest of the limit is allowed. Then nothing more is.
    ok(&mut w, gov, "central_bank_retire", json!({ "amount": 2_500 }));
    assert_eq!(w.global().res::<CentralBank>().retired_in(1), 4_000);
    assert_eq!(
        err(&mut w, gov, "central_bank_retire", json!({ "amount": 1 })).code,
        ErrorCode::NotAllowed
    );

    // The treasury must have the money.
    w.set_param("finance.retire_limit_per_year", 1_000_000);
    let error = err(&mut w, gov, "central_bank_retire", json!({ "amount": 1_001 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_001, "available": 1_000 }));
    ok(&mut w, gov, "central_bank_retire", json!({ "amount": 1_000 }));
    assert_eq!(w.money(NATION), Money(0));
    assert_eq!(burned(&w, "central_bank.retire"), 5_000);
    assert_eq!(conserved(&w), start - 5_000);
}

#[test]
fn issue_refuses_an_amount_that_the_treasury_cannot_hold() {
    let mut w = world();
    let gov = banker(&mut w);
    // A treasury at the top of the range of the ledger. No game rule can
    // make one; the test writes the balance directly.
    fund_global(&mut w, NATION, unsup_kernel::SUPPLY_CAP - 500);

    let error = err(&mut w, gov, "central_bank_issue", json!({ "amount": 501 }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
    // Nothing was created, and nothing counts against the yearly limit.
    assert_eq!(minted(&w, "central_bank.issue"), 0);
    assert_eq!(w.global().res::<CentralBank>().issued_in(1), 0);
    assert_eq!(w.money(NATION), Money(unsup_kernel::SUPPLY_CAP - 500));

    // An amount that fits is issued.
    ok(&mut w, gov, "central_bank_issue", json!({ "amount": 500 }));
    assert_eq!(minted(&w, "central_bank.issue"), 500);
    assert_eq!(w.global().res::<CentralBank>().issued_in(1), 500);
    w.assert_conserved();
}
