//! One frame, prepared for viewers.
//!
//! The cache makes one [`Digest`] per frame. Every viewer task reads the
//! same digest. A message is serialized once, by the first viewer that
//! needs it, and all viewers share the bytes.

use std::sync::{Arc, OnceLock};

use axum::extract::ws::Utf8Bytes;
use unsup_protocol::{Clock, FrameOutput, Tick, ZoneId};

use crate::cache::Resolved;
use crate::wire::{ClipFrame, FeedEvent, ServerMessage, Shot, Story};

/// Writes a message as JSON text. A message that cannot be written gives
/// an empty text; every message of this crate is plain data, so this does
/// not happen.
pub(crate) fn to_text(message: &ServerMessage) -> Utf8Bytes {
    match serde_json::to_string(message) {
        Ok(text) => Utf8Bytes::from(text),
        Err(error) => {
            tracing::error!(%error, "a feed message cannot be written as JSON");
            Utf8Bytes::default()
        }
    }
}

/// What one zone has for viewers in one frame.
#[derive(Debug)]
pub(crate) struct ZoneBeat {
    /// The zone.
    pub zone: ZoneId,
    /// Local tick of the zone after the frame.
    pub tick: Tick,
    /// Place of the zone delta in the frame, when the zone changed.
    pub delta: Option<usize>,
    /// The resolved delta, when a resident left the zone in this frame.
    pub resolved: Option<Resolved>,
    /// Public events of the zone.
    pub events: Vec<Arc<FeedEvent>>,
    text: OnceLock<Utf8Bytes>,
}

impl ZoneBeat {
    /// Makes the beat of a zone without events.
    pub fn new(zone: ZoneId, tick: Tick) -> ZoneBeat {
        ZoneBeat {
            zone,
            tick,
            delta: None,
            resolved: None,
            events: Vec::new(),
            text: OnceLock::new(),
        }
    }
}

/// One frame, prepared for viewers.
#[derive(Debug)]
pub(crate) struct Digest {
    /// Number of the frame.
    pub frame: u64,
    source: Arc<FrameOutput>,
    /// Every public event of the frame for the feed, in the order of the
    /// feed.
    pub events: Vec<Arc<FeedEvent>>,
    /// The zones that changed or had an event, in ascending id order.
    zones: Vec<ZoneBeat>,
    /// The `frame` message, the same for every viewer.
    frame_text: Utf8Bytes,
    /// The `director` message, when the shot changed in this frame.
    director_text: Option<Utf8Bytes>,
    /// The `stories` message, when stories changed in this frame.
    stories_text: Option<Utf8Bytes>,
}

impl Digest {
    /// Makes the digest of a frame. `zones` must be in ascending id order.
    pub fn new(
        source: Arc<FrameOutput>,
        zones: Vec<ZoneBeat>,
        populations: Vec<(ZoneId, u32)>,
        headlines: &[Arc<FeedEvent>],
        events: Vec<Arc<FeedEvent>>,
        viewers: u32,
    ) -> Digest {
        let frame_text = to_text(&ServerMessage::Frame {
            frame: source.frame,
            clock: source.clock,
            populations,
            headlines: headlines.iter().map(|event| (**event).clone()).collect(),
            viewers,
        });
        Digest {
            frame: source.frame,
            source,
            events,
            zones,
            frame_text,
            director_text: None,
            stories_text: None,
        }
    }

    /// Adds what the show has for viewers in this frame: the shot of the
    /// director when it changed, and the stories that changed.
    pub fn set_show(&mut self, shot: Option<Shot>, stories: Vec<Story>) {
        self.director_text = shot.map(|shot| to_text(&ServerMessage::Director { shot }));
        self.stories_text = (!stories.is_empty())
            .then(|| to_text(&ServerMessage::Stories { stories, reset: false }));
    }

    /// Returns the `director` message, when the shot changed in this frame.
    pub fn director_text(&self) -> Option<Utf8Bytes> {
        self.director_text.clone()
    }

    /// Returns the `stories` message, when stories changed in this frame.
    pub fn stories_text(&self) -> Option<Utf8Bytes> {
        self.stories_text.clone()
    }

    /// Returns the calendar time of the frame.
    pub fn clock(&self) -> Clock {
        self.source.clock
    }

    /// Returns the zones that changed or had an event, in ascending id
    /// order.
    pub fn zones(&self) -> impl Iterator<Item = ZoneId> + '_ {
        self.zones.iter().map(|beat| beat.zone)
    }

    /// Returns what a zone has in this frame as a frame of a clip, or
    /// `None` when the zone has nothing.
    pub fn zone_frame(&self, zone: ZoneId) -> Option<ClipFrame> {
        let at = self.zones.binary_search_by_key(&zone, |beat| beat.zone).ok()?;
        match self.delta_message(&self.zones[at]) {
            ServerMessage::Delta {
                frame, tick, moved, entered, left, activity, events, ..
            } => Some(ClipFrame {
                frame,
                clock: self.source.clock,
                tick,
                moved,
                entered,
                left,
                activity,
                events,
            }),
            _ => None,
        }
    }

    /// Returns the `frame` message.
    pub fn frame_text(&self) -> Utf8Bytes {
        self.frame_text.clone()
    }

    /// Returns the `delta` message of a zone, or `None` when the zone has
    /// nothing in this frame.
    pub fn zone_text(&self, zone: ZoneId) -> Option<Utf8Bytes> {
        let at = self.zones.binary_search_by_key(&zone, |beat| beat.zone).ok()?;
        let beat = &self.zones[at];
        Some(beat.text.get_or_init(|| to_text(&self.delta_message(beat))).clone())
    }

    fn delta_message(&self, beat: &ZoneBeat) -> ServerMessage {
        let raw = beat.delta.and_then(|at| self.source.zones.get(at));
        // A resolved delta replaces the lists of the simulation.
        let (entered, left, moved, activity) = match (&beat.resolved, raw) {
            (Some(Resolved { entered, left, moved, activity }), _) => {
                (entered.as_slice(), left.as_slice(), moved.as_slice(), activity.as_slice())
            }
            (None, Some(delta)) => (
                delta.entered.as_slice(),
                delta.left.as_slice(),
                delta.moved.as_slice(),
                delta.activity.as_slice(),
            ),
            (None, None) => Default::default(),
        };
        ServerMessage::Delta {
            frame: self.frame,
            zone: beat.zone,
            tick: beat.tick,
            moved: moved.iter().map(|(id, pos)| (*id, pos.x, pos.y)).collect(),
            entered: entered.to_vec(),
            left: left.to_vec(),
            activity: activity.to_vec(),
            events: beat.events.iter().map(|event| (**event).clone()).collect(),
        }
    }
}
