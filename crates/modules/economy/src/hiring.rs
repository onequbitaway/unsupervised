//! Ending jobs: one worker leaves, a job closes, a site loses all its jobs.

use serde_json::json;
use unsup_kernel::{Pos, Realm, ResidentId, SiteId};

use crate::board;
use crate::events;
use crate::state::jobs::JobBook;
use crate::state::ties::{self, TieChange};

/// Notice to a worker whose job ended without its own action.
pub const NOTICE_JOB_ENDED: &str = "economy.job_ended";

/// Returns the centre of a site, or the origin when the site is gone.
pub(crate) fn site_pos(realm: &Realm, site: SiteId) -> Pos {
    realm.sites().get(site).map_or(Pos::new(0, 0), |s| s.rect.centre())
}

/// Tells a worker, its ties and the viewers that its job ended. `reason`
/// is `quit`, `fired`, `closed` or `gone`.
fn released(realm: &Realm, job: u64, site: SiteId, resident: ResidentId, reason: &str) {
    ties::change(realm, resident, TieChange::JobEnded { job });
    if reason == "fired" || reason == "closed" {
        realm.notify(
            resident,
            NOTICE_JOB_ENDED,
            None,
            json!({ "job": job, "site": site, "reason": reason }),
        );
    }
    events::worker_left(realm, resident, site_pos(realm, site), site, job, reason);
}

/// Takes one worker out of a job. Returns true when the resident was a
/// worker of the job.
pub(crate) fn remove_worker(realm: &Realm, job: u64, resident: ResidentId, reason: &str) -> bool {
    let site = {
        let mut book = realm.res_mut::<JobBook>();
        let Some(entry) = book.jobs.get_mut(&job) else {
            return false;
        };
        let before = entry.workers.len();
        entry.workers.retain(|worker| *worker != resident);
        if entry.workers.len() == before {
            return false;
        }
        entry.site
    };
    released(realm, job, site, resident, reason);
    board::touch(realm);
    true
}

/// Closes a job and releases its workers. Returns how many workers it had,
/// or `None` when the zone has no such job.
pub(crate) fn close_job(realm: &Realm, job: u64, reason: &str) -> Option<usize> {
    let entry = realm.res_mut::<JobBook>().jobs.remove(&job)?;
    for &worker in &entry.workers {
        released(realm, job, entry.site, worker, reason);
    }
    board::touch(realm);
    Some(entry.workers.len())
}

/// Closes every job at a site.
pub(crate) fn close_site_jobs(realm: &Realm, site: SiteId, reason: &str) {
    let ids = realm.res::<JobBook>().ids_at(site);
    for job in ids {
        close_job(realm, job, reason);
    }
}

/// Takes a resident that left the world out of every job of the zone.
pub(crate) fn remove_everywhere(realm: &Realm, resident: ResidentId) {
    let jobs: Vec<u64> = realm
        .res::<JobBook>()
        .jobs
        .values()
        .filter(|job| job.workers.contains(&resident))
        .map(|job| job.id)
        .collect();
    for job in jobs {
        remove_worker(realm, job, resident, "gone");
    }
}
