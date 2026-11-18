//! What a resident remembers about its place in the economy: its job, the
//! sites that it owns and the homes that it rents.
//!
//! The zone of a site holds the truth (the job book, the owner of the site,
//! the leases). This component is a copy that moves with the resident, so
//! that the resident sees its job and its property from every zone. When
//! the truth changes while the resident is elsewhere, the zone sends a
//! [`TieChange`] to the resident.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Component, Dest, Idx, Message, Realm, ResidentId, SiteId, ZoneId};

/// The job of a resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JobTie {
    /// Id of the job.
    pub(crate) job: u64,
    /// The site of the job.
    pub(crate) site: SiteId,
    /// The zone of the site.
    pub(crate) zone: ZoneId,
    /// Kind of the site.
    pub(crate) kind: String,
    /// Wage in cents for one shift, before income tax.
    pub(crate) wage: i64,
}

/// A site that a resident owns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SiteTie {
    /// The site.
    pub(crate) site: SiteId,
    /// The zone of the site.
    pub(crate) zone: ZoneId,
    /// Kind of the site when the resident got it.
    pub(crate) kind: String,
}

/// A home that a resident rents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LeaseTie {
    /// The site.
    pub(crate) site: SiteId,
    /// The zone of the site.
    pub(crate) zone: ZoneId,
    /// Rent in cents for one day.
    pub(crate) rent: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct TiesData {
    job: Option<JobTie>,
    owned: Vec<SiteTie>,
    leases: Vec<LeaseTie>,
}

impl TiesData {
    fn is_empty(&self) -> bool {
        self.job.is_none() && self.owned.is_empty() && self.leases.is_empty()
    }
}

/// The economic ties of one resident. Most residents have none, so the
/// data is behind one pointer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Ties {
    data: Option<Box<TiesData>>,
}

impl Component for Ties {
    const NAME: &'static str = "economy.ties";
}

impl Ties {
    /// Returns the job of the resident.
    pub(crate) fn job(&self) -> Option<&JobTie> {
        self.data.as_ref().and_then(|d| d.job.as_ref())
    }

    /// Returns the sites that the resident owns, oldest first.
    pub(crate) fn owned(&self) -> &[SiteTie] {
        self.data.as_ref().map_or(&[], |d| d.owned.as_slice())
    }

    /// Returns the homes that the resident rents, oldest first.
    pub(crate) fn leases(&self) -> &[LeaseTie] {
        self.data.as_ref().map_or(&[], |d| d.leases.as_slice())
    }

    /// Changes the data and drops it when nothing is left, so that the same
    /// ties always have the same bytes.
    fn edit(&mut self, f: impl FnOnce(&mut TiesData)) {
        let mut data = self.data.take().unwrap_or_default();
        f(&mut data);
        if !data.is_empty() {
            self.data = Some(data);
        }
    }

    /// Applies one change.
    pub(crate) fn apply(&mut self, change: TieChange) {
        self.edit(|data| match change {
            TieChange::Hired(job) => data.job = Some(job),
            TieChange::JobEnded { job } => {
                if data.job.as_ref().is_some_and(|tie| tie.job == job) {
                    data.job = None;
                }
            }
            TieChange::Owned(site) => {
                data.owned.retain(|tie| tie.site != site.site);
                data.owned.push(site);
            }
            TieChange::Disowned { site } => data.owned.retain(|tie| tie.site != site),
            TieChange::Leased(lease) => {
                data.leases.retain(|tie| tie.site != lease.site);
                data.leases.push(lease);
            }
            TieChange::LeaseEnded { site } => data.leases.retain(|tie| tie.site != site),
        });
    }
}

/// A change of the ties of a resident. It is also the message that carries
/// the change to a resident in another zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TieChange {
    /// The resident has a new job.
    Hired(JobTie),
    /// The job ended: the resident quit, was fired, or the job was closed.
    JobEnded {
        /// Id of the job.
        job: u64,
    },
    /// The resident owns a site now.
    Owned(SiteTie),
    /// The resident no longer owns the site.
    Disowned {
        /// The site.
        site: SiteId,
    },
    /// The resident rents a home now.
    Leased(LeaseTie),
    /// The lease ended.
    LeaseEnded {
        /// The site.
        site: SiteId,
    },
}

impl Message for TieChange {
    const KIND: &'static str = "economy.tie_change";
}

/// Applies a change to the ties of a resident: at once when the resident is
/// in this zone, else by a message that follows the resident.
pub(crate) fn change(realm: &Realm, resident: ResidentId, change: TieChange) {
    match realm.idx_of(resident) {
        Some(idx) => realm.col_mut::<Ties>()[idx].apply(change),
        None => realm.send(Dest::Resident(resident), &change),
    }
}

/// Handles a [`TieChange`] that reached the zone of the resident.
pub(crate) fn on_change(realm: &Realm, idx: Idx, change: TieChange) {
    realm.col_mut::<Ties>()[idx].apply(change);
}
