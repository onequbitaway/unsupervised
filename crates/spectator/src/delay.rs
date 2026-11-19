//! The delay of the stream.
//!
//! Everything that the service reads from the simulation waits here for
//! `stream.delay_frames` frames before the cache gets it. So everything
//! that a viewer can get (the feed, the HTTP reads, the show) is that many
//! frames behind the simulation. With a delay of 0 nothing waits.
//!
//! Each thing waits from a frame number: a frame from its own number, a
//! read of the world from its stamp (`source.rs`). It leaves when the
//! simulation is the delay past that number.

use std::collections::VecDeque;
use std::sync::Arc;

use unsup_protocol::{CityView, Clock, FrameOutput, WorldView, ZoneView};

/// The whole world, as the service read it from the simulation.
#[derive(Debug)]
pub(crate) struct WorldRead {
    /// The frame of the layout.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// Every city.
    pub cities: Vec<CityView>,
    /// Every zone.
    pub zones: Vec<ZoneView>,
    /// The view of every zone that this process hosts.
    pub views: Vec<WorldView>,
}

/// One thing that waits for viewers.
#[derive(Debug)]
pub(crate) enum Held {
    /// One frame of the simulation.
    Frame(Arc<FrameOutput>),
    /// The whole world: at the start, and after the service lost frames.
    World(Box<WorldRead>),
    /// The tiles and sites of one zone, read again.
    Map(WorldView),
}

/// The things that wait.
///
/// Frames and whole worlds leave in the order in which they came: a frame
/// changes what the frame before it left. The map of a zone depends on no
/// frame, so the maps wait in a row of their own and hold no frame up.
#[derive(Debug)]
pub(crate) struct DelayLine {
    delay: u64,
    /// Frames and worlds, each with the frame from which its delay counts,
    /// oldest first.
    held: VecDeque<(u64, Held)>,
    /// Maps of zones, each with the frame from which its delay counts.
    maps: VecDeque<(u64, WorldView)>,
}

impl DelayLine {
    /// Makes a line that holds everything for `delay` frames.
    pub fn new(delay: u64) -> DelayLine {
        DelayLine { delay, held: VecDeque::new(), maps: VecDeque::new() }
    }

    /// Returns the delay in frames.
    pub fn delay(&self) -> u64 {
        self.delay
    }

    /// Returns true when a thing with the stamp `from` waited long enough:
    /// the simulation is at least `delay` frames past it. With a delay of
    /// 0 nothing waits.
    fn is_due(&self, from: u64, live: u64) -> bool {
        self.delay == 0 || from.saturating_add(self.delay) <= live
    }

    /// Puts a thing into the line. `from` is the frame from which its
    /// delay counts: the number of a frame, or the stamp of a read
    /// (`source.rs`).
    pub fn push(&mut self, from: u64, item: Held) {
        match item {
            Held::Map(view) => self.maps.push_back((from, view)),
            item => self.held.push_back((from, item)),
        }
    }

    /// Takes a thing out that waited long enough. `live` is the newest
    /// frame of the simulation that the service received. A map comes
    /// first, then the oldest frame or world.
    pub fn pop_due(&mut self, live: u64) -> Option<Held> {
        if let Some(at) = self.maps.iter().position(|(from, _)| self.is_due(*from, live)) {
            return self.maps.remove(at).map(|(_, view)| Held::Map(view));
        }
        let (from, _) = self.held.front()?;
        if !self.is_due(*from, live) {
            return None;
        }
        self.held.pop_front().map(|(_, item)| item)
    }

    /// Returns the number of things that wait.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.held.len() + self.maps.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(number: u64) -> Held {
        Held::Frame(Arc::new(FrameOutput { frame: number, ..FrameOutput::default() }))
    }

    fn number(item: Option<Held>) -> Option<u64> {
        match item {
            Some(Held::Frame(frame)) => Some(frame.frame),
            Some(other) => panic!("not a frame: {other:?}"),
            None => None,
        }
    }

    #[test]
    fn without_a_delay_nothing_waits() {
        let mut line = DelayLine::new(0);
        line.push(7, frame(7));
        assert_eq!(number(line.pop_due(7)), Some(7));
        assert_eq!(number(line.pop_due(7)), None);
        assert_eq!(line.len(), 0);
    }

    #[test]
    fn a_frame_waits_for_the_delay() {
        let mut line = DelayLine::new(3);
        assert_eq!(line.delay(), 3);
        for live in 1..=5 {
            line.push(live, frame(live));
        }
        // At frame 5 the frames 1 and 2 are 3 frames old or more.
        assert_eq!(number(line.pop_due(3)), None);
        assert_eq!(number(line.pop_due(5)), Some(1));
        assert_eq!(number(line.pop_due(5)), Some(2));
        assert_eq!(number(line.pop_due(5)), None);
        assert_eq!(line.len(), 3);
        assert_eq!(number(line.pop_due(6)), Some(3));
    }

    fn map() -> Held {
        Held::Map(WorldView::Layout {
            frame: 0,
            clock: Clock::default(),
            cities: Vec::new(),
            zones: Vec::new(),
        })
    }

    #[test]
    fn frames_leave_in_the_order_in_which_they_came() {
        let mut line = DelayLine::new(2);
        line.push(4, frame(4));
        line.push(5, frame(5));
        assert_eq!(number(line.pop_due(6)), Some(4));
        assert_eq!(number(line.pop_due(6)), None);
        assert_eq!(number(line.pop_due(7)), Some(5));
    }

    #[test]
    fn a_frame_does_not_pass_a_thing_that_came_before_it() {
        // A world that was read while the simulation ran: its stamp is
        // above the numbers of the frames that follow it.
        let mut line = DelayLine::new(2);
        line.push(
            9,
            Held::World(Box::new(WorldRead {
                frame: 6,
                clock: Clock::default(),
                cities: Vec::new(),
                zones: Vec::new(),
                views: Vec::new(),
            })),
        );
        line.push(7, frame(7));
        line.push(8, frame(8));
        // Frame 7 is 3 frames old, but the world before it is not due.
        assert!(line.pop_due(10).is_none());
        assert!(matches!(line.pop_due(11), Some(Held::World(_))));
        assert_eq!(number(line.pop_due(11)), Some(7));
        assert_eq!(number(line.pop_due(11)), Some(8));
        assert_eq!(line.len(), 0);
    }

    #[test]
    fn a_map_waits_for_its_own_delay_and_holds_no_frame_up() {
        let mut line = DelayLine::new(2);
        line.push(4, frame(4));
        // A read of a map that ended at frame 6 has the stamp 7.
        line.push(7, map());
        line.push(5, frame(5));
        line.push(6, frame(6));
        assert_eq!(number(line.pop_due(6)), Some(4));
        assert_eq!(number(line.pop_due(7)), Some(5));
        assert_eq!(number(line.pop_due(8)), Some(6));
        // The map is not due before frame 9, however often it is asked.
        assert!(line.pop_due(8).is_none());
        assert_eq!(line.len(), 1);
        assert!(matches!(line.pop_due(9), Some(Held::Map(_))));
        assert_eq!(line.len(), 0);
    }

    #[test]
    fn without_a_delay_a_read_with_a_stamp_of_the_next_frame_does_not_wait() {
        let mut line = DelayLine::new(0);
        line.push(8, map());
        line.push(
            8,
            Held::World(Box::new(WorldRead {
                frame: 7,
                clock: Clock::default(),
                cities: Vec::new(),
                zones: Vec::new(),
                views: Vec::new(),
            })),
        );
        assert!(matches!(line.pop_due(7), Some(Held::Map(_))));
        assert!(matches!(line.pop_due(7), Some(Held::World(_))));
        assert!(line.pop_due(7).is_none());
    }
}
