//! The alert list: problems that an operator must see.

use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use unsup_store::unix_now;

/// One kind of problem, with how often and when it came.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Alert {
    /// A short name of the problem: `audit_failed`, `overload`,
    /// `journal_error`, `snapshot_error`, `simulation_stopped`,
    /// `frames_lost`, `time_dilation`, or a name that the host chose.
    pub kind: String,
    /// What happened the last time, in one or two sentences.
    pub message: String,
    /// How many times the problem was raised.
    pub count: u64,
    /// When it was raised first, in seconds since 1970.
    pub first_at: u64,
    /// When it was raised last, in seconds since 1970.
    pub last_at: u64,
}

/// The alert list of a gateway.
///
/// The list has one entry for each kind of problem. A problem that comes
/// again raises the count of its entry and replaces the message. The
/// gateway raises alerts for what it sees in the statistics of the
/// simulation. The host raises alerts for what only it can see, for
/// example an audit of the world that failed.
#[derive(Debug)]
pub struct Alerts {
    /// Greatest number of kinds in the list.
    max: usize,
    list: Mutex<Vec<Alert>>,
}

impl Alerts {
    pub(crate) fn new(max: usize) -> Alerts {
        Alerts { max: max.max(1), list: Mutex::new(Vec::new()) }
    }

    /// Raises an alert now. It also goes to the log as an error.
    pub fn raise(&self, kind: &str, message: impl Into<String>) {
        self.raise_at(kind, message, unix_now());
    }

    /// Raises an alert at the time `now`, in seconds since 1970.
    pub fn raise_at(&self, kind: &str, message: impl Into<String>, now: u64) {
        let message = message.into();
        tracing::error!(kind, %message, "alert");
        let mut list = self.list.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(alert) = list.iter_mut().find(|alert| alert.kind == kind) {
            alert.message = message;
            alert.count += 1;
            alert.last_at = now;
            return;
        }
        if list.len() >= self.max {
            // The kind that was quiet for the longest time makes room.
            if let Some(oldest) = (0..list.len()).min_by_key(|&index| (list[index].last_at, index))
            {
                list.remove(oldest);
            }
        }
        list.push(Alert { kind: kind.to_string(), message, count: 1, first_at: now, last_at: now });
    }

    /// Returns the alerts, the one that was raised last first.
    #[must_use]
    pub fn list(&self) -> Vec<Alert> {
        let mut list = self.list.lock().unwrap_or_else(PoisonError::into_inner).clone();
        list.sort_by(|a, b| b.last_at.cmp(&a.last_at).then_with(|| a.kind.cmp(&b.kind)));
        list
    }

    /// Removes the alert of one kind. Returns false when there was none.
    pub fn clear(&self, kind: &str) -> bool {
        let mut list = self.list.lock().unwrap_or_else(PoisonError::into_inner);
        let before = list.len();
        list.retain(|alert| alert.kind != kind);
        list.len() < before
    }
}

#[cfg(test)]
mod tests {
    use super::Alerts;

    #[test]
    fn a_problem_that_comes_again_counts_up_in_one_entry() {
        let alerts = Alerts::new(8);
        alerts.raise_at("overload", "3 calls were refused.", 100);
        alerts.raise_at("journal_error", "The disk is full.", 110);
        alerts.raise_at("overload", "5 calls were refused.", 120);
        let list = alerts.list();
        let shown: Vec<_> = list
            .iter()
            .map(|a| (a.kind.as_str(), a.message.as_str(), a.count, a.first_at, a.last_at))
            .collect();
        assert_eq!(
            shown,
            [
                ("overload", "5 calls were refused.", 2, 100, 120),
                ("journal_error", "The disk is full.", 1, 110, 110),
            ]
        );
        assert!(alerts.clear("overload"));
        assert!(!alerts.clear("overload"));
        assert_eq!(alerts.list().len(), 1);
    }

    #[test]
    fn a_full_list_drops_the_kind_that_was_quiet_longest() {
        let alerts = Alerts::new(2);
        alerts.raise_at("a", "first", 1);
        alerts.raise_at("b", "second", 2);
        alerts.raise_at("a", "again", 3);
        alerts.raise_at("c", "third", 4);
        let kinds: Vec<String> = alerts.list().into_iter().map(|alert| alert.kind).collect();
        assert_eq!(kinds, ["c", "a"]);
    }
}
