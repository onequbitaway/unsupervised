//! The delay of the stream.
//!
//! Everything that the service reads from the simulation waits here for
//! `stream.delay_frames` frames before the cache gets it. So everything
//! that a viewer can get (the feed, the HTTP reads, the show) is that many
//! frames behind the simulation. With a delay of 0 nothing waits.

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

/// The things that wait, oldest first.
#[derive(Debug)]
pub(crate) struct DelayLine {
    delay: u64,
    /// Each thing with the frame of the simulation at which it came.
    held: VecDeque<(u64, Held)>,
}

impl DelayLine {
    /// Makes a line that holds everything for `delay` frames.
    pub fn new(delay: u64) -> DelayLine {
        DelayLine { delay, held: VecDeque::new() }
    }

    /// Returns the delay in frames.
    pub fn delay(&self) -> u64 {
        self.delay
    }

    /// Puts a thing into the line. `live` is the newest frame of the
    /// simulation now.
    pub fn push(&mut self, live: u64, item: Held) {
        self.held.push_back((live, item));
    }

    /// Takes the oldest thing out when it waited long enough: the
    /// simulation is at least `delay` frames past the frame at which the
    /// thing came.
    pub fn pop_due(&mut self, live: u64) -> Option<Held> {
        let (came, _) = self.held.front()?;
        if came.saturating_add(self.delay) > live {
            return None;
        }
        self.held.pop_front().map(|(_, item)| item)
    }

    /// Returns the number of things that wait.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.held.len()
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

    #[test]
    fn things_leave_in_the_order_in_which_they_came() {
        let mut line = DelayLine::new(2);
        line.push(4, frame(4));
        line.push(
            4,
            Held::Map(WorldView::Layout {
                frame: 4,
                clock: Clock::default(),
                cities: Vec::new(),
                zones: Vec::new(),
            }),
        );
        line.push(5, frame(5));
        assert_eq!(number(line.pop_due(6)), Some(4));
        assert!(matches!(line.pop_due(6), Some(Held::Map(_))));
        assert_eq!(number(line.pop_due(6)), None);
        assert_eq!(number(line.pop_due(7)), Some(5));
    }
}
