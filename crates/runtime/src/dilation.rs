//! Time dilation: overloaded zones skip frames. [`Dilation`] is the pacer
//! that decides; its documentation has the rules.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::time::Duration;

use unsup_kernel::ZoneTiming;
use unsup_protocol::ZoneId;

use crate::journal::Pacer;
use crate::settings::settings;

/// How [`Dilation`] decides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DilationConfig {
    /// Time that the tick of one zone can take. `None` means
    /// `budget_percent` of the frame interval; with manual stepping (an
    /// interval of zero) no zone is ever slowed down then.
    pub budget: Option<Duration>,
    /// Share of the frame interval that the tick of one zone can use, in
    /// percent. Only used when `budget` is `None`.
    pub budget_percent: u32,
    /// A dilated zone runs faster again when the load is below this share
    /// of the budget, in percent.
    pub relax_percent: u32,
    /// Slowest pace: a zone ticks at least once in this many frames.
    pub max_divisor: u32,
    /// Number of measurements in the moving average of a tick time.
    pub smoothing: u32,
    /// Measurements in a row below the relax threshold before a zone runs
    /// one step faster.
    pub calm_ticks: u32,
    /// Threads that tick zones at the same time.
    pub threads: usize,
}

impl Default for DilationConfig {
    /// The numbers of `data/runtime.toml`, and one thread per processor.
    fn default() -> DilationConfig {
        let s = &settings().dilation;
        DilationConfig {
            budget: None,
            budget_percent: s.budget_percent,
            relax_percent: s.relax_percent,
            max_divisor: s.max_divisor,
            smoothing: s.smoothing,
            calm_ticks: s.calm_ticks,
            threads: std::thread::available_parallelism().map_or(1, std::num::NonZero::get),
        }
    }
}

/// What the pacer knows about one zone.
#[derive(Clone, Copy, Debug)]
struct ZoneState {
    /// Moving average of the tick time in nanoseconds.
    average: u64,
    /// True when `average` holds at least one measurement.
    measured: bool,
    /// The zone ticks in one frame of `divisor`.
    divisor: u32,
    /// Frames in a row that the zone skipped.
    skipped: u32,
    /// Measurements in a row below the relax threshold.
    calm: u32,
}

impl ZoneState {
    const NEW: ZoneState =
        ZoneState { average: 0, measured: false, divisor: 1, skipped: 0, calm: 0 };

    /// Time per frame that the zone costs at its pace, in nanoseconds.
    fn load(&self) -> u64 {
        self.average / u64::from(self.divisor)
    }

    /// The part of the load that counts for the sum of all zones. A tick
    /// runs on one thread. What a zone needs above one budget cannot move
    /// to another thread, so it is no reason to slow other zones down.
    fn share(&self, budget: u64) -> u128 {
        u128::from(self.load().min(budget))
    }
}

/// A [`Pacer`] that slows overloaded zones down.
///
/// A frame must fit in the frame interval. When the tick of a zone takes
/// too long, the pacer makes that zone tick only in every second, third,
/// ... frame. Time in the zone then runs slower than in the rest of the
/// world, and the rest of the world keeps its pace.
///
/// The rules:
///
/// - Each zone has a *divisor* `d`: the zone ticks in one frame of `d`. It
///   starts at 1 and never exceeds `max_divisor` (the floor of the pace).
///   A zone never skips more than `max_divisor - 1` frames in a row, also
///   while divisors change.
/// - The pacer keeps a moving average of the tick time of each zone. A zone
///   whose average is above `d` times the budget gets the divisor that
///   brings it back under the budget, at once.
/// - The zones also share the threads of the machine. When the sum of all
///   zone loads is above `threads` times the budget, the zone with the
///   highest load gets a higher divisor, until the sum fits. A zone counts
///   with at most one budget in that sum: a tick runs on one thread, so
///   what one zone needs above the budget is no reason to slow the others.
/// - A divisor goes down one step at a time, and only after the zone and
///   the whole world stayed below `relax_percent` of the budget for
///   `calm_ticks` measurements. The gap between "above the budget" and
///   "below `relax_percent` of it" is the hysteresis: a zone near the limit
///   does not flip between two paces.
/// - Zones with the same divisor take turns, so that the work of the slow
///   zones is spread over the frames.
///
/// The pacer reads the wall clock only through the timings that the
/// runtime gives it. Its answer goes into the `skip` list of `Input::Step`
/// and so into the input log: a replay repeats the decision without the
/// pacer.
#[derive(Clone, Debug)]
pub struct Dilation {
    config: DilationConfig,
    zones: Vec<ZoneState>,
    /// Number of frames that the pacer has decided.
    frame: u64,
}

impl Dilation {
    /// Makes a pacer.
    #[must_use]
    pub fn new(config: DilationConfig) -> Dilation {
        let config = DilationConfig {
            max_divisor: config.max_divisor.max(1),
            smoothing: config.smoothing.max(1),
            threads: config.threads.max(1),
            relax_percent: config.relax_percent.min(100),
            ..config
        };
        Dilation { config, zones: Vec::new(), frame: 0 }
    }

    /// Returns the divisor of a zone: the zone ticks in one frame of that
    /// many. 1 is the normal pace.
    #[must_use]
    pub fn divisor(&self, zone: ZoneId) -> u32 {
        self.zones.get(zone.0 as usize).map_or(1, |state| state.divisor)
    }

    /// Sets a higher divisor for a zone.
    fn raise(&mut self, index: usize, divisor: u32) {
        let state = &mut self.zones[index];
        state.divisor = divisor;
        state.calm = 0;
    }

    /// Returns the zones that skip the frame that comes now.
    ///
    /// A zone with the divisor `d` ticks in every `d`-th frame. Zones with
    /// the same divisor take turns: the first ticks in one frame, the
    /// second in the next, so that the work of the slow zones is spread
    /// over the frames. A zone that skipped `max_divisor - 1` frames in a
    /// row ticks in any case; that is the floor, and it also holds while
    /// the divisors change.
    fn schedule(&mut self) -> Vec<ZoneId> {
        let max = self.config.max_divisor;
        let mut turn = vec![0u64; max as usize + 1];
        let mut skip = Vec::new();
        for (index, state) in self.zones.iter_mut().enumerate() {
            if state.divisor <= 1 {
                state.skipped = 0;
                continue;
            }
            let divisor = u64::from(state.divisor);
            let phase = turn[state.divisor as usize];
            turn[state.divisor as usize] += 1;
            let due = (self.frame + phase).is_multiple_of(divisor);
            if due || state.skipped + 1 >= max {
                state.skipped = 0;
            } else {
                state.skipped += 1;
                skip.push(ZoneId(index as u32));
            }
        }
        self.frame += 1;
        skip
    }

    /// Puts the measurements of the last frame into the averages. Returns
    /// which zones were measured.
    fn observe(&mut self, timings: &[ZoneTiming]) -> Vec<usize> {
        let smoothing = i128::from(self.config.smoothing);
        let mut fresh = Vec::new();
        for timing in timings {
            let index = timing.zone.0 as usize;
            if index >= self.zones.len() {
                self.zones.resize(index + 1, ZoneState::NEW);
            }
            // A zone that did not tick reports 0.
            if timing.elapsed == 0 {
                continue;
            }
            let state = &mut self.zones[index];
            if state.measured {
                let average = i128::from(state.average);
                let moved = average + (i128::from(timing.elapsed) - average) / smoothing;
                state.average = u64::try_from(moved).unwrap_or(u64::MAX);
            } else {
                state.average = timing.elapsed;
                state.measured = true;
            }
            fresh.push(index);
        }
        fresh
    }

    /// Changes the divisors after new measurements.
    fn adjust(&mut self, fresh: &[usize], budget: u64) {
        let max = self.config.max_divisor;
        let relax = u128::from(self.config.relax_percent);
        let capacity = u128::from(budget) * self.config.threads as u128;
        let mut total: u128 = self.zones.iter().map(|state| state.share(budget)).sum();

        // Each zone on its own.
        for &index in fresh {
            let state = self.zones[index];
            let old_share = state.share(budget);
            let wanted = u32::try_from(state.average.div_ceil(budget)).unwrap_or(max).clamp(1, max);
            if wanted > state.divisor {
                self.raise(index, wanted);
            } else if state.divisor > 1 {
                let faster = ZoneState { divisor: state.divisor - 1, ..state };
                let alone = u128::from(faster.load()) * 100 <= u128::from(budget) * relax;
                let together = (total - old_share + faster.share(budget)) * 100 <= capacity * relax;
                let state = &mut self.zones[index];
                if alone && together {
                    state.calm += 1;
                    if state.calm >= self.config.calm_ticks {
                        state.divisor -= 1;
                        state.calm = 0;
                    }
                } else {
                    state.calm = 0;
                }
            }
            total = total - old_share + self.zones[index].share(budget);
        }

        // All zones together: the heaviest zones give way until the frame
        // fits on the threads.
        if total <= capacity {
            return;
        }
        let mut heaviest: BinaryHeap<(u64, Reverse<usize>)> = self
            .zones
            .iter()
            .enumerate()
            .filter(|(_, state)| state.measured && state.divisor < max)
            .map(|(index, state)| (state.load(), Reverse(index)))
            .collect();
        while total > capacity {
            let Some((_, Reverse(index))) = heaviest.pop() else {
                break;
            };
            let old_share = self.zones[index].share(budget);
            self.raise(index, self.zones[index].divisor + 1);
            let state = self.zones[index];
            total = total - old_share + state.share(budget);
            if state.divisor < max {
                heaviest.push((state.load(), Reverse(index)));
            }
        }
    }
}

impl Pacer for Dilation {
    fn skip(&mut self, timings: &[ZoneTiming], tick_interval: Duration) -> Vec<ZoneId> {
        let fresh = self.observe(timings);
        let budget = self
            .config
            .budget
            .unwrap_or_else(|| tick_interval * self.config.budget_percent.min(100) / 100);
        let budget = u64::try_from(budget.as_nanos()).unwrap_or(u64::MAX);
        if budget == 0 {
            // No target for the frame time: every zone ticks in every frame.
            for state in &mut self.zones {
                state.divisor = 1;
                state.calm = 0;
            }
        } else {
            self.adjust(&fresh, budget);
        }
        self.schedule()
    }

    fn dilated_zones(&self) -> usize {
        self.zones.iter().filter(|state| state.divisor > 1).count()
    }
}
