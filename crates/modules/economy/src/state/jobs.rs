//! The job book of a zone: every open job at the sites of the zone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{ResidentId, Resource, SiteId};

/// One job: a wage for a shift at a site, for a number of workers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Job {
    /// Id of the job.
    pub(crate) id: u64,
    /// The site where the shifts are.
    pub(crate) site: SiteId,
    /// Wage in cents for one shift, before income tax.
    pub(crate) wage: i64,
    /// How many workers the job takes.
    pub(crate) slots: u32,
    /// Least level in the skill of the job.
    pub(crate) min_level: i64,
    /// What a shift does: a good to gather or a recipe. `None` is the
    /// default of the site.
    pub(crate) task: Option<String>,
    /// The workers, in the order in which they were hired.
    pub(crate) workers: Vec<ResidentId>,
}

impl Job {
    /// Returns how many workers the job can still take.
    pub(crate) fn free_slots(&self) -> u32 {
        let taken = u32::try_from(self.workers.len()).unwrap_or(u32::MAX);
        self.slots.saturating_sub(taken)
    }
}

/// The jobs of one zone, by id.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct JobBook {
    /// Every job of the zone.
    pub(crate) jobs: BTreeMap<u64, Job>,
}

impl Resource for JobBook {
    const NAME: &'static str = "economy.jobs";
}

impl JobBook {
    /// Returns the jobs at a site, ordered by id.
    pub(crate) fn at_site(&self, site: SiteId) -> impl Iterator<Item = &Job> + '_ {
        self.jobs.values().filter(move |job| job.site == site)
    }

    /// Returns the job that a resident has at a site.
    pub(crate) fn job_of(&self, site: SiteId, resident: ResidentId) -> Option<&Job> {
        self.at_site(site).find(|job| job.workers.contains(&resident))
    }

    /// Returns the ids of the jobs at a site, ordered.
    pub(crate) fn ids_at(&self, site: SiteId) -> Vec<u64> {
        self.at_site(site).map(|job| job.id).collect()
    }
}
