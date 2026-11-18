//! The bank: accounts, the counter verbs, transfers and loans.
//!
//! Deposits are balances of `Holder::Account` in the ledger of the global
//! realm. The resource [`Bank`] holds what the ledger does not know: which
//! accounts are open, the loans and the credit records.

mod accounts;
mod counter;
mod daily;
mod loans;
mod observe;
mod state;
mod transfer;

use unsup_kernel::{Builder, Scope, Stage};

pub use self::counter::AmountArgs;
pub(crate) use self::daily::end_loan;
pub use self::loans::{LoanRepayArgs, LoanRequestArgs};
pub use self::state::{Account, Bank, Loan};
pub use self::transfer::TransferArgs;
use crate::params::Params;

/// Registers the bank.
pub(crate) fn register(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.resource::<Bank>();
        b.system(Stage::Pre, 0, "finance.bank_day", move |realm| daily::run(realm, &p));
    }
    counter::register(b, p);
    transfer::register(b, p);
    loans::register(b, p);
    observe::register(b, p);
}
