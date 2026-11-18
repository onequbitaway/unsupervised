//! The limit of sponsor missions for one owner.

use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use unsup_protocol::OwnerId;

/// The time in which the limit counts: one day.
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// One place that an owner took in its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    owner: OwnerId,
    at: Instant,
}

/// Counts the missions of each owner in the last 24 hours.
///
/// The count lives in memory, like every rate limit of the gateway: a
/// restart of the server empties it.
#[derive(Debug)]
pub(crate) struct Quota {
    /// Missions of one owner in 24 hours. 0: no limit.
    per_day: u32,
    /// The times of the places that each owner holds, oldest first.
    used: Mutex<BTreeMap<OwnerId, Vec<Instant>>>,
}

impl Quota {
    pub(crate) fn new(per_day: u32) -> Quota {
        Quota { per_day, used: Mutex::new(BTreeMap::new()) }
    }

    /// Returns the limit: missions of one owner in 24 hours.
    pub(crate) fn per_day(&self) -> u32 {
        self.per_day
    }

    /// Takes one place for an owner at the time `now`. The place counts
    /// for 24 hours, or until [`Quota::release`].
    ///
    /// # Errors
    ///
    /// When the owner holds all its places: the time until the oldest one
    /// is free again.
    pub(crate) fn reserve(&self, owner: OwnerId, now: Instant) -> Result<Slot, Duration> {
        let slot = Slot { owner, at: now };
        if self.per_day == 0 {
            return Ok(slot);
        }
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        // The places of all owners that are older than a day are free.
        used.retain(|_, places| {
            places.retain(|at| now.saturating_duration_since(*at) < DAY);
            !places.is_empty()
        });
        let places = used.entry(owner).or_default();
        if places.len() >= self.per_day as usize {
            let oldest = places.first().copied().unwrap_or(now);
            return Err(DAY.saturating_sub(now.saturating_duration_since(oldest)));
        }
        places.push(now);
        Ok(slot)
    }

    /// Gives a place back: its mission was not published.
    pub(crate) fn release(&self, slot: Slot) {
        let mut used = self.used.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(places) = used.get_mut(&slot.owner) {
            if let Some(index) = places.iter().position(|at| *at == slot.at) {
                places.remove(index);
            }
            if places.is_empty() {
                used.remove(&slot.owner);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADA: OwnerId = OwnerId(1);
    const BOB: OwnerId = OwnerId(2);

    #[test]
    fn an_owner_has_its_places_for_one_day() {
        let quota = Quota::new(2);
        let start = Instant::now();
        let hour = Duration::from_secs(3_600);
        quota.reserve(ADA, start).expect("the first place");
        quota.reserve(ADA, start + hour).expect("the second place");
        // The third mission waits until the first place is a day old.
        assert_eq!(quota.reserve(ADA, start + 2 * hour), Err(22 * hour));
        assert_eq!(
            quota.reserve(ADA, start + DAY - Duration::from_secs(1)),
            Err(Duration::from_secs(1))
        );
        // Another owner has its own places.
        quota.reserve(BOB, start + 2 * hour).expect("a place of another owner");
        // After a day the first place is free. The second one is not.
        quota.reserve(ADA, start + DAY).expect("the first place is free again");
        assert_eq!(quota.reserve(ADA, start + DAY), Err(hour));
        // A place that is given back is free at once.
        let quota = Quota::new(1);
        let slot = quota.reserve(ADA, start).expect("a place");
        assert_eq!(quota.reserve(ADA, start), Err(DAY));
        quota.release(slot);
        quota.reserve(ADA, start).expect("the place was given back");
    }

    #[test]
    fn a_limit_of_zero_counts_nothing() {
        let quota = Quota::new(0);
        let now = Instant::now();
        for _ in 0..100 {
            let slot = quota.reserve(ADA, now).expect("no limit");
            quota.release(slot);
        }
        assert!(quota.used.lock().expect("the lock").is_empty());
    }
}
