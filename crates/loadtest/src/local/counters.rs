//! Counts of what the residents did.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{ActionAck, ActionStatus};

use crate::stats::{self, Counts};

/// Requests and rejections of one verb.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbCounts {
    /// Requests that the world answered.
    pub sent: u64,
    /// Requests that the world rejected.
    pub rejected: u64,
}

/// Counts of the actions that a driver sent to a world.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionCounts {
    /// Requests given to `World::apply`.
    pub sent: u64,
    /// Acknowledged with `done`: the action finished at once.
    pub done: u64,
    /// Acknowledged with `started`: the resident is busy.
    pub started: u64,
    /// Acknowledged with `pending`: the action waits for the global realm.
    pub pending: u64,
    /// Acknowledged with `queued`: the resident was busy.
    pub queued: u64,
    /// Rejected by the world. Nothing changed.
    pub rejected: u64,
    /// Rejections by error code.
    pub rejected_by_code: Counts,
    /// Requests that the simulation refused before the action engine, by
    /// error name (for example `unknown_resident`).
    pub refused_by_sim: Counts,
    /// Requests and rejections by verb.
    pub by_verb: BTreeMap<String, VerbCounts>,
    /// Turns in which a resident found no action that it could take.
    pub idle_turns: u64,
    /// Turns in which a resident was still busy.
    pub busy_turns: u64,
    /// Residents that were no longer in the world at their turn.
    pub gone: u64,
    /// Observations that residents made.
    pub observes: u64,
}

impl ActionCounts {
    /// Returns the number of requests that the world accepted.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.done + self.started + self.pending + self.queued
    }

    /// Counts one acknowledgement.
    pub fn record(&mut self, ack: &ActionAck) {
        self.sent += 1;
        let rejected = match &ack.status {
            ActionStatus::Done { .. } => {
                self.done += 1;
                false
            }
            ActionStatus::Started { .. } => {
                self.started += 1;
                false
            }
            ActionStatus::Pending => {
                self.pending += 1;
                false
            }
            ActionStatus::Queued { .. } => {
                self.queued += 1;
                false
            }
            ActionStatus::Rejected { error } => {
                self.rejected += 1;
                stats::bump(&mut self.rejected_by_code, error.code.as_str(), 1);
                true
            }
        };
        let verb = match self.by_verb.get_mut(&ack.verb) {
            Some(verb) => verb,
            None => self.by_verb.entry(ack.verb.clone()).or_default(),
        };
        verb.sent += 1;
        verb.rejected += u64::from(rejected);
    }

    /// Adds the counts of `other`.
    pub fn add(&mut self, other: &ActionCounts) {
        self.sent += other.sent;
        self.done += other.done;
        self.started += other.started;
        self.pending += other.pending;
        self.queued += other.queued;
        self.rejected += other.rejected;
        stats::merge(&mut self.rejected_by_code, &other.rejected_by_code);
        stats::merge(&mut self.refused_by_sim, &other.refused_by_sim);
        for (verb, counts) in &other.by_verb {
            let mine = self.by_verb.entry(verb.clone()).or_default();
            mine.sent += counts.sent;
            mine.rejected += counts.rejected;
        }
        self.idle_turns += other.idle_turns;
        self.busy_turns += other.busy_turns;
        self.gone += other.gone;
        self.observes += other.observes;
    }
}
