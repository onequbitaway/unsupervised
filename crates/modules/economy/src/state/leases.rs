//! The leases of a zone: who rents which home, and what it owes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{ResidentId, Resource, SiteId};

/// One lease: a tenant pays rent each day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lease {
    /// The tenant.
    pub(crate) tenant: ResidentId,
    /// Rent in cents for one day.
    pub(crate) rent: i64,
    /// Rent in cents that is due and not paid.
    pub(crate) owed: i64,
    /// Days in a row on which the tenant did not pay.
    pub(crate) overdue: u32,
    /// True while a request for the rent waits for its answer.
    pub(crate) asked: bool,
}

/// The leases of one zone, by site, and the day of the last daily run.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Leases {
    /// The leases of every site that has one, in the order in which they
    /// started.
    pub(crate) sites: BTreeMap<SiteId, Vec<Lease>>,
}

impl Resource for Leases {
    const NAME: &'static str = "economy.leases";
}

impl Leases {
    /// Returns the lease of a tenant at a site.
    pub(crate) fn get(&self, site: SiteId, tenant: ResidentId) -> Option<&Lease> {
        self.sites.get(&site).and_then(|list| list.iter().find(|l| l.tenant == tenant))
    }

    /// Returns the lease of a tenant at a site for a change.
    pub(crate) fn get_mut(&mut self, site: SiteId, tenant: ResidentId) -> Option<&mut Lease> {
        self.sites.get_mut(&site).and_then(|list| list.iter_mut().find(|l| l.tenant == tenant))
    }

    /// Adds a lease.
    pub(crate) fn add(&mut self, site: SiteId, lease: Lease) {
        self.sites.entry(site).or_default().push(lease);
    }

    /// Removes the lease of a tenant at a site and returns it.
    pub(crate) fn remove(&mut self, site: SiteId, tenant: ResidentId) -> Option<Lease> {
        let list = self.sites.get_mut(&site)?;
        let at = list.iter().position(|l| l.tenant == tenant)?;
        let lease = list.remove(at);
        if list.is_empty() {
            self.sites.remove(&site);
        }
        Some(lease)
    }

    /// Returns every lease as `(site, tenant)`, ordered by site and then by
    /// the start of the lease.
    pub(crate) fn all(&self) -> Vec<(SiteId, ResidentId)> {
        self.sites
            .iter()
            .flat_map(|(&site, list)| list.iter().map(move |lease| (site, lease.tenant)))
            .collect()
    }
}
