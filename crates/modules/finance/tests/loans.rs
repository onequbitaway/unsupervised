//! Loans: the request, the daily instalments, early repayment and default.

mod common;

use common::{
    OPEN_FEE, account_holder, balance, bank_treasury, burned, conserved, customer, err, events,
    minted, next_day, next_day_frames, notice, ok, world,
};
use serde_json::{Value, json};
use unsup_finance::bank::Bank;
use unsup_kernel::protocol::{Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, ResidentId};

/// Sets the rates so that a loan costs exactly 1 percent per day (2,800
/// basis points in a year of 28 days) and deposits earn nothing, and gives
/// every resident a credit limit of 20,000 without history.
///
/// The loans of these tests are unsecured, as a server can allow them:
/// the bank holds no collateral. The data file has full collateral; the
/// tests at the end of this file cover it.
fn simple_rates(w: &mut TestWorld<'_>) {
    w.set_param("finance.base_rate_bp", 2_500);
    w.set_param("finance.loan_spread_bp", 300);
    w.set_param("finance.deposit_spread_bp", 10_000);
    w.set_param("finance.loan_base_limit", 20_000);
    w.set_param("finance.loan_collateral_bp", 0);
}

fn full(w: &TestWorld<'_>, r: ResidentId) -> Value {
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    w.observe_with(r, &opts).sections["bank"].clone()
}

#[test]
fn a_loan_is_created_by_the_faucet_and_repaid_into_the_sink() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 100_000);
    let start = conserved(&w);
    assert_eq!(start, OPEN_FEE + 100_000);

    w.act_ok(ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    let frames = w.step(2);
    let report = w.last_report(ann).expect("the loan request reports");
    assert_eq!(
        report.result,
        json!({
            "amount": 10_000,
            "rate_bp": 2_800,
            "term_days": 4,
            "instalment": 2_500,
            // The first instalment is due at the second midnight.
            "first_due_in_ticks": 2 * 1_440 - (report.tick.0 - 1),
            "balance": 110_000,
            // The loans of this world are unsecured.
            "held": 0,
        })
    );
    let granted = events(&frames, "finance.loan_granted");
    assert_eq!(granted.len(), 1);
    assert_eq!(
        granted[0].data,
        json!({ "resident": ann, "amount": 10_000, "rate_bp": 2_800, "term_days": 4 })
    );
    assert_eq!(minted(&w, "bank.loan_issue"), 10_000);
    assert_eq!(conserved(&w), start + 10_000);
    assert_eq!(
        w.observe(ann).sections["bank"]["loan"],
        json!({
            "principal": 10_000,
            "rate_bp": 2_800,
            "owed": 10_000,
            "next_instalment": 2_600,
            "next_in_ticks": 2 * 1_440 - w.world.frame(),
            "days_left": 4,
            "missed": 0,
            "held": 0,
        })
    );

    // The first midnight takes nothing.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 110_000);

    // Each later midnight takes 2,500 of principal and 1 percent of what
    // is left: 100, 75, 50, 25.
    let mut expected = 110_000;
    for (day, interest) in [100, 75, 50, 25].into_iter().enumerate() {
        let frames = next_day_frames(&mut w);
        expected -= 2_500 + interest;
        assert_eq!(balance(&w, ann), expected, "after instalment {}", day + 1);
        assert_eq!(burned(&w, "bank.loan_repay"), 2_500 * (day as i64 + 1));
        assert_eq!(events(&frames, "finance.loan_repaid").len(), usize::from(day == 3));
        conserved(&w);
    }
    assert_eq!(balance(&w, ann), 99_750);
    assert_eq!(bank_treasury(&w), OPEN_FEE + 250, "the interest goes to the bank treasury");
    assert_eq!(minted(&w, "bank.loan_issue"), 10_000);
    assert_eq!(burned(&w, "bank.loan_repay"), 10_000);
    assert_eq!(conserved(&w), start, "the money that the loan created is gone again");

    let bank = w.global().res::<Bank>();
    let account = bank.account(ann).expect("Ann has an account");
    assert_eq!((account.loan.clone(), account.loans_repaid, account.defaults), (None, 1, 0));
    assert_eq!((bank.outstanding_principal(), bank.loan_count()), (0, 0));
    drop(bank);
    assert!(w.observe(ann).sections["bank"].get("loan").is_none());

    // Nothing more is taken later.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 99_750);
}

#[test]
fn interest_below_one_cent_is_carried_to_the_next_day() {
    let mut w = world();
    // The rates of the data file: 500 + 300 basis points a year, so one
    // day costs 800 / 28 basis points. Deposits earn nothing here.
    w.set_param("finance.deposit_spread_bp", 10_000);
    w.set_param("finance.loan_base_limit", 20_000);
    let ann = account_holder(&mut w, "Ann", 100_000);
    ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    next_day(&mut w);
    // Exact interest: 28.57, 21.43, 14.29 and 7.14. The parts below one
    // cent are carried: 28, then 22 (0.57 + 0.43 make a cent), 14 and 7.
    let mut expected = 110_000;
    for interest in [28, 22, 14, 7] {
        next_day(&mut w);
        expected -= 2_500 + interest;
        assert_eq!(balance(&w, ann), expected, "after the instalment with {interest} of interest");
    }
    assert_eq!(bank_treasury(&w), OPEN_FEE + 71);
    assert_eq!(burned(&w, "bank.loan_repay"), 10_000);
    conserved(&w);
}

#[test]
fn loan_repay_pays_early_and_shortens_the_loan() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 100_000);
    let start = conserved(&w);
    ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));

    let result = ok(&mut w, ann, "loan_repay", json!({ "amount": 4_000 }));
    assert_eq!(
        result,
        json!({
            "paid": 4_000,
            "interest": 0,
            "principal": 4_000,
            "remaining": 6_000,
            "closed": false,
            "balance": 106_000,
        })
    );
    assert_eq!(burned(&w, "bank.loan_repay"), 4_000);
    assert_eq!(w.global().res::<Bank>().outstanding_principal(), 6_000);

    next_day(&mut w);
    // 2,500 and 1 percent of 6,000.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 106_000 - 2_560);
    // 2,500 and 1 percent of 3,500.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 106_000 - 2_560 - 2_535);

    // An amount above what is owed pays the whole loan and no more.
    let result = ok(&mut w, ann, "loan_repay", json!({ "amount": 1_000_000 }));
    assert_eq!(result["paid"], json!(1_000));
    assert_eq!(result["closed"], json!(true));
    assert_eq!(burned(&w, "bank.loan_repay"), 10_000);
    assert_eq!(bank_treasury(&w), OPEN_FEE + 95);
    assert_eq!(conserved(&w), start);
    assert_eq!(w.global().res::<Bank>().account(ann).map(|a| a.loans_repaid), Some(1));

    next_day(&mut w);
    assert_eq!(balance(&w, ann), 100_000 - 95, "a closed loan takes nothing");
}

#[test]
fn missed_instalments_lead_to_default() {
    let mut w = world();
    simple_rates(&mut w);
    let bob = account_holder(&mut w, "Bob", 100_000);
    let cat = account_holder(&mut w, "Cat", 0);
    let start = conserved(&w);
    ok(&mut w, cat, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    // Cat moves the money away and keeps 1,000. The fee is 9.
    w.set_param("finance.transfer_fee_bp", 0);
    ok(&mut w, cat, "transfer", json!({ "to": bob, "amount": 9_000 }));
    assert_eq!(balance(&w, cat), 1_000);

    next_day(&mut w);
    assert_eq!(notice(&w, cat, "finance.instalment_missed"), None);

    // First instalment: 2,500 and 100 of interest are due, 1,000 is there.
    next_day(&mut w);
    assert_eq!(
        notice(&w, cat, "finance.instalment_missed"),
        Some(json!({ "due": 2_600, "balance": 1_000, "held": 0, "missed": 1, "default_after": 3 }))
    );
    assert_eq!(balance(&w, cat), 1_000, "a missed instalment takes nothing");
    assert_eq!(w.observe(cat).sections["bank"]["loan"]["missed"], json!(1));
    // What is not paid stays due: the next collection wants two
    // instalments and two days of interest.
    assert_eq!(w.observe(cat).sections["bank"]["loan"]["next_instalment"], json!(5_200));

    next_day(&mut w);
    assert_eq!(
        notice(&w, cat, "finance.instalment_missed"),
        Some(json!({ "due": 5_200, "balance": 1_000, "held": 0, "missed": 2, "default_after": 3 }))
    );

    // The third miss is the default. The bank seizes the 1,000.
    let frames = next_day_frames(&mut w);
    let defaulted = events(&frames, "finance.loan_defaulted");
    assert_eq!(defaulted.len(), 1);
    assert_eq!(defaulted[0].data, json!({ "resident": cat, "outstanding": 9_300 }));
    assert_eq!(defaulted[0].actor, Some(cat));
    assert!(defaulted[0].public);
    assert_eq!(
        notice(&w, cat, "finance.loan_defaulted"),
        Some(json!({
            "outstanding": 9_300,
            "seized": 1_000,
            "bankrupt_until": { "year": 2, "day": 5 },
        }))
    );
    assert_eq!(balance(&w, cat), 0);
    assert_eq!(minted(&w, "bank.loan_issue"), 10_000);
    assert_eq!(burned(&w, "bank.loan_repay"), 1_000);
    assert_eq!(conserved(&w), start + 9_000, "the written-off principal stays in the world");
    {
        let bank = w.global().res::<Bank>();
        let account = bank.account(cat).expect("Cat has an account");
        assert_eq!((account.loan.clone(), account.loans_repaid, account.defaults), (None, 0, 1));
        // The default was on day 4 and the ban lasts 28 days.
        assert_eq!(account.bankrupt_until, 32);
        assert_eq!((bank.outstanding_principal(), bank.written_off()), (0, 9_000));
    }
    assert_eq!(
        w.observe(cat).sections["bank"],
        json!({ "balance": 0, "bankrupt_until": { "year": 2, "day": 5 } })
    );

    // A bankrupt resident cannot borrow.
    let error = err(&mut w, cat, "loan_request", json!({ "amount": 1_000, "term_days": 2 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.details, json!({ "until": { "year": 2, "day": 5 } }));

    // After the ban it can, with half the limit: 20,000 * 50 percent.
    for _ in 4..32 {
        next_day(&mut w);
    }
    assert_eq!(full(&w, cat)["credit"]["limit"], json!(10_000));
    let error = err(&mut w, cat, "loan_request", json!({ "amount": 10_001, "term_days": 2 }));
    assert_eq!((error.code, error.details), (ErrorCode::NotAllowed, json!({ "limit": 10_000 })));
    ok(&mut w, cat, "loan_request", json!({ "amount": 10_000, "term_days": 2 }));
    conserved(&w);
}

#[test]
fn a_late_payment_clears_the_missed_count() {
    let mut w = world();
    simple_rates(&mut w);
    w.set_param("finance.transfer_fee_bp", 0);
    let bob = account_holder(&mut w, "Bob", 100_000);
    let cat = account_holder(&mut w, "Cat", 0);
    ok(&mut w, cat, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    ok(&mut w, cat, "transfer", json!({ "to": bob, "amount": 10_000 }));
    next_day(&mut w);
    next_day(&mut w);
    next_day(&mut w);
    assert_eq!(w.observe(cat).sections["bank"]["loan"]["missed"], json!(2));

    // Bob sends the money back in time. The next collection takes three
    // instalments and three days of interest on the whole principal.
    ok(&mut w, bob, "transfer", json!({ "to": cat, "amount": 10_000 }));
    next_day(&mut w);
    assert_eq!(balance(&w, cat), 10_000 - 7_500 - 300);
    let loan = w.observe(cat).sections["bank"]["loan"].clone();
    assert_eq!((loan["missed"].clone(), loan["principal"].clone()), (json!(0), json!(2_500)));
    assert_eq!(w.global().res::<Bank>().account(cat).map(|a| a.defaults), Some(0));

    // The last day takes the rest of the principal and its interest; Cat
    // is short of the 25 of interest only.
    next_day(&mut w);
    assert_eq!(w.observe(cat).sections["bank"]["loan"]["missed"], json!(1));
    // `loan_repay` pays interest first.
    let error = err(&mut w, cat, "loan_repay", json!({ "amount": 2_525 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 2_525, "available": 2_200 }));
    let result = ok(&mut w, cat, "loan_repay", json!({ "amount": 2_200 }));
    assert_eq!(result["interest"], json!(25));
    assert_eq!(result["principal"], json!(2_175));
    assert_eq!(result["remaining"], json!(325));
    conserved(&w);
}

#[test]
fn loan_request_refusals() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 30_000);
    let dan = customer(&mut w, "Dan", 0);
    let ask = |w: &mut _, r, amount: i64, term: i64| {
        err(w, r, "loan_request", json!({ "amount": amount, "term_days": term }))
    };

    assert_eq!(ask(&mut w, dan, 5_000, 4).code, ErrorCode::NotFound);
    assert_eq!(ask(&mut w, ann, 0, 4).code, ErrorCode::InvalidArgs);
    assert_eq!(ask(&mut w, ann, 999, 4).code, ErrorCode::InvalidArgs, "below `loan_min`");
    assert_eq!(ask(&mut w, ann, 5_000, 0).code, ErrorCode::InvalidArgs);
    assert_eq!(ask(&mut w, ann, 5_000, 29).code, ErrorCode::InvalidArgs);

    let error = ask(&mut w, ann, 20_001, 4);
    assert_eq!((error.code, error.details), (ErrorCode::NotAllowed, json!({ "limit": 20_000 })));

    // The bank keeps 10 percent of the 30,000 on deposit, and has lent
    // nothing: it can lend 27,000. With a reserve of 40 percent it can
    // lend 18,000.
    w.set_param("finance.reserve_ratio_bp", 4_000);
    let error = ask(&mut w, ann, 18_001, 4);
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 18_001, "available": 18_000 }));

    assert_eq!(minted(&w, "bank.loan_issue"), 0, "a refused request creates no money");
    ok(&mut w, ann, "loan_request", json!({ "amount": 18_000, "term_days": 4 }));
    assert_eq!(ask(&mut w, ann, 1_000, 4).code, ErrorCode::Conflict, "one loan at a time");
    assert_eq!(minted(&w, "bank.loan_issue"), 18_000);
    // The reserve limit counts what is already lent: 40 percent of the
    // 48,000 now on deposit are kept, 28,800 can be lent, 18,000 are.
    let eve = account_holder(&mut w, "Eve", 0);
    let error = ask(&mut w, eve, 10_801, 4);
    assert_eq!(error.details, json!({ "needed": 10_801, "available": 10_800 }));
    conserved(&w);
}

#[test]
fn loan_repay_refusals() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 30_000);
    let dan = customer(&mut w, "Dan", 0);
    assert_eq!(err(&mut w, dan, "loan_repay", json!({ "amount": 5 })).code, ErrorCode::NotFound);
    assert_eq!(err(&mut w, ann, "loan_repay", json!({ "amount": 5 })).code, ErrorCode::NotFound);
    ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    assert_eq!(err(&mut w, ann, "loan_repay", json!({ "amount": 0 })).code, ErrorCode::InvalidArgs);
    w.set_param("finance.transfer_fee_bp", 0);
    let bob = account_holder(&mut w, "Bob", 0);
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 39_000 }));
    let error = err(&mut w, ann, "loan_repay", json!({ "amount": 5_000 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 5_000, "available": 1_000 }));
    assert_eq!(burned(&w, "bank.loan_repay"), 0);
    conserved(&w);
}

#[test]
fn the_credit_limit_follows_history_and_seasoned_deposits() {
    let mut w = world();
    w.set_param("finance.deposit_spread_bp", 10_000);
    // The data file gives credit for deposits only. This world also gives
    // credit for the age of an account and for repaid loans.
    w.set_param("finance.loan_limit_per_account_day", 500);
    w.set_param("finance.loan_limit_per_repaid_loan", 20_000);
    let ann = account_holder(&mut w, "Ann", 40_000);
    // Day 0: no history, and the deposit is not seasoned.
    assert_eq!(
        full(&w, ann)["credit"],
        json!({ "limit": 0, "seasoned_balance": 0, "loans_repaid": 0, "defaults": 0 })
    );
    let error = err(&mut w, ann, "loan_request", json!({ "amount": 1_000, "term_days": 2 }));
    assert_eq!((error.code, error.details), (ErrorCode::NotAllowed, json!({ "limit": 0 })));

    // Day 1: one day of account age. The lowest balance of day 0 was 0.
    next_day(&mut w);
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(500));
    // Day 2: two days of age, and the 40,000 stayed for the whole of day 1.
    next_day(&mut w);
    assert_eq!(full(&w, ann)["credit"]["seasoned_balance"], json!(40_000));
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(2 * 500 + 40_000));

    // Money that leaves today lowers the seasoned balance at once; money
    // that comes in today does not raise it.
    w.set_param("finance.transfer_fee_bp", 0);
    let bob = account_holder(&mut w, "Bob", 0);
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 30_000 }));
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(1_000 + 10_000));
    ok(&mut w, bob, "transfer", json!({ "to": ann, "amount": 30_000 }));
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(1_000 + 10_000));

    // A repaid loan adds 20,000.
    ok(&mut w, ann, "loan_request", json!({ "amount": 11_000, "term_days": 2 }));
    ok(&mut w, ann, "loan_repay", json!({ "amount": 11_000 }));
    assert_eq!(full(&w, ann)["credit"]["loans_repaid"], json!(1));
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(1_000 + 10_000 + 20_000));

    // The limit never passes `loan_limit_max`.
    w.set_param("finance.loan_limit_max", 12_345);
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(12_345));
    conserved(&w);
}

#[test]
fn deposit_interest_comes_from_the_bank_treasury() {
    let mut w = world();
    // 200 basis points per year: 280,000 earn exactly 200 per day.
    w.set_param("finance.base_rate_bp", 500);
    w.set_param("finance.deposit_spread_bp", 300);
    let ann = account_holder(&mut w, "Ann", 280_000);
    let total = conserved(&w);
    assert_eq!(bank_treasury(&w), OPEN_FEE);

    // The lowest balance of the first day was 0: no interest.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 280_000);

    // 200 are due, but the bank has only the 100 of the fee.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 280_100);
    assert_eq!(bank_treasury(&w), 0);

    // An empty treasury pays nothing.
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 280_100);

    // With income the bank pays in full. A second account with a quarter
    // of the balance gets a quarter; when the treasury is short, both get
    // the same share.
    let bob = account_holder(&mut w, "Bob", 70_000);
    assert_eq!(bank_treasury(&w), OPEN_FEE);
    next_day(&mut w);
    // Bob opened today, so only Ann earns: 200 due, 100 there.
    assert_eq!((balance(&w, ann), balance(&w, bob)), (280_200, 70_000));
    common::fund_global(&mut w, unsup_kernel::Holder::Org(unsup_kernel::OrgId::CENTRAL_BANK), 125);
    next_day(&mut w);
    // 200 and 50 are due, 125 are there: each gets one half.
    assert_eq!((balance(&w, ann), balance(&w, bob)), (280_300, 70_025));
    assert_eq!(bank_treasury(&w), 0);
    common::fund_global(
        &mut w,
        unsup_kernel::Holder::Org(unsup_kernel::OrgId::CENTRAL_BANK),
        1_000,
    );
    next_day(&mut w);
    assert_eq!((balance(&w, ann), balance(&w, bob)), (280_500, 70_075));
    assert_eq!(bank_treasury(&w), 750);
    assert_eq!(conserved(&w), total + OPEN_FEE + 70_000 + 1_125, "interest creates no money");
}

#[test]
fn an_instalment_is_rounded_up_and_the_last_one_takes_the_rest() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 100_000);
    let start = conserved(&w);
    // 10,000 in 3 instalments: 3,333.33 each, rounded up to 3,334.
    let result = ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 3 }));
    assert_eq!(result["instalment"], json!(3_334));
    next_day(&mut w);

    // 1 percent a day of what is left: 100, then 66.66, then 33.32 and the
    // 0.66 that was carried. The last instalment is the rest: 3,332.
    let mut expected = 110_000;
    for (principal, interest) in [(3_334, 100), (3_334, 66), (3_332, 33)] {
        next_day(&mut w);
        expected -= principal + interest;
        assert_eq!(balance(&w, ann), expected, "after the instalment of {principal}");
    }
    assert_eq!(burned(&w, "bank.loan_repay"), 10_000);
    assert_eq!(bank_treasury(&w), OPEN_FEE + 199);
    assert_eq!(w.global().res::<Bank>().account(ann).map(|a| a.loans_repaid), Some(1));
    assert_eq!(conserved(&w), start);
}

#[test]
fn the_first_instalment_follows_the_parameter() {
    let mut w = world();
    simple_rates(&mut w);
    // The data file gives the borrower two midnights. With 1 the first
    // midnight already collects.
    assert_eq!(w.param("finance.loan_first_due_days"), 2);
    w.set_param("finance.loan_first_due_days", 1);
    let ann = account_holder(&mut w, "Ann", 100_000);
    w.act_ok(ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    w.step(2);
    let report = w.last_report(ann).expect("the loan request reports");
    assert_eq!(report.result["first_due_in_ticks"], json!(1_440 - (report.tick.0 - 1)));
    assert_eq!(
        w.observe(ann).sections["bank"]["loan"]["next_in_ticks"],
        json!(1_440 - w.world.frame())
    );
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 110_000 - 2_600);
    assert_eq!(burned(&w, "bank.loan_repay"), 2_500);

    // A loan keeps the day that it got. A later change of the parameter is
    // for new loans.
    w.set_param("finance.loan_first_due_days", 5);
    next_day(&mut w);
    assert_eq!(balance(&w, ann), 110_000 - 2_600 - 2_575);
    conserved(&w);
}

#[test]
fn deposit_interest_below_one_cent_is_carried_to_the_next_day() {
    let mut w = world();
    // 200 basis points a year on 1,000: 0.714 cents a day.
    w.set_param("finance.base_rate_bp", 500);
    w.set_param("finance.deposit_spread_bp", 300);
    let ann = account_holder(&mut w, "Ann", 1_000);
    let total = conserved(&w);
    // The first midnight pays nothing: the lowest balance of day 0 was 0.
    // Then the parts of a cent add up: 0.71, 1.43, 2.14 (on 1,001), 2.86
    // (on 1,002), 3.58. A cent is paid each time the sum passes a whole
    // number.
    for expected in [1_000, 1_000, 1_001, 1_002, 1_002, 1_003] {
        next_day(&mut w);
        assert_eq!(balance(&w, ann), expected);
    }
    assert_eq!(bank_treasury(&w), OPEN_FEE - 3);
    assert_eq!(conserved(&w), total);
}

#[test]
fn a_loan_that_the_account_cannot_hold_is_refused() {
    let mut w = world();
    simple_rates(&mut w);
    let ann = account_holder(&mut w, "Ann", 100_000);
    // An account at the top of the range of the ledger. No game rule can
    // make one; the test writes the balance directly.
    common::fund_global(&mut w, Bank::holder(ann), (unsup_kernel::SUPPLY_CAP + 100_000) - 110_000);
    assert_eq!(balance(&w, ann), (unsup_kernel::SUPPLY_CAP + 100_000) - 10_000);

    let error = err(&mut w, ann, "loan_request", json!({ "amount": 10_001, "term_days": 4 }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
    assert_eq!(error.message, "The amount is too large for your account.");
    // Nothing was created, so nothing is owed.
    assert_eq!(minted(&w, "bank.loan_issue"), 0);
    let bank = w.global().res::<Bank>();
    assert_eq!((bank.outstanding_principal(), bank.loan_count()), (0, 0));
    assert!(bank.account(ann).is_some_and(|account| account.loan.is_none()));
    drop(bank);
    assert_eq!(balance(&w, ann), (unsup_kernel::SUPPLY_CAP + 100_000) - 10_000);

    // An amount that fits is granted.
    ok(&mut w, ann, "loan_request", json!({ "amount": 10_000, "term_days": 4 }));
    assert_eq!(minted(&w, "bank.loan_issue"), 10_000);
    assert_eq!(balance(&w, ann), (unsup_kernel::SUPPLY_CAP + 100_000));
    w.assert_conserved();
}

/// The deposit of the saver of [`secured`].
const SAVINGS: i64 = 500_000;

/// A world with the numbers of the data file, except that deposits earn
/// nothing and a loan costs exactly 1 percent per day. A saver has a
/// large deposit, so that the reserve limit leaves room for the loans.
fn secured(w: &mut TestWorld<'_>) {
    w.set_param("finance.base_rate_bp", 2_500);
    w.set_param("finance.loan_spread_bp", 300);
    w.set_param("finance.deposit_spread_bp", 10_000);
    w.set_param("finance.transfer_fee_bp", 0);
    account_holder(w, "Sam", SAVINGS);
}

#[test]
fn the_data_file_gives_credit_for_seasoned_deposits_only() {
    let mut w = world();
    secured(&mut w);
    assert_eq!(w.param("finance.loan_base_limit"), 0);
    assert_eq!(w.param("finance.loan_limit_per_account_day"), 0);
    assert_eq!(w.param("finance.loan_limit_per_repaid_loan"), 0);
    assert_eq!(w.param("finance.loan_collateral_bp"), 10_000);
    // A resident with an old account and no money gets nothing, however
    // long it waits.
    let idle = account_holder(&mut w, "Idle", 0);
    let ann = account_holder(&mut w, "Ann", 30_000);
    for _ in 0..5 {
        next_day(&mut w);
    }
    assert_eq!(full(&w, idle)["credit"]["limit"], json!(0));
    let error = err(&mut w, idle, "loan_request", json!({ "amount": 1_000, "term_days": 2 }));
    assert_eq!((error.code, error.details), (ErrorCode::NotAllowed, json!({ "limit": 0 })));
    // The deposit that stayed is the limit.
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(30_000));
    // A repaid loan adds nothing to it.
    ok(&mut w, ann, "loan_request", json!({ "amount": 30_000, "term_days": 2 }));
    ok(&mut w, ann, "loan_repay", json!({ "amount": 30_000 }));
    assert_eq!(full(&w, ann)["credit"]["loans_repaid"], json!(1));
    assert_eq!(full(&w, ann)["credit"]["limit"], json!(30_000));
    conserved(&w);
}

#[test]
fn the_bank_holds_collateral_and_a_default_costs_it_nothing() {
    let mut w = world();
    secured(&mut w);
    let ann = account_holder(&mut w, "Ann", 20_000);
    let bob = account_holder(&mut w, "Bob", 0);
    next_day(&mut w);
    next_day(&mut w);
    let start = conserved(&w);
    assert_eq!(start, 3 * OPEN_FEE + SAVINGS + 20_000);

    // Ann borrows all that her deposit allows. The bank holds the same
    // amount in her account.
    let loan = ok(&mut w, ann, "loan_request", json!({ "amount": 20_000, "term_days": 4 }));
    assert_eq!((loan["balance"].as_i64(), loan["held"].as_i64()), (Some(40_000), Some(20_000)));
    assert_eq!(w.observe(ann).sections["bank"]["loan"]["held"], json!(20_000));

    // She cannot take the deposit and the loan: that was the way to double
    // money with a resident that then defaults.
    let error = err(&mut w, ann, "withdraw", json!({ "amount": 40_000 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 40_000, "available": 20_000, "held": 20_000 }));
    let error = err(&mut w, ann, "transfer", json!({ "to": bob, "amount": 20_001 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 20_001, "available": 20_000, "held": 20_000 }));
    assert_eq!(balance(&w, ann), 40_000, "a refused request moves nothing");
    // The free half leaves the bank.
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 19_000 }));
    ok(&mut w, ann, "withdraw", json!({ "amount": 1_000 }));
    w.step(1);
    assert_eq!((balance(&w, ann), w.cash(ann).0), (20_000, 1_000));

    // The instalments come from free money, not from the collateral: 5,000
    // and 200 of interest are due and nothing is free.
    next_day(&mut w);
    next_day(&mut w);
    assert_eq!(
        notice(&w, ann, "finance.instalment_missed"),
        Some(json!({
            "due": 5_200, "balance": 20_000, "held": 20_000, "missed": 1, "default_after": 3,
        }))
    );
    assert_eq!(balance(&w, ann), 20_000);
    // An early repayment of principal can use the collateral that it
    // frees. Interest cannot.
    let error = err(&mut w, ann, "loan_repay", json!({ "amount": 200 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 200, "available": 0 }));
    next_day(&mut w);

    // The third miss is the default. The bank takes its collateral: the
    // whole principal. Only the interest of three days is lost.
    let frames = next_day_frames(&mut w);
    let defaulted = events(&frames, "finance.loan_defaulted");
    assert_eq!(defaulted.len(), 1);
    assert_eq!(defaulted[0].data, json!({ "resident": ann, "outstanding": 600 }));
    assert_eq!(balance(&w, ann), 0);
    assert_eq!(burned(&w, "bank.loan_repay"), 20_000);
    assert_eq!(minted(&w, "bank.loan_issue"), 20_000);
    {
        let bank = w.global().res::<Bank>();
        assert_eq!((bank.outstanding_principal(), bank.written_off()), (0, 0));
        assert_eq!(bank.account(ann).map(|account| account.defaults), Some(1));
    }
    // The loan made no money: what Ann moved out is what she had before.
    assert_eq!(conserved(&w), start);
    assert_eq!(balance(&w, bob) + w.cash(ann).0, 20_000);
}

#[test]
fn a_repayment_frees_the_collateral_step_by_step() {
    let mut w = world();
    secured(&mut w);
    let ann = account_holder(&mut w, "Ann", 20_000);
    let bob = account_holder(&mut w, "Bob", 0);
    next_day(&mut w);
    next_day(&mut w);
    ok(&mut w, ann, "loan_request", json!({ "amount": 20_000, "term_days": 4 }));
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 20_000 }));
    // Nothing is free, but Ann can pay principal out of the collateral.
    let paid = ok(&mut w, ann, "loan_repay", json!({ "amount": 5_000 }));
    assert_eq!((paid["principal"].as_i64(), paid["balance"].as_i64()), (Some(5_000), Some(15_000)));
    assert_eq!(w.observe(ann).sections["bank"]["loan"]["held"], json!(15_000));
    // The rest closes the loan, and the account is free again.
    let paid = ok(&mut w, ann, "loan_repay", json!({ "amount": 15_000 }));
    assert_eq!((paid["closed"].as_bool(), paid["balance"].as_i64()), (Some(true), Some(0)));
    assert!(w.observe(ann).sections["bank"].get("loan").is_none());
    assert_eq!(burned(&w, "bank.loan_repay"), 20_000);
    conserved(&w);
}

#[test]
fn a_smaller_cover_leaves_a_part_of_the_loan_unsecured() {
    let mut w = world();
    secured(&mut w);
    // A server that accepts the risk: the bank holds half of each loan.
    w.set_param("finance.loan_collateral_bp", 5_000);
    let ann = account_holder(&mut w, "Ann", 20_000);
    let bob = account_holder(&mut w, "Bob", 0);
    next_day(&mut w);
    next_day(&mut w);
    let loan = ok(&mut w, ann, "loan_request", json!({ "amount": 20_000, "term_days": 4 }));
    assert_eq!(loan["held"], json!(10_000));
    let error = err(&mut w, ann, "transfer", json!({ "to": bob, "amount": 30_001 }));
    assert_eq!(error.details, json!({ "needed": 30_001, "available": 30_000, "held": 10_000 }));
    ok(&mut w, ann, "transfer", json!({ "to": bob, "amount": 30_000 }));
    for _ in 0..5 {
        next_day(&mut w);
    }
    // The default takes the collateral and writes the other half off.
    let bank = w.global().res::<Bank>();
    assert_eq!((bank.outstanding_principal(), bank.written_off()), (0, 10_000));
    drop(bank);
    assert_eq!(balance(&w, ann), 0);
    conserved(&w);
}
