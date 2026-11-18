//! One frame, prepared for viewers.
//!
//! The cache makes one [`Digest`] per frame. Every viewer task reads the
//! same digest. A message is serialized once, by the first viewer that
//! needs it, and all viewers share the bytes.

use std::sync::{Arc, OnceLock};

use axum::extract::ws::Utf8Bytes;
use unsup_protocol::{FrameOutput, Tick, ZoneId};

use crate::cache::Resolved;
use crate::wire::{FeedEvent, ServerMessage};

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
    /// The zones that changed or had an event, in ascending id order.
    zones: Vec<ZoneBeat>,
    /// The `frame` message, the same for every viewer.
    frame_text: Utf8Bytes,
}

impl Digest {
    /// Makes the digest of a frame. `zones` must be in ascending id order.
    pub fn new(
        source: Arc<FrameOutput>,
        zones: Vec<ZoneBeat>,
        populations: Vec<(ZoneId, u32)>,
        headlines: &[Arc<FeedEvent>],
        viewers: u32,
    ) -> Digest {
        let frame_text = to_text(&ServerMessage::Frame {
            frame: source.frame,
            clock: source.clock,
            populations,
            headlines: headlines.iter().map(|event| (**event).clone()).collect(),
            viewers,
        });
        Digest { frame: source.frame, source, zones, frame_text }
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
