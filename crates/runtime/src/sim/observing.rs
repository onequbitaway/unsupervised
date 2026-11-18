//! How the simulation thread builds observations: always through
//! [`observe_batch`], and those of the `observe` calls of one batch of
//! calls in one step.

use std::time::Instant;

use unsup_protocol::{Observation, ObserveOpts, ResidentId, SimError};

use super::Sim;
use crate::mailbox::Answer;
use crate::observe::observe_batch;

impl Sim<'_> {
    /// Builds observations in one step and counts the work. The answers
    /// are in the order of `requests`, one for each request.
    fn observe_many(
        &mut self,
        requests: &[(ResidentId, ObserveOpts)],
    ) -> Vec<Result<Observation, SimError>> {
        if requests.is_empty() {
            return Vec::new();
        }
        let start = Instant::now();
        let mut answers = observe_batch(&mut self.world, requests);
        // The callers pair the answers with the requests by position. A
        // request without an answer would leave its caller without one.
        debug_assert_eq!(answers.len(), requests.len(), "one answer for each request");
        answers.resize_with(requests.len(), || Err(SimError::Invalid));
        self.observe_time += start.elapsed();
        self.observe_batches += 1;
        self.observations += requests.len() as u64;
        answers
    }

    /// Answers the `observe` calls of the current batch in one step.
    pub(super) fn answer_observes(&mut self, answers: &mut Vec<Answer>) {
        if self.observes.is_empty() {
            return;
        }
        // The buffers keep their allocation from batch to batch.
        let mut requests = std::mem::take(&mut self.observes);
        let mut replies = std::mem::take(&mut self.observe_replies);
        let observations = self.observe_many(&requests);
        answers.extend(
            replies
                .drain(..)
                .zip(observations)
                .map(|(reply, observation)| Answer::Observation(reply, observation)),
        );
        requests.clear();
        self.observes = requests;
        self.observe_replies = replies;
    }

    /// Builds the observation that a waiting call gets: the news from its
    /// notice cursor on. A cursor that is ahead of the inbox (after a
    /// rollback) counts as "now".
    pub(super) fn observation(
        &mut self,
        resident: ResidentId,
        since: u64,
    ) -> Result<Observation, SimError> {
        let since = since.min(self.world.next_notice(resident)?);
        let opts = ObserveOpts { since_notice: since, ..ObserveOpts::default() };
        self.observe_many(&[(resident, opts)]).pop().unwrap_or(Err(SimError::Invalid))
    }
}
