//! Timers: what the kernel must do at a given tick.
//!
//! Actions that end and conditions that expire are scheduled in buckets by
//! tick, so that a tick touches only the residents that are due and never
//! scans the whole table.

use std::collections::BTreeMap;

use unsup_protocol::ResidentId;

/// Number of buckets of the wheel. A timer that is due later than this many
/// ticks from now waits in the `far` map.
const SLOTS: u64 = 512;

/// Greatest number of empty bucket buffers that the wheel keeps for reuse.
const SPARE_MAX: usize = 64;

/// What a timer is for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum TimerKind {
    /// A condition of the resident ends. The key is the condition id.
    Condition,
    /// The running action of the resident ends or times out. The key is the
    /// action id.
    Action,
}

/// One timer. The ordering (resident, kind, key) is the order in which the
/// timers of one tick fire, so that the result does not depend on the order
/// in which they were set.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) struct Timer {
    pub(crate) resident: ResidentId,
    pub(crate) kind: TimerKind,
    pub(crate) key: u64,
}

/// A timer wheel: one bucket per tick for the near future, a sorted map for
/// the rest.
///
/// The wheel is not part of the snapshot. It is rebuilt from the agendas and
/// the conditions, which hold the due ticks.
#[derive(Clone, Debug)]
pub(crate) struct TimerWheel {
    slots: Vec<Vec<Timer>>,
    far: BTreeMap<u64, Vec<Timer>>,
    /// Empty buffers of buckets that have fired. A bucket that fills next
    /// takes one, so only the buckets in use hold memory, and a tick does
    /// not allocate.
    spare: Vec<Vec<Timer>>,
}

impl Default for TimerWheel {
    fn default() -> TimerWheel {
        TimerWheel {
            slots: vec![Vec::new(); SLOTS as usize],
            far: BTreeMap::new(),
            spare: Vec::new(),
        }
    }
}

impl TimerWheel {
    /// Sets a timer for tick `due`. `now` is the current tick; a timer that
    /// is not in the future fires at the next tick.
    pub(crate) fn insert(&mut self, now: u64, due: u64, timer: Timer) {
        let due = due.max(now + 1);
        if due - now < SLOTS {
            let slot = &mut self.slots[(due % SLOTS) as usize];
            if slot.capacity() == 0
                && let Some(buffer) = self.spare.pop()
            {
                *slot = buffer;
            }
            slot.push(timer);
        } else {
            self.far.entry(due).or_default().push(timer);
        }
    }

    /// Moves the timers of tick `now` to `out`, in firing order. Call it
    /// once per tick, with ticks that grow by one.
    pub(crate) fn drain_due(&mut self, now: u64, out: &mut Vec<Timer>) {
        // Bring timers that are now near into the wheel. A far timer is at
        // least SLOTS ticks away when it is set, so one that is due at
        // `now + SLOTS - 1` enters here, well before it is due.
        while let Some(entry) = self.far.first_entry() {
            let due = *entry.key();
            if due.saturating_sub(now) >= SLOTS {
                break;
            }
            let timers = entry.remove();
            if due <= now {
                out.extend(timers);
            } else {
                self.slots[(due % SLOTS) as usize].extend(timers);
            }
        }
        let mut fired = std::mem::take(&mut self.slots[(now % SLOTS) as usize]);
        out.append(&mut fired);
        if fired.capacity() > 0 && self.spare.len() < SPARE_MAX {
            self.spare.push(fired);
        }
        out.sort_unstable();
    }

    /// Returns the number of timers that wait.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.slots.iter().map(Vec::len).sum::<usize>()
            + self.far.values().map(Vec::len).sum::<usize>()
    }

    /// Returns an estimate of the heap bytes that the wheel uses.
    pub(crate) fn memory_estimate(&self) -> usize {
        let per = size_of::<Timer>();
        self.slots.iter().map(|s| s.capacity() * per + size_of::<Vec<Timer>>()).sum::<usize>()
            + self.spare.iter().map(|s| s.capacity() * per + size_of::<Vec<Timer>>()).sum::<usize>()
            + self.far.values().map(|s| s.capacity() * per + 48).sum::<usize>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;

    fn timer(resident: u64, key: u64) -> Timer {
        Timer { resident: ResidentId(resident), kind: TimerKind::Action, key }
    }

    #[test]
    fn every_timer_fires_once_at_its_tick() {
        let mut rng = Rng::seeded(1);
        let mut wheel = TimerWheel::default();
        let mut expected: BTreeMap<u64, Vec<Timer>> = BTreeMap::new();
        let mut fired = 0usize;
        let mut set = 0usize;
        let mut out = Vec::new();
        for now in 0..4_000u64 {
            out.clear();
            wheel.drain_due(now, &mut out);
            let mut want = expected.remove(&now).unwrap_or_default();
            want.sort_unstable();
            assert_eq!(out, want, "tick {now}");
            fired += out.len();
            // Set a few timers: near, at the edge of the wheel and far.
            for _ in 0..3 {
                let delay = match rng.below(4) {
                    0 => 1,
                    1 => rng.below(40) + 1,
                    2 => SLOTS - 2 + rng.below(4),
                    _ => rng.below(1_500) + 1,
                };
                let t = timer(rng.below(50), now * 10 + rng.below(10));
                wheel.insert(now, now + delay, t);
                expected.entry(now + delay).or_default().push(t);
                set += 1;
            }
        }
        assert!(fired > 8_000, "{fired}");
        assert_eq!(fired + wheel.len(), set);
        assert!(wheel.memory_estimate() > 0);
    }

    #[test]
    fn only_buckets_in_use_hold_memory() {
        let mut wheel = TimerWheel::default();
        let mut out = Vec::new();
        // 100 timers per tick, each due 10 ticks later, for four turns of
        // the wheel.
        for now in 0..4 * SLOTS {
            out.clear();
            wheel.drain_due(now, &mut out);
            for i in 0..100 {
                wheel.insert(now, now + 10, timer(i, now));
            }
        }
        let with_capacity = wheel.slots.iter().filter(|s| s.capacity() > 0).count();
        assert_eq!(with_capacity, 10, "ten ticks ahead are in use");
        assert!(wheel.spare.len() <= 2, "{}", wheel.spare.len());
        // About 10 buckets of 100 timers, not 512 of them.
        assert!(wheel.memory_estimate() < 80_000, "{}", wheel.memory_estimate());
    }

    #[test]
    fn a_timer_in_the_past_fires_at_the_next_tick() {
        let mut wheel = TimerWheel::default();
        wheel.insert(10, 10, timer(1, 1));
        wheel.insert(10, 3, timer(2, 2));
        let mut out = Vec::new();
        wheel.drain_due(10, &mut out);
        assert!(out.is_empty());
        wheel.drain_due(11, &mut out);
        assert_eq!(out, [timer(1, 1), timer(2, 2)]);
    }

    #[test]
    fn timers_of_one_tick_fire_in_resident_order() {
        let mut wheel = TimerWheel::default();
        let cond = Timer { resident: ResidentId(2), kind: TimerKind::Condition, key: 9 };
        for t in [timer(3, 1), timer(1, 5), timer(2, 4), cond, timer(1, 2)] {
            wheel.insert(0, 7, t);
        }
        let mut out = Vec::new();
        wheel.drain_due(7, &mut out);
        assert_eq!(out, [timer(1, 2), timer(1, 5), cond, timer(2, 4), timer(3, 1)]);
    }
}
