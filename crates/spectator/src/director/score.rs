//! The score of a subject: how much it deserves the shot now.
//!
//! All numbers are integers, so the same events give the same scores on
//! every machine.

use std::collections::VecDeque;
use std::sync::Arc;

use unsup_protocol::{ResidentId, SiteId, ZoneId};

use crate::config::DirectorConfig;
use crate::wire::FeedEvent;

/// What a shot can be about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Subject {
    /// One resident.
    Resident(ResidentId),
    /// One site.
    Place(SiteId),
    /// One district.
    Zone(ZoneId),
}

/// One event in the score of a subject.
#[derive(Clone, Debug)]
pub(crate) struct Beat {
    /// The event.
    pub event: Arc<FeedEvent>,
    /// One line about the event.
    pub line: String,
    /// The story of the event, if it is part of one.
    pub story: Option<u64>,
    /// Percent of the drama score that this subject gets.
    pub share: u64,
}

impl Beat {
    /// Returns the weight of the event at a frame: its drama score at the
    /// frame of the event, falling in a straight line to zero over
    /// `fresh` frames. The unit is one drama point times `fresh`.
    pub fn weight(&self, frame: u64, fresh: u64) -> u64 {
        let age = frame.saturating_sub(self.event.frame);
        u64::from(self.event.drama) * fresh.saturating_sub(age)
    }
}

/// The events of one subject that still count, oldest first.
#[derive(Clone, Debug, Default)]
pub(crate) struct Track {
    /// The events.
    pub beats: VecDeque<Beat>,
}

impl Track {
    /// Removes the events that count no more. Returns true when none is
    /// left.
    pub fn prune(&mut self, frame: u64, fresh: u64) -> bool {
        while self.beats.front().is_some_and(|beat| beat.weight(frame, fresh) == 0) {
            self.beats.pop_front();
        }
        self.beats.is_empty()
    }

    /// Returns the score of the subject at a frame: the sum of the weights
    /// of its events, and more for each event after the first.
    pub fn score(&self, frame: u64, config: &DirectorConfig) -> u64 {
        let fresh = config.fresh_frames;
        let mut sum: u64 = 0;
        let mut live: u64 = 0;
        for beat in &self.beats {
            let weight = beat.weight(frame, fresh);
            if weight > 0 {
                sum = sum.saturating_add(weight.saturating_mul(beat.share));
                live += 1;
            }
        }
        let gather = config
            .gather_percent
            .saturating_mul(live.saturating_sub(1))
            .min(config.gather_max_percent);
        sum.saturating_mul(100 + gather) / (100 * 100 * fresh.max(1))
    }

    /// Returns the event that speaks for the subject at a frame: the one
    /// with the greatest weight. Of two with the same weight the newer one
    /// wins.
    pub fn lead(&self, frame: u64, fresh: u64) -> Option<&Beat> {
        let mut best: Option<&Beat> = None;
        for beat in &self.beats {
            let weight = beat.weight(frame, fresh);
            if weight > 0 && best.is_none_or(|lead| weight >= lead.weight(frame, fresh)) {
                best = Some(beat);
            }
        }
        best
    }

    /// Returns the zone of the newest event that has one.
    pub fn zone(&self) -> Option<ZoneId> {
        self.beats.iter().rev().find_map(|beat| beat.event.zone)
    }

    /// Returns the frame of the newest event.
    pub fn newest(&self) -> u64 {
        self.beats.back().map_or(0, |beat| beat.event.frame)
    }
}

/// Returns a score after the loss for variety. `left` is the number of
/// frames since the thing left the air.
pub(crate) fn after_penalty(score: u64, percent: u64, left: u64, variety_frames: u64) -> u64 {
    if left >= variety_frames {
        return score;
    }
    let loss = percent * (variety_frames - left) / variety_frames.max(1);
    score * (100 - loss.min(100)) / 100
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::Config;
    use crate::facts::test_event;

    fn beat(frame: u64, drama: u8, share: u64) -> Beat {
        Beat {
            event: test_event(frame, 0, "x.y", drama, None, json!({})),
            line: String::new(),
            story: None,
            share,
        }
    }

    #[test]
    fn the_weight_of_an_event_falls_in_a_line_to_zero() {
        let beat = beat(10, 60, 100);
        assert_eq!(beat.weight(10, 90), 60 * 90);
        assert_eq!(beat.weight(55, 90), 60 * 45);
        assert_eq!(beat.weight(99, 90), 60);
        assert_eq!(beat.weight(100, 90), 0);
        assert_eq!(beat.weight(500, 90), 0);
    }

    #[test]
    fn events_that_gather_score_more_than_their_sum() {
        let config = Config::default().director;
        assert_eq!(
            (config.fresh_frames, config.gather_percent, config.gather_max_percent),
            (90, 25, 100)
        );
        let mut track = Track::default();
        track.beats.push_back(beat(10, 40, 100));
        assert_eq!(track.score(10, &config), 40);
        // Half of the time is over: half of the score.
        assert_eq!(track.score(55, &config), 20);
        track.beats.push_back(beat(55, 40, 100));
        // 20 + 40, and 25 percent more for the second event.
        assert_eq!(track.score(55, &config), 75);
        for _ in 0..6 {
            track.beats.push_back(beat(55, 10, 100));
        }
        // 120 points, and the greatest addition: 100 percent.
        assert_eq!(track.score(55, &config), 240);
        // A share of 50 percent gives half.
        let mut zone = Track::default();
        zone.beats.push_back(beat(10, 40, 50));
        assert_eq!(zone.score(10, &config), 20);
    }

    #[test]
    fn old_events_leave_the_track() {
        let mut track = Track::default();
        track.beats.push_back(beat(10, 40, 100));
        track.beats.push_back(beat(50, 40, 100));
        assert!(!track.prune(99, 90));
        assert_eq!(track.beats.len(), 2);
        assert!(!track.prune(100, 90));
        assert_eq!(track.beats.len(), 1);
        assert!(track.prune(140, 90));
    }

    #[test]
    fn the_lead_is_the_event_with_the_greatest_weight() {
        let mut track = Track::default();
        track.beats.push_back(beat(10, 80, 100));
        track.beats.push_back(beat(60, 40, 100));
        // At frame 60: 80 * 40 against 40 * 90.
        assert_eq!(track.lead(60, 90).map(|beat| beat.event.frame), Some(60));
        // At frame 20 the second event did not happen yet in this test, but
        // its weight is capped by its own frame: the first one leads.
        let mut early = Track::default();
        early.beats.push_back(beat(10, 80, 100));
        early.beats.push_back(beat(12, 40, 100));
        assert_eq!(early.lead(12, 90).map(|beat| beat.event.frame), Some(10));
        // Equal weight: the newer one.
        let mut equal = Track::default();
        equal.beats.push_back(beat(10, 40, 100));
        equal.beats.push_back(beat(10, 40, 100));
        equal.beats[1].line = "newer".into();
        assert_eq!(equal.lead(10, 90).map(|beat| beat.line.as_str()), Some("newer"));
        assert!(Track::default().lead(10, 90).is_none());
    }

    #[test]
    fn the_loss_for_variety_falls_to_zero() {
        assert_eq!(after_penalty(100, 60, 0, 180), 40);
        assert_eq!(after_penalty(100, 60, 90, 180), 70);
        assert_eq!(after_penalty(100, 60, 180, 180), 100);
        assert_eq!(after_penalty(100, 60, 900, 180), 100);
        assert_eq!(after_penalty(100, 100, 0, 180), 0);
    }
}
