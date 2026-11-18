//! Numbers about the running simulation, and the meters behind them.

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Numbers about the running simulation.
///
/// Counters count since the start of the runtime. Times are in
/// microseconds. A rollback does not reset them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimStats {
    /// Number of the last frame.
    pub frame: u64,
    /// Residents in the zones that this process hosts.
    pub residents: u64,
    /// Inputs applied since the start, frames included.
    pub inputs_applied: u64,
    /// Frames that this runtime ran since the start.
    pub frames_run: u64,
    /// Mean time of a frame in microseconds, since the start.
    pub mean_frame_micros: u64,
    /// Longest time of a frame in microseconds, since the start.
    pub worst_frame_micros: u64,
    /// Time of the last frame in microseconds.
    pub last_frame_micros: u64,
    /// Calls to [`SimHandle::wait`](crate::SimHandle::wait) that wait now.
    pub waiting: u64,
    /// Inputs that were refused because the journal could not record them.
    pub journal_errors: u64,

    /// Median frame time over the last frames (see
    /// [`RuntimeConfig::stats_window`](crate::RuntimeConfig::stats_window)).
    pub p50_frame_micros: u64,
    /// Frame time that 95 of 100 of the last frames stayed below.
    pub p95_frame_micros: u64,
    /// Frame time that 99 of 100 of the last frames stayed below.
    pub p99_frame_micros: u64,
    /// Inputs applied per second in the last full measuring window.
    pub inputs_per_second: u64,
    /// Calls that wait in the mailbox of the simulation thread now.
    pub mailbox_depth: u64,
    /// Greatest number of calls that the mailbox holds.
    pub mailbox_capacity: u64,
    /// Calls that were refused with `overloaded` because the mailbox was
    /// full or too many calls waited.
    pub overloaded: u64,
    /// Zones that run slower than one tick per frame now (time dilation).
    pub dilated_zones: u64,
    /// Zone ticks that time dilation skipped since the start.
    pub zone_ticks_skipped: u64,
    /// Inputs in the input log since the world was made.
    pub journal_records: u64,
    /// Inputs in the log that are not forced to the disk yet.
    pub journal_lag: u64,
    /// Snapshots written since the start.
    pub snapshots: u64,
    /// Snapshots that could not be written since the start.
    pub snapshot_errors: u64,
    /// Frame of the last snapshot that is on the disk.
    pub last_snapshot_frame: u64,
    /// Time that the simulation thread stood still for the last snapshot.
    pub last_snapshot_micros: u64,
    /// Time that the background thread needed to write the last snapshot.
    pub last_snapshot_write_micros: u64,
    /// Size of the last snapshot in bytes.
    pub last_snapshot_bytes: u64,
    /// Rollbacks since the start.
    pub rollbacks: u64,
    /// Time that the simulation thread spent taking calls from the mailbox
    /// and encoding their inputs for the journal.
    pub intake_micros: u64,
    /// Time that the simulation thread waited for the journal to commit.
    pub commit_micros: u64,
    /// Time that the simulation thread spent in the world between frames:
    /// applying inputs and building observations.
    pub apply_micros: u64,
    /// Time that the simulation thread spent handing answers to callers.
    pub reply_micros: u64,
    /// Observations that the simulation thread built since the start, for
    /// `observe` calls and for `wait` calls. A cluster builds observations
    /// in its workers and reports 0.
    pub observations: u64,
    /// Steps in which the simulation thread built observations. One step
    /// builds the observations of all `observe` calls of one batch, or of
    /// all `wait` calls that one frame woke.
    pub observe_batches: u64,
    /// Time that the simulation thread spent building observations.
    pub observe_micros: u64,
}

/// Converts a duration to whole microseconds.
pub(crate) fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// The times of the last frames, for percentiles.
pub(crate) struct FrameWindow {
    times: Vec<u64>,
    capacity: usize,
    next: usize,
}

impl FrameWindow {
    /// Makes a window over the last `capacity` frames.
    pub(crate) fn new(capacity: usize) -> FrameWindow {
        FrameWindow { times: Vec::new(), capacity: capacity.max(1), next: 0 }
    }

    /// Adds the time of one frame in microseconds.
    pub(crate) fn push(&mut self, micros: u64) {
        if self.times.len() < self.capacity {
            self.times.push(micros);
        } else {
            self.times[self.next] = micros;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    /// Returns the frame times at the given percentiles (nearest rank). All
    /// are 0 before the first frame.
    pub(crate) fn percentiles<const N: usize>(&self, percents: [u32; N]) -> [u64; N] {
        let mut sorted = self.times.clone();
        sorted.sort_unstable();
        percents.map(|percent| {
            let rank = (sorted.len() * percent as usize).div_ceil(100);
            sorted.get(rank.saturating_sub(1)).copied().unwrap_or(0)
        })
    }
}

/// Measures how fast a counter grows, over windows of a fixed length.
pub(crate) struct RateMeter {
    window: Duration,
    mark: Instant,
    mark_total: u64,
    rate: u64,
}

impl RateMeter {
    /// Starts a meter at `now` with a counter of 0.
    pub(crate) fn new(window: Duration, now: Instant) -> RateMeter {
        RateMeter {
            window: window.max(Duration::from_millis(1)),
            mark: now,
            mark_total: 0,
            rate: 0,
        }
    }

    /// Tells the meter the value of the counter at `now`. When a window is
    /// over, the meter computes the rate of that window.
    pub(crate) fn observe(&mut self, now: Instant, total: u64) {
        let elapsed = now.saturating_duration_since(self.mark);
        if elapsed < self.window {
            return;
        }
        let grown = u128::from(total.saturating_sub(self.mark_total));
        self.rate =
            u64::try_from(grown * 1_000_000_000 / elapsed.as_nanos().max(1)).unwrap_or(u64::MAX);
        self.mark = now;
        self.mark_total = total;
    }

    /// Returns the rate of the last full window, per second.
    pub(crate) fn per_second(&self) -> u64 {
        self.rate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_use_the_nearest_rank_over_the_last_frames() {
        let mut window = FrameWindow::new(100);
        assert_eq!(window.percentiles([50, 99]), [0, 0]);
        for time in 1..=100 {
            window.push(time);
        }
        assert_eq!(window.percentiles([50, 95, 99, 100]), [50, 95, 99, 100]);
        // Ten more frames push the ten oldest out: 11..=110 remain.
        for time in 101..=110 {
            window.push(time);
        }
        assert_eq!(window.percentiles([1, 50, 100]), [11, 60, 110]);
    }

    #[test]
    fn a_window_of_one_frame_is_that_frame() {
        let mut window = FrameWindow::new(8);
        window.push(700);
        assert_eq!(window.percentiles([50, 95, 99]), [700, 700, 700]);
    }

    #[test]
    fn the_rate_is_the_growth_of_a_full_window() {
        let start = Instant::now();
        let mut meter = RateMeter::new(Duration::from_secs(1), start);
        meter.observe(start + Duration::from_millis(400), 1_000);
        assert_eq!(meter.per_second(), 0, "no window is over yet");
        meter.observe(start + Duration::from_millis(2_000), 5_000);
        assert_eq!(meter.per_second(), 2_500, "5,000 inputs in two seconds");
        meter.observe(start + Duration::from_millis(2_500), 9_000);
        assert_eq!(meter.per_second(), 2_500, "half a window does not change the rate");
        meter.observe(start + Duration::from_millis(3_000), 9_000);
        assert_eq!(meter.per_second(), 4_000, "4,000 inputs in the next second");
        meter.observe(start + Duration::from_millis(4_000), 9_000);
        assert_eq!(meter.per_second(), 0, "an idle window");
    }
}
