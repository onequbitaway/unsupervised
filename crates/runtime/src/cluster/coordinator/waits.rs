//! The `wait` calls of a cluster.
//!
//! The coordinator keeps the calls that wait. A frame names the residents
//! that got news. For those that have a waiting call, the coordinator asks
//! the worker of each resident for the observation, one question per
//! worker and frame.

use std::time::{Duration, Instant};

use unsup_protocol::{Observation, ResidentId, SimError, WaitFilter};

use super::super::error::ClusterError;
use super::serve::Server;
use crate::mailbox::Reply;
use crate::sim::has_news;
use crate::waiters::Waiter;

impl Server {
    /// Answers the waiting calls of the residents that got news, when
    /// their filter lets the news through. The others keep waiting.
    pub(super) fn wake(&mut self, woke: &[ResidentId]) -> Result<(), ClusterError> {
        let mut waiting: Vec<(ResidentId, Waiter)> = Vec::new();
        for &resident in woke {
            waiting.extend(self.waiters.take(resident).into_iter().map(|w| (resident, w)));
        }
        if waiting.is_empty() {
            return Ok(());
        }
        let items: Vec<(ResidentId, u64)> = waiting.iter().map(|(r, w)| (*r, w.since)).collect();
        let observations = self.core.observe_many(&items)?;
        let mut still: Vec<(ResidentId, Vec<Waiter>)> = Vec::new();
        for ((resident, waiter), answer) in waiting.into_iter().zip(observations) {
            match answer {
                Ok(observation) if !has_news(&observation, &waiter.filter) => {
                    match still.last_mut() {
                        Some((last, list)) if *last == resident => list.push(waiter),
                        _ => still.push((resident, vec![waiter])),
                    }
                }
                answer => {
                    // The caller may have gone away. That is fine.
                    let _ = waiter.reply.send(answer);
                }
            }
        }
        for (resident, list) in still {
            self.waiters.put_back(resident, list);
        }
        Ok(())
    }

    /// Answers the given calls with whatever arrived since their cursor.
    fn answer_now(&mut self, calls: Vec<(ResidentId, Waiter)>) -> Result<(), ClusterError> {
        if calls.is_empty() {
            return Ok(());
        }
        let items: Vec<(ResidentId, u64)> = calls.iter().map(|(r, w)| (*r, w.since)).collect();
        let observations = self.core.observe_many(&items)?;
        for ((_, waiter), answer) in calls.into_iter().zip(observations) {
            let _ = waiter.reply.send(answer);
        }
        Ok(())
    }

    /// Answers every waiting call whose time is up.
    pub(super) fn expire(&mut self, now: Instant) -> Result<(), ClusterError> {
        let mut expired = Vec::new();
        while let Some(call) = self.waiters.pop_expired(now) {
            expired.push(call);
        }
        self.answer_now(expired)
    }

    pub(super) fn start_wait(
        &mut self,
        resident: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
        reply: Reply<Observation>,
    ) -> Result<(), ClusterError> {
        let peeked = match self.core.peek(resident, since, timeout.is_zero())? {
            Ok(peeked) => peeked,
            Err(error) => {
                let _ = reply.send(Err(error));
                return Ok(());
            }
        };
        // With an older cursor from the caller, news may be there already.
        if let Some(observation) = peeked.observation
            && (has_news(&observation, &filter) || timeout.is_zero())
        {
            let _ = reply.send(Ok(observation));
            return Ok(());
        }
        // A resident has too many calls that wait: the oldest ones end now,
        // as if their time were up.
        let evicted = self.waiters.make_room(resident, self.max_waiters_per_resident);
        self.answer_now(evicted.into_iter().map(|waiter| (resident, waiter)).collect())?;
        if self.waiters.len() >= self.max_waiters {
            self.meters.waiters_full += 1;
            let _ = reply.send(Err(SimError::Overloaded));
            return Ok(());
        }
        self.waiters.add(resident, filter, peeked.since, Instant::now() + timeout, reply);
        Ok(())
    }
}
