//! Counts of what the residents did.
//!
//! A request has two answers. The acknowledgement comes at once: the world
//! accepted the request or rejected it. An accepted action that takes time
//! ends later with a report: it finished or it failed. It fails when the
//! world refuses it at its turn in the queue, or when it stops before its
//! end. Both refusals are data of a run, not failures of the tool.

use std::collections::BTreeMap;
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{ActionAck, ActionStatus, Observation};

use crate::report::thousands;
use crate::stats::{self, Counts};

/// Requests, rejections and reports of one verb.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerbCounts {
    /// Requests that the world answered.
    pub sent: u64,
    /// Requests that the world rejected in the acknowledgement.
    pub rejected: u64,
    /// Rejections by error code.
    #[serde(default)]
    pub rejected_by_code: Counts,
    /// Reports that residents read: the action finished.
    #[serde(default)]
    pub finished: u64,
    /// Reports that residents read: the world accepted the action, and it
    /// failed later.
    #[serde(default)]
    pub failed: u64,
    /// Failed reports by error code.
    #[serde(default)]
    pub failed_by_code: Counts,
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
    /// Acknowledged with `pending`: the action waits for the global realm
    /// or for another zone.
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
    /// Requests, rejections and reports by verb.
    pub by_verb: BTreeMap<String, VerbCounts>,
    /// Reports of ended actions that residents read in their observations:
    /// the action finished.
    #[serde(default)]
    pub reports_finished: u64,
    /// Reports that residents read: the world accepted the action, and it
    /// failed later.
    #[serde(default)]
    pub reports_failed: u64,
    /// Failed reports by error code.
    #[serde(default)]
    pub failed_by_code: Counts,
    /// Actions that were not sent because the world rejected the walk to
    /// their site. A plan stops at the first rejection.
    #[serde(default)]
    pub not_sent: u64,
    /// Turns in which a resident found no action that it could take.
    pub idle_turns: u64,
    /// Turns in which a resident was still busy.
    pub busy_turns: u64,
    /// Residents that were no longer in the world at their turn.
    pub gone: u64,
    /// Observations that residents made.
    pub observes: u64,
}

/// Returns `part` of `whole` in percent, or 0 for an empty whole.
fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 }
}

impl ActionCounts {
    /// Returns the number of requests that the world accepted.
    #[must_use]
    pub fn accepted(&self) -> u64 {
        self.done + self.started + self.pending + self.queued
    }

    /// Returns the share of the requests that the world rejected in the
    /// acknowledgement, from 0 to 1.
    #[must_use]
    pub fn rejection_rate(&self) -> f64 {
        percent(self.rejected, self.sent) / 100.0
    }

    fn verb(&mut self, name: &str) -> &mut VerbCounts {
        self.by_verb.entry(name.to_string()).or_default()
    }

    /// Counts one acknowledgement.
    pub fn record(&mut self, ack: &ActionAck) {
        self.sent += 1;
        let rejected = match &ack.status {
            ActionStatus::Done { .. } => {
                self.done += 1;
                None
            }
            ActionStatus::Started { .. } => {
                self.started += 1;
                None
            }
            ActionStatus::Pending => {
                self.pending += 1;
                None
            }
            ActionStatus::Queued { .. } => {
                self.queued += 1;
                None
            }
            ActionStatus::Rejected { error } => {
                self.rejected += 1;
                stats::bump(&mut self.rejected_by_code, error.code.as_str(), 1);
                Some(error.code.as_str())
            }
        };
        let verb = match self.by_verb.get_mut(&ack.verb) {
            Some(verb) => verb,
            None => self.by_verb.entry(ack.verb.clone()).or_default(),
        };
        verb.sent += 1;
        if let Some(code) = rejected {
            verb.rejected += 1;
            stats::bump(&mut verb.rejected_by_code, code, 1);
        }
    }

    /// Counts the end of one action, from its report. `failed` is the
    /// error code when the report says that the action failed.
    pub fn record_end(&mut self, verb: &str, failed: Option<&str>) {
        match failed {
            None => {
                self.reports_finished += 1;
                self.verb(verb).finished += 1;
            }
            Some(code) => {
                self.reports_failed += 1;
                stats::bump(&mut self.failed_by_code, code, 1);
                let verb = self.verb(verb);
                verb.failed += 1;
                stats::bump(&mut verb.failed_by_code, code, 1);
            }
        }
    }

    /// Counts the reports of ended actions in an observation.
    pub fn record_reports(&mut self, observation: &Observation) {
        let reports = observation.sections.get("reports").and_then(Value::as_array);
        for report in reports.into_iter().flatten() {
            let verb = report.get("verb").and_then(Value::as_str).unwrap_or("unknown");
            if report.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                self.record_end(verb, None);
            } else {
                let code = report
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                self.record_end(verb, Some(code));
            }
        }
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
            stats::merge(&mut mine.rejected_by_code, &counts.rejected_by_code);
            mine.finished += counts.finished;
            mine.failed += counts.failed;
            stats::merge(&mut mine.failed_by_code, &counts.failed_by_code);
        }
        self.reports_finished += other.reports_finished;
        self.reports_failed += other.reports_failed;
        stats::merge(&mut self.failed_by_code, &other.failed_by_code);
        self.not_sent += other.not_sent;
        self.idle_turns += other.idle_turns;
        self.busy_turns += other.busy_turns;
        self.gone += other.gone;
        self.observes += other.observes;
    }

    /// Returns the lines of a summary that give the refusals: the rate of
    /// rejections by error code, the reports that residents read, and one
    /// line for each verb.
    #[must_use]
    pub fn refusal_lines(&self) -> String {
        let mut out = String::new();
        // Writing to a `String` cannot fail.
        let _ = writeln!(
            out,
            "  rejected at once: {} of {} requests ({:.2} %){}",
            thousands(self.rejected),
            thousands(self.sent),
            percent(self.rejected, self.sent),
            codes(&self.rejected_by_code, self.sent),
        );
        let reports = self.reports_finished + self.reports_failed;
        let _ = writeln!(
            out,
            "  reports read: {} ended actions, {} failed ({:.2} %){}",
            thousands(reports),
            thousands(self.reports_failed),
            percent(self.reports_failed, reports),
            codes(&self.failed_by_code, reports),
        );
        if self.not_sent > 0 {
            let _ = writeln!(
                out,
                "  not sent: {} actions whose walk to the site was rejected",
                thousands(self.not_sent)
            );
        }
        for (name, verb) in &self.by_verb {
            let ended = verb.finished + verb.failed;
            let _ = writeln!(
                out,
                "    {name:<14} {:>11} sent  {:>6.2} % rejected{}  {:>9} reports  {:>6.2} % failed{}",
                thousands(verb.sent),
                percent(verb.rejected, verb.sent),
                codes(&verb.rejected_by_code, verb.sent),
                thousands(ended),
                percent(verb.failed, ended),
                codes(&verb.failed_by_code, ended),
            );
        }
        out
    }
}

/// Writes counts by code as rates of `whole`, for example
/// ` [insufficient 12.50 %, not_here 0.10 %]`. Empty counts give no text.
fn codes(counts: &Counts, whole: u64) -> String {
    if counts.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(code, &count)| format!("{code} {:.2} %", percent(count, whole)))
        .collect();
    format!(" [{}]", parts.join(", "))
}

#[cfg(test)]
mod tests {
    use unsup_protocol::{ActionError, ErrorCode, ResidentId, Tick};

    use super::*;

    fn ack(verb: &str, status: ActionStatus) -> ActionAck {
        ActionAck { request_id: None, action_id: 1, verb: verb.into(), status }
    }

    fn rejected(code: ErrorCode) -> ActionStatus {
        ActionStatus::Rejected {
            error: ActionError { code, message: String::new(), details: Value::Null },
        }
    }

    #[test]
    fn rejections_and_reports_are_counted_by_code_and_by_verb() {
        let mut counts = ActionCounts::default();
        counts.record(&ack("work", ActionStatus::Started { ends_at: Tick(9), ticks: 9 }));
        counts.record(&ack("work", rejected(ErrorCode::Insufficient)));
        counts.record(&ack("work", rejected(ErrorCode::NotHere)));
        counts.record(&ack("say", ActionStatus::Done { result: Value::Null }));
        assert_eq!((counts.sent, counts.rejected, counts.accepted()), (4, 2, 2));
        assert!((counts.rejection_rate() - 0.5).abs() < 1e-12);
        let work = &counts.by_verb["work"];
        assert_eq!((work.sent, work.rejected), (3, 2));
        assert_eq!(work.rejected_by_code["insufficient"], 1);
        assert_eq!(work.rejected_by_code["not_here"], 1);
        assert_eq!(counts.by_verb["say"].rejected, 0);

        let observation = Observation {
            frame: 1,
            tick: Tick(1),
            clock: Default::default(),
            resident: ResidentId(1),
            sections: BTreeMap::from([(
                "reports".to_string(),
                serde_json::json!([
                    { "verb": "work", "ok": true },
                    { "verb": "work", "ok": false, "error": { "code": "interrupted" } },
                    { "verb": "move_to", "ok": false, "error": { "code": "timeout" } },
                ]),
            )]),
            notices: Vec::new(),
            next_notice: 4,
        };
        counts.record_reports(&observation);
        assert_eq!((counts.reports_finished, counts.reports_failed), (1, 2));
        assert_eq!(counts.failed_by_code["interrupted"], 1);
        assert_eq!(counts.failed_by_code["timeout"], 1);
        assert_eq!((counts.by_verb["work"].finished, counts.by_verb["work"].failed), (1, 1));
        assert_eq!(counts.by_verb["move_to"].failed_by_code["timeout"], 1);

        let lines = counts.refusal_lines();
        assert!(
            lines.contains("rejected at once: 2 of 4 requests (50.00 %) [insufficient 25.00 %, not_here 25.00 %]"),
            "{lines}"
        );
        assert!(lines.contains("reports read: 3 ended actions, 2 failed (66.67 %)"), "{lines}");

        let mut sum = ActionCounts::default();
        sum.add(&counts);
        sum.add(&counts);
        assert_eq!(sum.sent, 8);
        assert_eq!(sum.by_verb["work"].rejected_by_code["insufficient"], 2);
        assert_eq!(sum.failed_by_code["timeout"], 2);
    }
}
