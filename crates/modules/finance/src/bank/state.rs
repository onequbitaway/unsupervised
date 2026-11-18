//! The state of the bank: accounts, loans and credit records. It lives in
//! the global realm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Holder, Realm, ResidentId, Resource};

/// One loan of a resident.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Loan {
    /// Principal that is not yet repaid, in cents.
    pub principal: i64,
    /// Yearly interest rate in basis points. It is fixed when the loan is
    /// granted.
    pub rate_bp: i64,
    /// Principal part of one daily instalment, in cents.
    pub instalment: i64,
    /// Daily instalments that are still to come.
    pub days_left: u32,
    /// The day (see `Bank::day`) at whose start the first instalment is due.
    pub first_due_day: u64,
    /// Principal that is due now: the instalments that were not paid.
    pub due_principal: i64,
    /// Interest that is due now, in cents.
    pub due_interest: i64,
    /// Interest below one cent that is carried to the next day. The unit is
    /// 1 / (10,000 * days per year) of a cent.
    pub interest_carry: i64,
    /// Daily instalments that were missed in a row.
    pub missed: u32,
}

impl Loan {
    /// Returns everything that the borrower owes now: the principal and the
    /// interest that is due.
    #[must_use]
    pub fn owed(&self) -> i64 {
        self.principal + self.due_interest
    }
}

/// The bank account of one resident, with its credit record.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Account {
    /// The day on which the account was opened.
    pub opened_day: u64,
    /// The lowest balance since the start of the current day.
    pub low_today: i64,
    /// The lowest balance of the day before.
    pub low_yesterday: i64,
    /// Deposit interest below one cent that is carried to the next day.
    pub interest_carry: i64,
    /// Loans that the resident repaid in full.
    pub loans_repaid: u32,
    /// Loans on which the resident defaulted.
    pub defaults: u32,
    /// The resident cannot borrow before this day. 0 when it never
    /// defaulted.
    pub bankrupt_until: u64,
    /// The loan of the resident, if it has one.
    pub loan: Option<Loan>,
}

impl Account {
    /// Returns the seasoned balance: the lowest balance of the day before
    /// and of the current day. Money that came in today does not count.
    #[must_use]
    pub fn seasoned(&self) -> i64 {
        self.low_yesterday.min(self.low_today).max(0)
    }
}

/// The bank: every account, and the totals that the reserve limit and the
/// statistics need.
///
/// The money of an account is the balance of `Holder::Account(resident id)`
/// in the ledger of the global realm. This resource holds only what the
/// ledger does not know.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bank {
    pub(crate) accounts: BTreeMap<u64, Account>,
    /// Sum of the balances of all open accounts.
    pub(crate) deposits: i64,
    /// Sum of the principal of all loans.
    pub(crate) principal: i64,
    /// Principal that the bank gave up after a default, since the world
    /// began.
    pub(crate) written_off: i64,
    /// The last day whose start the bank has processed.
    pub(crate) day: u64,
}

impl Resource for Bank {
    const NAME: &'static str = "finance.bank";
}

impl Bank {
    /// Returns the ledger holder of the account of a resident.
    #[must_use]
    pub fn holder(resident: ResidentId) -> Holder {
        Holder::Account(resident.0)
    }

    /// Returns the balance of the account of a resident in cents. Call it
    /// with the global realm.
    #[must_use]
    pub fn balance(realm: &Realm, resident: ResidentId) -> i64 {
        realm.ledger().money(Bank::holder(resident)).0
    }

    /// Returns the account of a resident, if it has one.
    #[must_use]
    pub fn account(&self, resident: ResidentId) -> Option<&Account> {
        self.accounts.get(&resident.0)
    }

    /// Returns true when the resident has an account.
    #[must_use]
    pub fn is_open(&self, resident: ResidentId) -> bool {
        self.accounts.contains_key(&resident.0)
    }

    /// Returns every account, ordered by resident.
    pub fn accounts(&self) -> impl Iterator<Item = (ResidentId, &Account)> + '_ {
        self.accounts.iter().map(|(&id, account)| (ResidentId(id), account))
    }

    /// Returns the sum of the balances of all accounts in cents.
    #[must_use]
    pub fn deposits(&self) -> i64 {
        self.deposits
    }

    /// Returns the sum of the principal of all loans in cents.
    #[must_use]
    pub fn outstanding_principal(&self) -> i64 {
        self.principal
    }

    /// Returns the principal that the bank gave up after defaults, in cents.
    #[must_use]
    pub fn written_off(&self) -> i64 {
        self.written_off
    }

    /// Returns the number of loans.
    #[must_use]
    pub fn loan_count(&self) -> usize {
        self.accounts.values().filter(|account| account.loan.is_some()).count()
    }
}
