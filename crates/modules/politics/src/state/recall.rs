//! The record of one recall petition.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::ResidentId;

use crate::model::{Gov, Office};

/// A petition for an early election of one office.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Recall {
    /// Id of the petition.
    pub(crate) id: u64,
    /// The government of the office.
    pub(crate) gov: Gov,
    /// The office.
    pub(crate) office: Office,
    /// The resident that started the petition.
    pub(crate) starter: ResidentId,
    /// Frame of the start.
    pub(crate) started: u64,
    /// Frame at which the petition ends without effect.
    pub(crate) deadline: u64,
    /// Signatures that the petition needs.
    pub(crate) needed: u64,
    /// The residents that signed. The starter is the first.
    pub(crate) signers: BTreeSet<ResidentId>,
}

impl Recall {
    /// Returns the number of signatures.
    pub(crate) fn signatures(&self) -> u64 {
        self.signers.len() as u64
    }
}

/// Returns the signatures that a recall needs among `residents` residents:
/// `permille` thousandths of them, rounded up, and at least `least`.
pub(crate) fn needed(residents: u64, permille: i64, least: i64) -> u64 {
    let permille = u64::try_from(permille.clamp(0, 1000)).unwrap_or(1000);
    let least = u64::try_from(least.max(1)).unwrap_or(1);
    residents.saturating_mul(permille).div_ceil(1000).max(least)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_threshold_is_a_part_of_the_residents_with_a_floor() {
        assert_eq!(needed(100, 100, 3), 10);
        assert_eq!(needed(101, 100, 3), 11);
        assert_eq!(needed(10, 100, 3), 3);
        assert_eq!(needed(0, 100, 3), 3);
        assert_eq!(needed(50, 1000, 3), 50);
        assert_eq!(needed(50, 0, 0), 1);
    }
}
