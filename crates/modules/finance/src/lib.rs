//! Bank, loans, the exchange, shares, the central bank and statistics.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`
//! for the kernel and `docs/design/modules/finance.md` for the rules and
//! the formulas of this module.
//!
//! # Map of the crate
//!
//! - [`bank`] has the accounts (`open_account`, `deposit`, `withdraw`,
//!   `transfer`) and the loans (`loan_request`, `loan_repay`). Its state is
//!   in the global realm.
//! - [`exchange`] has one order book per item at the `exchange` site of a
//!   city (`order_place`, `order_cancel`, `market_claim`, `market_book`).
//!   Its state is in the zone.
//! - [`shares`] has the shares of organizations (`issue_shares`,
//!   `buy_shares`, `sell_shares`, `cancel_shares`, `shares_book`,
//!   `pay_dividend`).
//! - [`central`] has the central bank (`central_bank_set_rate`,
//!   `central_bank_issue`, `central_bank_retire`).
//! - [`stats`] has the public statistics (`finance.stats_published`).
//! - [`estate`] says what happens to all of this when a resident leaves
//!   the world.
//! - `interrupted` tells a resident that a request which had left for the
//!   global realm is still handled after its action stopped.
//!
//! Money never appears or vanishes in this module except through the
//! faucets `bank.loan_issue` and `central_bank.issue` and the sinks
//! `bank.loan_repay` and `central_bank.retire`.

pub mod bank;
pub mod central;
pub mod estate;
pub mod exchange;
mod interrupted;
mod params;
pub mod shares;
pub mod stats;
mod util;

use unsup_kernel::{Builder, Plugin};

use crate::params::Params;

/// The finance module.
#[derive(Clone, Copy, Debug, Default)]
pub struct FinancePlugin;

impl Plugin for FinancePlugin {
    fn name(&self) -> &'static str {
        util::MODULE
    }

    fn build(&self, b: &mut Builder) {
        let p = Params::load(b);
        bank::register(b, p);
        exchange::register(b, p);
        shares::register(b, p);
        central::register(b, p);
        stats::register(b, p);
        estate::register(b, p);
        interrupted::register(b);
    }
}

/// Returns the plugin of the finance module.
#[must_use]
pub fn plugin() -> FinancePlugin {
    FinancePlugin
}
