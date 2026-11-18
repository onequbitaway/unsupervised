//! The calls that wait for news of a resident.
//!
//! A frame names the residents that got news. The runtime looks up only
//! their waiting calls. The deadlines are kept in order, so the runtime
//! finds the calls whose time is up without a look at the others. Nothing
//! here scans all waiting calls, so the cost of a frame does not grow with
//! their number.

use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

use tokio::sync::oneshot;
use unsup_protocol::{Observation, ResidentId, SimError, WaitFilter};

/// One `wait` call that has not been answered yet.
pub(crate) struct Waiter {
    /// Unique number of the call, for the deadline index.
    id: u64,
    /// What the caller wants to be woken for.
    pub filter: WaitFilter,
    /// News with a sequence number at or above this value counts.
    pub since: u64,
    deadline: Instant,
    /// Where the answer goes.
    pub reply: oneshot::Sender<Result<Observation, SimError>>,
}

/// All waiting calls, by resident and by deadline.
#[derive(Default)]
pub(crate) struct Waiters {
    by_resident: HashMap<ResidentId, Vec<Waiter>>,
    deadlines: BTreeSet<(Instant, u64, ResidentId)>,
    next_id: u64,
    len: usize,
}

impl Waiters {
    /// Returns the number of waiting calls.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Returns the earliest deadline.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        self.deadlines.first().map(|&(deadline, _, _)| deadline)
    }

    /// Makes room for one more call of a resident. Calls of that resident
    /// whose caller went away are dropped. When the resident then still has
    /// `per_resident` calls or more, the oldest ones are returned: the
    /// caller answers them now.
    pub(crate) fn make_room(&mut self, resident: ResidentId, per_resident: usize) -> Vec<Waiter> {
        let Some(list) = self.by_resident.get_mut(&resident) else {
            return Vec::new();
        };
        let before = list.len();
        let deadlines = &mut self.deadlines;
        list.retain(|waiter| {
            let alive = !waiter.reply.is_closed();
            if !alive {
                deadlines.remove(&(waiter.deadline, waiter.id, resident));
            }
            alive
        });
        let room = per_resident.max(1) - 1;
        let extra = list.len().saturating_sub(room);
        let evicted: Vec<Waiter> = list.drain(..extra).collect();
        for waiter in &evicted {
            deadlines.remove(&(waiter.deadline, waiter.id, resident));
        }
        self.len -= before - list.len();
        if list.is_empty() {
            self.by_resident.remove(&resident);
        }
        evicted
    }

    /// Adds a call.
    pub(crate) fn add(
        &mut self,
        resident: ResidentId,
        filter: WaitFilter,
        since: u64,
        deadline: Instant,
        reply: oneshot::Sender<Result<Observation, SimError>>,
    ) {
        let id = self.next_id;
        self.next_id += 1;
        self.by_resident.entry(resident).or_default().push(Waiter {
            id,
            filter,
            since,
            deadline,
            reply,
        });
        self.deadlines.insert((deadline, id, resident));
        self.len += 1;
    }

    /// Takes the waiting calls of one resident. The caller answers some and
    /// gives the others back with [`Waiters::put_back`].
    pub(crate) fn take(&mut self, resident: ResidentId) -> Vec<Waiter> {
        let list = self.by_resident.remove(&resident).unwrap_or_default();
        for waiter in &list {
            self.deadlines.remove(&(waiter.deadline, waiter.id, resident));
        }
        self.len -= list.len();
        list
    }

    /// Gives back calls that [`Waiters::take`] returned and that keep
    /// waiting.
    pub(crate) fn put_back(&mut self, resident: ResidentId, list: Vec<Waiter>) {
        if list.is_empty() {
            return;
        }
        for waiter in &list {
            self.deadlines.insert((waiter.deadline, waiter.id, resident));
        }
        self.len += list.len();
        self.by_resident.entry(resident).or_default().extend(list);
    }

    /// Takes the next call whose deadline is at or before `now`.
    pub(crate) fn pop_expired(&mut self, now: Instant) -> Option<(ResidentId, Waiter)> {
        let &(deadline, id, resident) = self.deadlines.first()?;
        if deadline > now {
            return None;
        }
        self.deadlines.remove(&(deadline, id, resident));
        let list = self.by_resident.get_mut(&resident)?;
        let position = list.iter().position(|waiter| waiter.id == id)?;
        let waiter = list.remove(position);
        if list.is_empty() {
            self.by_resident.remove(&resident);
        }
        self.len -= 1;
        Some((resident, waiter))
    }

    /// Takes every waiting call.
    pub(crate) fn drain(&mut self) -> Vec<(ResidentId, Waiter)> {
        self.deadlines.clear();
        self.len = 0;
        self.by_resident
            .drain()
            .flat_map(|(resident, list)| list.into_iter().map(move |waiter| (resident, waiter)))
            .collect()
    }
}
