//! The state of the central bank: what it issued and retired this year.

use serde::{Deserialize, Serialize};
use unsup_kernel::Resource;

/// The yearly counters of the central bank. They live in the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CentralBank {
    /// The game year of the counters.
    pub(crate) year: u32,
    /// Cents issued to the nation treasury in that year.
    pub(crate) issued: i64,
    /// Cents of treasury money retired in that year.
    pub(crate) retired: i64,
}

impl Resource for CentralBank {
    const NAME: &'static str = "finance.central_bank";
}

impl CentralBank {
    /// Starts new counters when the game year changed.
    pub(crate) fn roll(&mut self, year: u32) {
        if self.year != year {
            *self = CentralBank { year, issued: 0, retired: 0 };
        }
    }

    /// Returns the cents issued in a game year.
    #[must_use]
    pub fn issued_in(&self, year: u32) -> i64 {
        if self.year == year { self.issued } else { 0 }
    }

    /// Returns the cents retired in a game year.
    #[must_use]
    pub fn retired_in(&self, year: u32) -> i64 {
        if self.year == year { self.retired } else { 0 }
    }
}
