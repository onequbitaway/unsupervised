//! The traces that a crime leaves at its scene.
//!
//! Each zone keeps the traces of the crimes that happened there. An
//! officer that uses `investigate` near a trace can turn it into evidence.
//! A trace gets weaker with time and is gone after `crime.trace_ticks`
//! frames.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Builder, Pos, Realm, ResidentId, Resource, Scope, Stage};
use unsup_life::names::TICKS_PER_HOUR;

use crate::ids::Handles;
use crate::kinds::CrimeKind;

/// The trace of one crime.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Trace {
    pub(crate) case: u64,
    pub(crate) kind: CrimeKind,
    pub(crate) pos: Pos,
    /// Frame of the crime.
    pub(crate) frame: u64,
    /// Stealth level of the perpetrator at the time of the crime.
    pub(crate) stealth: i64,
    /// The officers that already read this trace.
    pub(crate) read_by: Vec<ResidentId>,
}

/// The traces of the crimes of one zone, oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenes {
    traces: Vec<Trace>,
}

impl Resource for Scenes {
    const NAME: &'static str = "crime.scenes";
}

impl Scenes {
    /// Returns the number of traces in the zone.
    #[must_use]
    pub fn len(&self) -> usize {
        self.traces.len()
    }

    /// Returns true when the zone has no trace.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.traces.is_empty()
    }

    /// Adds a trace. The zone keeps at most `max` traces; the oldest goes
    /// first.
    pub(crate) fn add(&mut self, trace: Trace, max: usize) {
        self.traces.push(trace);
        if self.traces.len() > max {
            let extra = self.traces.len() - max;
            self.traces.drain(..extra);
        }
    }

    /// Returns the newest trace within `radius` tiles of `pos` that the
    /// officer did not read and that is younger than `ttl` frames. With
    /// `case`, only the trace of that case counts.
    pub(crate) fn find(
        &self,
        pos: Pos,
        radius: u32,
        officer: ResidentId,
        case: Option<u64>,
        frame: u64,
        ttl: u64,
    ) -> Option<&Trace> {
        self.traces.iter().rev().find(|trace| {
            trace.pos.dist(pos) <= radius
                && frame.saturating_sub(trace.frame) < ttl
                && !trace.read_by.contains(&officer)
                && case.is_none_or(|wanted| wanted == trace.case)
        })
    }

    /// Notes that an officer read the trace of a case.
    pub(crate) fn mark_read(&mut self, case: u64, officer: ResidentId) {
        if let Some(trace) = self.traces.iter_mut().find(|trace| trace.case == case)
            && !trace.read_by.contains(&officer)
        {
            trace.read_by.push(officer);
        }
    }

    /// Removes the traces that are `ttl` frames old or older.
    fn prune(&mut self, frame: u64, ttl: u64) {
        self.traces.retain(|trace| frame.saturating_sub(trace.frame) < ttl);
    }
}

/// Returns the evidence points that a trace of this age gives: all of
/// `full` for a new trace, less with each frame, nothing at `ttl` frames.
pub(crate) fn strength(full: i64, age: u64, ttl: u64) -> i64 {
    if ttl == 0 || age >= ttl {
        return 0;
    }
    let left = i128::from(ttl - age);
    let points = i128::from(full.max(0)) * left / i128::from(ttl);
    i64::try_from(points).unwrap_or(i64::MAX)
}

/// Registers the traces and the pass that removes old ones.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() != Scope::Zone {
        return;
    }
    b.resource::<Scenes>();
    b.every(TICKS_PER_HOUR, Stage::Post, 0, "crime.traces", move |realm: &Realm| {
        if realm.res::<Scenes>().is_empty() {
            return;
        }
        let ttl = h.frames(realm, h.p.trace_ticks);
        realm.res_mut::<Scenes>().prune(realm.frame(), ttl);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(case: u64, x: i32, frame: u64) -> Trace {
        Trace {
            case,
            kind: CrimeKind::Burglary,
            pos: Pos::new(x, 0),
            frame,
            stealth: 0,
            read_by: Vec::new(),
        }
    }

    #[test]
    fn a_trace_gets_weaker_and_ends() {
        assert_eq!(strength(30, 0, 2_880), 30);
        assert_eq!(strength(30, 1_440, 2_880), 15);
        assert_eq!(strength(30, 2_879, 2_880), 0);
        assert_eq!(strength(30, 2_880, 2_880), 0);
        assert_eq!(strength(30, 5, 0), 0);
    }

    #[test]
    fn find_takes_the_newest_unread_trace_in_reach() {
        let officer = ResidentId(5);
        let mut scenes = Scenes::default();
        scenes.add(trace(1, 0, 10), 8);
        scenes.add(trace(2, 3, 20), 8);
        scenes.add(trace(3, 40, 30), 8);
        let found =
            |s: &Scenes, case| s.find(Pos::new(1, 0), 6, officer, case, 50, 100).map(|t| t.case);
        assert_eq!(found(&scenes, None), Some(2));
        assert_eq!(found(&scenes, Some(1)), Some(1));
        assert_eq!(found(&scenes, Some(3)), None);
        scenes.mark_read(2, officer);
        assert_eq!(found(&scenes, None), Some(1));
        // Another officer still finds it.
        assert_eq!(
            scenes.find(Pos::new(1, 0), 6, ResidentId(6), None, 50, 100).map(|t| t.case),
            Some(2)
        );
        // At frame 110 the trace of frame 10 is gone.
        scenes.prune(110, 100);
        assert_eq!(scenes.len(), 2);
        scenes.add(trace(4, 0, 111), 2);
        assert_eq!(scenes.traces.iter().map(|t| t.case).collect::<Vec<_>>(), [3, 4]);
    }
}
