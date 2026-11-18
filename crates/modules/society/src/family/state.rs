//! The registry of marriages and households. It is state of the global
//! realm and the source of truth. The component `Kin` of each resident is a
//! copy that this module writes.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::{ResidentId, Resource, SiteId, ZoneId};

/// An open marriage proposal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Proposal {
    pub(crate) to: ResidentId,
    /// The last frame in which the proposal can be accepted.
    pub(crate) expires: u64,
}

/// A household: a host and the residents that live in its home.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Household {
    /// The host. It owns or rents the home.
    pub(crate) head: ResidentId,
    /// The zone of the home.
    pub(crate) zone: ZoneId,
    /// The home.
    pub(crate) site: SiteId,
    /// The members, with the host.
    pub(crate) members: BTreeSet<ResidentId>,
    /// Invited resident to the last frame in which it can join.
    pub(crate) invited: BTreeMap<ResidentId, u64>,
}

/// Every marriage, proposal and household. A resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Families {
    /// Each married resident to its spouse. Both directions are there.
    pub(crate) spouses: BTreeMap<ResidentId, ResidentId>,
    /// The open proposal of each resident that proposed.
    pub(crate) proposals: BTreeMap<ResidentId, Proposal>,
    pub(crate) households: BTreeMap<u64, Household>,
    pub(crate) household_of: BTreeMap<ResidentId, u64>,
}

impl Resource for Families {
    const NAME: &'static str = "society.families";
}
