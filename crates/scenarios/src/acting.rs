//! Verbs: a resident sends a request and the world runs until the report
//! is there.

use serde_json::Value;
use unsup_kernel::protocol::{ActionAck, ActionReport, ActionStatus, ObserveOpts};
use unsup_kernel::{ActionError, ErrorCode, ResidentId};

use crate::game::Game;

/// Frames that a helper waits for the report of an action. The longest
/// action of the game, a walk, times out before that.
const REPORT_FRAMES: u64 = 3_000;

impl Game {
    /// Sends a request and returns the acknowledgement. No frame runs.
    pub fn submit(&mut self, r: ResidentId, verb: &str, args: Value) -> ActionAck {
        let ack = self.world.act(r, verb, args);
        if matches!(ack.status, ActionStatus::Done { .. }) {
            self.completed.insert(verb.to_string());
        }
        ack
    }

    /// Returns the report of an action when the inbox of the resident has
    /// it.
    fn report_of(&self, r: ResidentId, since: u64, action: u64) -> Option<ActionReport> {
        let opts = ObserveOpts {
            since_notice: since,
            sections: Some(vec!["reports".to_string()]),
            ..ObserveOpts::default()
        };
        let observation = self.world.observe_with(r, &opts);
        let reports = observation.sections.get("reports")?.as_array()?;
        reports
            .iter()
            .filter_map(|report| serde_json::from_value::<ActionReport>(report.clone()).ok())
            .find(|report| report.action_id == action)
    }

    /// Reads the outcome of an action from its report, when the report is
    /// there.
    fn reported(
        &mut self,
        r: ResidentId,
        since: u64,
        ack: &ActionAck,
    ) -> Option<Result<Value, ActionError>> {
        let report = self.report_of(r, since, ack.action_id)?;
        Some(match report.error {
            None if report.ok => {
                self.completed.insert(ack.verb.clone());
                Ok(report.result)
            }
            None => Err(ActionError::internal("The report has no error.")),
            Some(error) => Err(error),
        })
    }

    /// Runs frames until an action that was sent has ended. Returns its
    /// result or its error.
    ///
    /// `since` is the notice cursor of the resident before the request.
    ///
    /// # Panics
    ///
    /// Panics when no report comes.
    pub fn outcome(
        &mut self,
        r: ResidentId,
        since: u64,
        ack: ActionAck,
    ) -> Result<Value, ActionError> {
        match ack.status {
            ActionStatus::Done { result } => return Ok(result),
            ActionStatus::Rejected { error } => return Err(error),
            ActionStatus::Started { .. } | ActionStatus::Pending | ActionStatus::Queued { .. } => {}
        }
        for _ in 0..REPORT_FRAMES {
            self.step(1);
            if let Some(outcome) = self.reported(r, since, &ack) {
                return outcome;
            }
        }
        panic!("`{}` of resident {r} wrote no report in {REPORT_FRAMES} frames", ack.verb);
    }

    /// Sends one request for each of several residents in the same frame,
    /// and runs the world until every action has ended. Returns the
    /// results in the order of the requests.
    ///
    /// # Panics
    ///
    /// Panics when an action is refused or fails, or when a report does
    /// not come.
    pub fn act_all(&mut self, requests: &[(ResidentId, &str, Value)]) -> Vec<Value> {
        let mut open = Vec::new();
        let mut results: Vec<Option<Value>> = vec![None; requests.len()];
        for (index, (r, verb, args)) in requests.iter().enumerate() {
            let since = self.cursor(*r);
            let ack = self.submit(*r, verb, args.clone());
            match ack.status {
                ActionStatus::Done { result } => results[index] = Some(result),
                ActionStatus::Rejected { error } => panic!(
                    "`{verb}` {args} of resident {r} was refused: {} ({:?})",
                    error.message, error.code
                ),
                _ => open.push((index, *r, since, ack)),
            }
        }
        for _ in 0..REPORT_FRAMES {
            if open.is_empty() {
                break;
            }
            self.step(1);
            let mut still = Vec::new();
            for (index, r, since, ack) in open {
                match self.reported(r, since, &ack) {
                    Some(Ok(result)) => results[index] = Some(result),
                    Some(Err(error)) => panic!(
                        "`{}` of resident {r} failed at frame {}: {} ({:?})",
                        ack.verb,
                        self.frame(),
                        error.message,
                        error.code
                    ),
                    None => still.push((index, r, since, ack)),
                }
            }
            open = still;
        }
        assert!(open.is_empty(), "{} actions wrote no report", open.len());
        results.into_iter().flatten().collect()
    }

    /// Sends the same request for several residents in the same frame and
    /// runs the world until every action has ended.
    pub fn each(&mut self, residents: &[ResidentId], verb: &str, args: Value) -> Vec<Value> {
        let requests: Vec<_> = residents.iter().map(|r| (*r, verb, args.clone())).collect();
        self.act_all(&requests)
    }

    /// Returns the notice cursor of a resident: the sequence number of its
    /// next notice or report.
    #[must_use]
    pub fn cursor(&self, r: ResidentId) -> u64 {
        match self.world.world.next_notice(r) {
            Ok(cursor) => cursor,
            Err(e) => panic!("resident {r} has no inbox: {e}"),
        }
    }

    /// Sends a request and runs the world until the action has ended.
    /// Returns its result or its error.
    pub fn try_act(
        &mut self,
        r: ResidentId,
        verb: &str,
        args: Value,
    ) -> Result<Value, ActionError> {
        let since = self.cursor(r);
        let ack = self.submit(r, verb, args);
        self.outcome(r, since, ack)
    }

    /// Sends a request that must end well, runs the world until the action
    /// has ended and returns its result.
    ///
    /// # Panics
    ///
    /// Panics when the action is refused or fails.
    pub fn act(&mut self, r: ResidentId, verb: &str, args: Value) -> Value {
        match self.try_act(r, verb, args.clone()) {
            Ok(result) => result,
            Err(error) => panic!(
                "`{verb}` {args} of resident {r} failed at frame {}: {} ({:?}) {}",
                self.frame(),
                error.message,
                error.code,
                error.details
            ),
        }
    }

    /// Sends a request that must fail, at once or in its report, and
    /// returns the error code.
    ///
    /// # Panics
    ///
    /// Panics when the action ends well.
    pub fn refused(&mut self, r: ResidentId, verb: &str, args: Value) -> ErrorCode {
        match self.try_act(r, verb, args.clone()) {
            Ok(result) => panic!("`{verb}` {args} of resident {r} ended well: {result}"),
            Err(error) => error.code,
        }
    }

    /// Returns the verbs of the catalogue that the conditions of a resident
    /// do not block, in the order of their names.
    ///
    /// The helper sends every verb with empty arguments in one frame. The
    /// kernel checks the conditions first, so a verb is blocked exactly
    /// when the answer is the error `blocked`. Every other answer means
    /// that the conditions permit the verb: wrong arguments, a full queue,
    /// too many free actions in the tick, or an action that starts.
    ///
    /// `cancel` comes last, one frame later, because it stops what the
    /// other probes started: the helper leaves the resident idle with an
    /// empty queue.
    /// Use it on a resident whose state can change: an allowed verb with
    /// empty arguments can run.
    pub fn allowed_verbs(&mut self, r: ResidentId) -> Vec<String> {
        let blocked = |status: &ActionStatus| matches!(status, ActionStatus::Rejected { error } if error.code == ErrorCode::Blocked);
        let mut names: Vec<String> =
            self.world.world.catalogue().verbs.into_iter().map(|verb| verb.verb).collect();
        names.sort();
        let mut allowed = Vec::new();
        for name in names.iter().filter(|name| *name != "cancel") {
            let ack = self.world.act(r, name, serde_json::json!({}));
            if !blocked(&ack.status) {
                allowed.push(name.clone());
            }
        }
        // A new tick, so that the limit of free actions for each tick does
        // not refuse the `cancel`.
        self.step(1);
        let ack = self.world.act(r, "cancel", serde_json::json!({}));
        if !blocked(&ack.status) {
            allowed.push("cancel".to_string());
        }
        allowed.sort();
        self.step(1);
        allowed
    }
}
