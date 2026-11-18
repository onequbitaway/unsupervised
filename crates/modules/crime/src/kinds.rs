//! The kinds of crime.

use serde::{Deserialize, Serialize};

/// A kind of crime. Each kind has its own sentence numbers in the data
/// file: `crime.<kind>.fine`, `crime.<kind>.jail_ticks`,
/// `crime.<kind>.fine_max`, `crime.<kind>.jail_max_ticks`,
/// `crime.<kind>.notoriety` and `crime.<kind>.drama`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrimeKind {
    /// Cash taken from a resident in secret (`pickpocket`).
    Theft,
    /// The till and the stock of a site taken (`burgle`).
    Burglary,
    /// Cash and goods taken from a resident by force (`rob`).
    Robbery,
    /// An attack on a resident (`assault`).
    Assault,
    /// An attack that killed the victim.
    Homicide,
    /// Damage to a site (`vandalize`).
    Vandalism,
    /// A secret payment (`bribe`).
    Bribery,
    /// Contraband that a resident carried.
    Smuggling,
}

impl CrimeKind {
    /// Every kind, in the order of the data file.
    pub const ALL: [CrimeKind; 8] = [
        CrimeKind::Theft,
        CrimeKind::Burglary,
        CrimeKind::Robbery,
        CrimeKind::Assault,
        CrimeKind::Homicide,
        CrimeKind::Vandalism,
        CrimeKind::Bribery,
        CrimeKind::Smuggling,
    ];

    /// Returns the name of the kind as events, notices and parameters
    /// write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            CrimeKind::Theft => "theft",
            CrimeKind::Burglary => "burglary",
            CrimeKind::Robbery => "robbery",
            CrimeKind::Assault => "assault",
            CrimeKind::Homicide => "homicide",
            CrimeKind::Vandalism => "vandalism",
            CrimeKind::Bribery => "bribery",
            CrimeKind::Smuggling => "smuggling",
        }
    }

    /// Returns the position of the kind in [`CrimeKind::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Returns true for a crime that nobody sees: no witness, no public
    /// event and no place on the docket until evidence exists.
    #[must_use]
    pub const fn is_hidden(self) -> bool {
        matches!(self, CrimeKind::Bribery)
    }

    /// Returns true for a crime that the residents around it can notice.
    /// A bribe is secret, and contraband is in a pocket until a search
    /// finds it.
    #[must_use]
    pub const fn has_witnesses(self) -> bool {
        !matches!(self, CrimeKind::Bribery | CrimeKind::Smuggling)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_index_matches_the_list_and_the_wire_name() {
        for (slot, kind) in CrimeKind::ALL.into_iter().enumerate() {
            assert_eq!(kind.index(), slot);
            let wire = serde_json::to_value(kind).expect("a kind is JSON");
            assert_eq!(wire, serde_json::json!(kind.name()));
        }
    }
}
