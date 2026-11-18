//! The work of the bank at the start of each game day: daily instalments,
//! defaults, deposit interest and the low-water marks.

use serde_json::json;
use unsup_kernel::{EventMeta, Realm, ResidentId};

use super::counter::BANK_TREASURY;
use super::loans;
use super::state::Bank;
use crate::params::Params;
use crate::util::{BP, calendar_day, day_of, drama, mul_div};

/// Runs the daily work once for every new day. A global system calls it
/// every tick.
pub(crate) fn run(realm: &Realm, p: &Params) {
    let today = day_of(realm);
    if realm.res::<Bank>().day >= today {
        return;
    }
    let mut bank = realm.res_mut::<Bank>();
    bank.day = today;
    let residents: Vec<ResidentId> = bank.accounts.keys().map(|&id| ResidentId(id)).collect();
    for &resident in &residents {
        collect(&mut bank, realm, p, resident, today);
    }
    pay_deposit_interest(&mut bank, realm, p);
    for &resident in &residents {
        let balance = Bank::balance(realm, resident);
        if let Some(account) = bank.accounts.get_mut(&resident.0) {
            account.low_yesterday = account.low_today;
            account.low_today = balance;
        }
    }
}

/// Collects the daily instalment of one loan, or counts a missed one.
fn collect(bank: &mut Bank, realm: &Realm, p: &Params, resident: ResidentId, today: u64) {
    let days_per_year = i64::from(realm.layout().days_per_year());
    let Some(loan) = bank.accounts.get_mut(&resident.0).and_then(|a| a.loan.as_mut()) else {
        return;
    };
    if today < loan.first_due_day {
        return;
    }
    loan.accrue_day(days_per_year);
    let (interest, principal) = (loan.due_interest, loan.due_principal);
    if loans::pay(bank, realm, p, resident, interest, principal).is_ok() {
        return;
    }
    let Some(loan) = bank.accounts.get_mut(&resident.0).and_then(|a| a.loan.as_mut()) else {
        return;
    };
    loan.missed += 1;
    let missed = loan.missed;
    let limit = realm.param(p.loan_missed_max);
    if i64::from(missed) >= limit {
        default_loan(bank, realm, p, resident, today);
    } else {
        realm.notify(
            resident,
            "finance.instalment_missed",
            None,
            json!({
                "due": interest + principal,
                "balance": Bank::balance(realm, resident),
                "missed": missed,
                "default_after": limit,
            }),
        );
    }
}

/// How a loan ended that the borrower did not repay.
pub(crate) struct LoanEnd {
    /// Cents that the bank took from the account.
    pub(crate) seized: i64,
    /// Principal and due interest that nobody paid.
    pub(crate) unpaid: i64,
}

/// Ends the loan of a resident without repayment: the bank seizes the
/// deposits up to the debt, principal first, and gives up the rest. The
/// seized principal is destroyed through the sink `bank.loan_repay`; the
/// seized interest goes to the bank treasury. Returns `None` when the
/// resident has no loan.
pub(crate) fn end_loan(
    bank: &mut Bank,
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
) -> Option<LoanEnd> {
    let loan = bank.accounts.get_mut(&resident.0).and_then(|a| a.loan.take())?;
    let balance = Bank::balance(realm, resident);
    let principal = balance.min(loan.principal);
    let interest = (balance - principal).min(loan.due_interest);
    // Neither can fail: the two amounts together are at most the balance.
    let _ = bank.burn_out(realm, resident, p.loan_repay, principal);
    let _ = bank.pay_out(realm, resident, BANK_TREASURY, interest, "loan.interest");
    bank.principal -= loan.principal;
    bank.written_off += loan.principal - principal;
    Some(LoanEnd { seized: principal + interest, unpaid: loan.owed() - principal - interest })
}

/// Makes a loan default: the loan ends without repayment, the record of
/// the resident gets a default, and the resident cannot borrow for
/// `finance.bankruptcy_days`.
fn default_loan(bank: &mut Bank, realm: &Realm, p: &Params, resident: ResidentId, today: u64) {
    let Some(end) = end_loan(bank, realm, p, resident) else {
        return;
    };
    let until = today + u64::try_from(realm.param(p.bankruptcy_days)).unwrap_or(0);
    if let Some(account) = bank.accounts.get_mut(&resident.0) {
        account.defaults += 1;
        account.bankrupt_until = until;
    }
    realm.emit(
        EventMeta::public("finance.loan_defaulted", drama(realm, p.drama_loan_defaulted))
            .actor(resident),
        &json!({ "resident": resident, "outstanding": end.unpaid }),
    );
    realm.notify(
        resident,
        "finance.loan_defaulted",
        None,
        json!({
            "outstanding": end.unpaid,
            "seized": end.seized,
            "bankrupt_until": calendar_day(realm, until),
        }),
    );
}

/// Pays one day of interest on the lowest balance of the day to every
/// account. The money comes from the bank treasury. When the treasury does
/// not have enough, every account gets the same share of its interest.
fn pay_deposit_interest(bank: &mut Bank, realm: &Realm, p: &Params) {
    let rate = (realm.param(p.base_rate_bp) - realm.param(p.deposit_spread_bp)).max(0);
    if rate == 0 {
        return;
    }
    let unit = i128::from(BP) * i128::from(realm.layout().days_per_year().max(1));
    let mut due: Vec<(ResidentId, i64)> = Vec::new();
    let mut total = 0i64;
    for (&id, account) in &mut bank.accounts {
        let numer = i128::from(account.low_today.max(0)) * i128::from(rate)
            + i128::from(account.interest_carry);
        let cents = i64::try_from(numer / unit).unwrap_or(0);
        account.interest_carry = i64::try_from(numer % unit).unwrap_or(0);
        if cents > 0 {
            due.push((ResidentId(id), cents));
            total = total.saturating_add(cents);
        }
    }
    let funds = realm.ledger().money(BANK_TREASURY).0;
    for (resident, cents) in due {
        let paid = if total <= funds { cents } else { mul_div(cents, funds, total) };
        // It cannot fail: the payments together are at most the funds.
        let _ = bank.pay_in(realm, BANK_TREASURY, resident, paid, "bank.interest");
    }
}
