//! Loans: `loan_request` and `loan_repay`, the credit limit, the reserve
//! limit and the payment rules that the daily collection shares.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, Builder, EventMeta, LedgerError, Realm, ResidentId, TICKS_PER_DAY,
};

use super::counter::BANK_TREASURY;
use super::state::{Account, Bank, Loan};
use crate::params::Params;
use crate::util::{BP, calendar_day, day_of, drama, no_account, part, positive, remote, spec};

/// Arguments of `loan_request`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct LoanRequestArgs {
    /// The amount to borrow in cents. The bank puts it in your account.
    pub amount: i64,
    /// The number of daily instalments in which you repay the loan.
    pub term_days: u32,
}

/// Arguments of `loan_repay`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct LoanRepayArgs {
    /// The amount to repay from your account in cents. An amount above
    /// what you owe pays the whole loan.
    pub amount: i64,
}

impl Loan {
    /// Adds one day to what is due: the interest of one day on the
    /// principal, and one instalment. On the last day of the term the
    /// whole principal is due.
    pub(crate) fn accrue_day(&mut self, days_per_year: i64) {
        let unit = i128::from(BP) * i128::from(days_per_year.max(1));
        let numer =
            i128::from(self.principal) * i128::from(self.rate_bp) + i128::from(self.interest_carry);
        // The carry is below `unit`, and the interest of one day is below
        // the principal, so both fit.
        self.due_interest += i64::try_from(numer / unit).unwrap_or(0);
        self.interest_carry = i64::try_from(numer % unit).unwrap_or(0);
        self.days_left = self.days_left.saturating_sub(1);
        self.due_principal = if self.days_left == 0 {
            self.principal
        } else {
            self.principal.min(self.due_principal + self.instalment)
        };
    }

    /// Returns what the next daily collection takes from the account:
    /// the principal part and the interest part, in cents.
    #[must_use]
    pub fn next_instalment(&self, days_per_year: u32) -> (i64, i64) {
        let mut next = self.clone();
        next.accrue_day(i64::from(days_per_year));
        (next.due_principal, next.due_interest)
    }
}

/// Returns the credit limit of an account on a day, in cents.
///
/// See `docs/design/modules/finance.md` for the formula.
pub(crate) fn credit_limit(realm: &Realm, p: &Params, account: &Account, day: u64) -> i64 {
    let age = i64::try_from(day.saturating_sub(account.opened_day)).unwrap_or(i64::MAX);
    let raw = realm
        .param(p.loan_base_limit)
        .saturating_add(age.saturating_mul(realm.param(p.loan_limit_per_account_day)))
        .saturating_add(
            i64::from(account.loans_repaid)
                .saturating_mul(realm.param(p.loan_limit_per_repaid_loan)),
        )
        .saturating_add(part(account.seasoned(), realm.param(p.loan_deposit_factor_bp)));
    let lost = i64::from(account.defaults).saturating_mul(realm.param(p.loan_default_penalty_bp));
    part(raw, (BP - lost).max(0)).min(realm.param(p.loan_limit_max))
}

/// Returns the collateral that the bank holds for an open principal: the
/// part of the account balance that the borrower cannot take out.
pub(crate) fn collateral(realm: &Realm, p: &Params, principal: i64) -> i64 {
    part(principal.max(0), realm.param(p.loan_collateral_bp).clamp(0, BP))
}

/// Returns the collateral that the bank holds in the account of a
/// resident, in cents. It is zero without a loan.
pub(crate) fn held(realm: &Realm, p: &Params, bank: &Bank, resident: ResidentId) -> i64 {
    let principal = bank.account(resident).and_then(|a| a.loan.as_ref()).map_or(0, |l| l.principal);
    collateral(realm, p, principal)
}

/// Returns what a resident can take out of its account: the balance
/// without the collateral of its loan.
pub(crate) fn available(realm: &Realm, p: &Params, bank: &Bank, resident: ResidentId) -> i64 {
    (Bank::balance(realm, resident) - held(realm, p, bank, resident)).max(0)
}

/// Checks that a resident can take `needed` cents out of its account. The
/// collateral of a loan stays in the account.
pub(crate) fn check_free(
    realm: &Realm,
    p: &Params,
    bank: &Bank,
    resident: ResidentId,
    needed: i64,
    what: &str,
) -> Result<(), ActionError> {
    let held = held(realm, p, bank, resident);
    let free = (Bank::balance(realm, resident) - held).max(0);
    if free >= needed {
        return Ok(());
    }
    let mut details = json!({ "needed": needed, "available": free });
    let message = if held > 0 {
        details["held"] = json!(held);
        format!(
            "Your balance is too low for this {what}. The bank holds {held} cents as collateral for your loan."
        )
    } else {
        format!("Your balance is too low for this {what}.")
    };
    Err(ActionError::insufficient(message).with_details(details))
}

/// Returns how much the bank can still lend under the reserve limit, in
/// cents.
pub(crate) fn lending_room(realm: &Realm, p: &Params, bank: &Bank) -> i64 {
    let lendable = part(bank.deposits, (BP - realm.param(p.reserve_ratio_bp)).max(0));
    (lendable - bank.principal).max(0)
}

/// Returns the number of ticks until the bank next collects an instalment
/// of a loan.
pub(crate) fn ticks_to_collection(realm: &Realm, loan: &Loan) -> u64 {
    let next_day = (day_of(realm) + 1).max(loan.first_due_day);
    (next_day * TICKS_PER_DAY).saturating_sub(realm.frame())
}

/// Pays a part of the loan of a resident from its account. The interest
/// goes to the bank treasury. The principal is destroyed through the sink
/// `bank.loan_repay`. A loan without principal and without due interest is
/// closed and counts as repaid. Returns true when the loan was closed.
///
/// The payment must leave the collateral of the principal that stays open
/// in the account. So a repayment of principal can use the collateral
/// that it frees, and the interest must come from free money.
pub(crate) fn pay(
    bank: &mut Bank,
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    interest: i64,
    principal: i64,
) -> Result<bool, LedgerError> {
    let balance = Bank::balance(realm, resident);
    let open = bank.account(resident).and_then(|a| a.loan.as_ref()).map_or(0, |l| l.principal);
    let kept = collateral(realm, p, open - principal);
    let needed = interest.saturating_add(principal);
    if balance - kept < needed {
        return Err(LedgerError::Insufficient {
            holder: Bank::holder(resident),
            needed,
            available: (balance - kept).max(0),
        });
    }
    bank.pay_out(realm, resident, BANK_TREASURY, interest, "loan.interest")?;
    bank.burn_out(realm, resident, p.loan_repay, principal)?;
    bank.principal -= principal;
    let Some(account) = bank.accounts.get_mut(&resident.0) else {
        return Ok(false);
    };
    let Some(loan) = account.loan.as_mut() else {
        return Ok(false);
    };
    loan.due_interest -= interest;
    loan.principal -= principal;
    loan.due_principal = (loan.due_principal - principal).max(0);
    if loan.due_principal == 0 && loan.due_interest == 0 {
        loan.missed = 0;
    }
    if loan.principal > 0 || loan.due_interest > 0 {
        return Ok(false);
    }
    account.loan = None;
    account.loans_repaid += 1;
    realm.emit(
        EventMeta::public("finance.loan_repaid", drama(realm, p.drama_routine)).actor(resident),
        &json!({ "resident": resident }),
    );
    Ok(true)
}

/// Registers the loan verbs.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.global_verb::<LoanRequestArgs>(
        spec(
            "loan_request",
            "Borrow money from the bank. The bank puts it in your account and collects an \
             instalment each day.",
            false,
        ),
        move |realm, ctx, args| request(realm, &p, ctx.resident, args),
    );
    b.global_verb::<LoanRepayArgs>(
        remote("loan_repay", "Repay a part or all of your loan from your bank account.", false),
        move |realm, ctx, args| repay(realm, &p, ctx.resident, args),
    );
}

fn request(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    args: &LoanRequestArgs,
) -> Result<Value, ActionError> {
    let amount = positive(args.amount, "amount")?;
    let least = realm.param(p.loan_min);
    if amount < least {
        return Err(ActionError::invalid_args(format!("A loan is at least {least} cents.")));
    }
    let (shortest, longest) =
        (realm.param(p.loan_term_min_days), realm.param(p.loan_term_max_days));
    let term = i64::from(args.term_days);
    if term < shortest || term > longest {
        return Err(ActionError::invalid_args(format!(
            "The term of a loan is from {shortest} to {longest} days."
        )));
    }
    let today = day_of(realm);
    let mut bank = realm.res_mut::<Bank>();
    let Some(account) = bank.account(resident) else {
        return Err(no_account(resident, true));
    };
    if account.loan.is_some() {
        return Err(ActionError::conflict("You already have a loan. Repay it first."));
    }
    if today < account.bankrupt_until {
        return Err(ActionError::not_allowed("You are bankrupt. The bank does not lend to you.")
            .with_details(json!({ "until": calendar_day(realm, account.bankrupt_until) })));
    }
    let limit = credit_limit(realm, p, account, today);
    if amount > limit {
        return Err(ActionError::not_allowed("The amount is above your credit limit.")
            .with_details(json!({ "limit": limit })));
    }
    let room = lending_room(realm, p, &bank);
    if amount > room {
        return Err(ActionError::insufficient("The bank has reached its reserve limit.")
            .with_details(json!({ "needed": amount, "available": room })));
    }
    let rate_bp = (realm.param(p.base_rate_bp) + realm.param(p.loan_spread_bp)).max(0);
    // The bank collects at midnight. The first collection is this many
    // midnights away, so that a loan of the late evening is not due at once.
    let first_due_in_days = u64::try_from(realm.param(p.loan_first_due_days).max(1)).unwrap_or(1);
    let loan = Loan {
        principal: amount,
        rate_bp,
        // Rounded up, so that `term` instalments always cover the principal.
        instalment: (amount + term - 1) / term,
        days_left: args.term_days,
        first_due_day: today + first_due_in_days,
        ..Loan::default()
    };
    if bank.mint_in(realm, p.loan_issue, resident, amount) != amount {
        // Nothing was created, so nothing is owed.
        return Err(ActionError::invalid_args("The amount is too large for your account."));
    }
    bank.principal += amount;
    let first_due_in = ticks_to_collection(realm, &loan);
    let instalment = loan.instalment;
    if let Some(account) = bank.accounts.get_mut(&resident.0) {
        account.loan = Some(loan);
    }
    realm.emit(
        EventMeta::public("finance.loan_granted", drama(realm, p.drama_loan_granted))
            .actor(resident),
        &json!({
            "resident": resident,
            "amount": amount,
            "rate_bp": rate_bp,
            "term_days": args.term_days,
        }),
    );
    Ok(json!({
        "amount": amount,
        "rate_bp": rate_bp,
        "term_days": args.term_days,
        "instalment": instalment,
        "first_due_in_ticks": first_due_in,
        "balance": Bank::balance(realm, resident),
        "held": collateral(realm, p, amount),
    }))
}

fn repay(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    args: &LoanRepayArgs,
) -> Result<Value, ActionError> {
    let amount = positive(args.amount, "amount")?;
    let mut bank = realm.res_mut::<Bank>();
    let Some(account) = bank.account(resident) else {
        return Err(no_account(resident, true));
    };
    let Some(loan) = &account.loan else {
        return Err(ActionError::not_found("You have no loan."));
    };
    let owed = loan.owed();
    let paid = amount.min(owed);
    let interest = paid.min(loan.due_interest);
    let principal = paid - interest;
    let closed = pay(&mut bank, realm, p, resident, interest, principal)?;
    Ok(json!({
        "paid": paid,
        "interest": interest,
        "principal": principal,
        "remaining": owed - paid,
        "closed": closed,
        "balance": Bank::balance(realm, resident),
    }))
}
