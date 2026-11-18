//! Turns the statistics of the simulation into alerts.

use unsup_runtime::SimStats;

use super::alerts::Alerts;

/// Remembers the counters of the last look at the simulation, so that
/// each look raises an alert only for what is new.
#[derive(Debug, Default)]
pub(crate) struct SimWatch {
    overloaded: u64,
    journal_errors: u64,
    snapshot_errors: u64,
    dilated: bool,
    stopped: bool,
}

impl SimWatch {
    /// Compares the statistics with those of the last look.
    pub(crate) fn look(&mut self, stats: &SimStats, alerts: &Alerts) {
        self.stopped = false;
        let refused = stats.overloaded.saturating_sub(self.overloaded);
        if refused > 0 {
            alerts.raise(
                "overload",
                format!(
                    "The simulation refused {refused} calls because it had too much work. \
                     It refused {} calls since the start. The mailbox holds {} of {} calls.",
                    stats.overloaded, stats.mailbox_depth, stats.mailbox_capacity
                ),
            );
        }
        self.overloaded = stats.overloaded;

        let lost = stats.journal_errors.saturating_sub(self.journal_errors);
        if lost > 0 {
            alerts.raise(
                "journal_error",
                format!(
                    "The journal could not record {lost} inputs. The world did not apply them. \
                     {} inputs were refused since the start. Look at the disk of the data \
                     directory.",
                    stats.journal_errors
                ),
            );
        }
        self.journal_errors = stats.journal_errors;

        let failed = stats.snapshot_errors.saturating_sub(self.snapshot_errors);
        if failed > 0 {
            alerts.raise(
                "snapshot_error",
                format!(
                    "{failed} snapshots could not be written. The last good snapshot is of \
                     frame {}.",
                    stats.last_snapshot_frame
                ),
            );
        }
        self.snapshot_errors = stats.snapshot_errors;

        let dilated = stats.dilated_zones > 0;
        if dilated && !self.dilated {
            alerts.raise(
                "time_dilation",
                format!(
                    "{} zones run slower than one tick in each frame, because their ticks \
                     take too long.",
                    stats.dilated_zones
                ),
            );
        }
        self.dilated = dilated;
    }

    /// Notes that the simulation does not answer any more. Raises the
    /// alert one time.
    pub(crate) fn stopped(&mut self, alerts: &Alerts) {
        if !self.stopped {
            self.stopped = true;
            alerts.raise(
                "simulation_stopped",
                "The simulation stopped while the server runs. Read the log for the cause, \
                 then start the server again: it recovers the world from the data directory.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use unsup_runtime::SimStats;

    use super::{Alerts, SimWatch};

    #[test]
    fn each_look_raises_an_alert_only_for_what_is_new() {
        let alerts = Alerts::new(16);
        let mut watch = SimWatch::default();
        watch.look(&SimStats::default(), &alerts);
        assert_eq!(alerts.list(), []);

        let busy = SimStats {
            overloaded: 3,
            journal_errors: 2,
            snapshot_errors: 1,
            dilated_zones: 4,
            last_snapshot_frame: 600,
            mailbox_capacity: 16,
            ..SimStats::default()
        };
        watch.look(&busy, &alerts);
        let mut kinds: Vec<(String, u64)> =
            alerts.list().into_iter().map(|alert| (alert.kind, alert.count)).collect();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                ("journal_error".to_string(), 1),
                ("overload".to_string(), 1),
                ("snapshot_error".to_string(), 1),
                ("time_dilation".to_string(), 1),
            ]
        );
        let overload = alerts.list().into_iter().find(|a| a.kind == "overload").expect("an alert");
        assert_eq!(
            overload.message,
            "The simulation refused 3 calls because it had too much work. It refused 3 calls \
             since the start. The mailbox holds 0 of 16 calls."
        );

        // The same numbers again: nothing is new.
        watch.look(&busy, &alerts);
        assert!(alerts.list().iter().all(|alert| alert.count == 1));
        // Two more refused calls: only that alert counts up.
        watch.look(&SimStats { overloaded: 5, ..busy }, &alerts);
        let counts: Vec<u64> = ["overload", "journal_error"]
            .map(|kind| alerts.list().iter().find(|a| a.kind == kind).expect("an alert").count)
            .to_vec();
        assert_eq!(counts, [2, 1]);
    }

    #[test]
    fn a_simulation_that_stopped_is_one_alert_until_it_answers_again() {
        let alerts = Alerts::new(16);
        let mut watch = SimWatch::default();
        watch.stopped(&alerts);
        watch.stopped(&alerts);
        assert_eq!(alerts.list().iter().map(|a| a.count).collect::<Vec<_>>(), [1]);
        watch.look(&SimStats::default(), &alerts);
        watch.stopped(&alerts);
        assert_eq!(alerts.list()[0].count, 2);
    }
}
