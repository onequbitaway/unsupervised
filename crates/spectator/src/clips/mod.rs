//! Replay clips: the frames of one zone around one public event.
//!
//! The recorder keeps the last frames of the feed (`clips.window_frames`)
//! and, for every zone, the residents as they were before the oldest of
//! those frames. With both it can show any kept frame of any zone again.
//!
//! A clip has the frames from `before_frames` before its event to
//! `after_frames` after it. A pinned clip is kept after its frames left
//! the recorder; the service also writes it to the storage.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Arc;

use unsup_protocol::{Pos, ResidentId, ResidentView, ZoneId};

use crate::config::ClipsConfig;
use crate::digest::Digest;
use crate::wire::{Clip, ClipFrame, ClipRef, EventId, FeedEvent};

/// The residents of one zone at one time, by id.
type ZoneState = BTreeMap<ResidentId, ResidentView>;

/// Why an event has no clip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoClip {
    /// Clips are turned off.
    Off,
    /// The event has no place.
    NoPlace,
    /// The frames around the event are not kept any more.
    Gone,
}

/// What a request for a pin did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pin {
    /// The clip is pinned now, or was already.
    Done,
    /// The frames after the event are still to come. The recorder pins the
    /// clip when they are there.
    Waiting,
}

/// Returns the address of a clip in the viewer.
pub(crate) fn link_of(id: EventId) -> String {
    format!("/?clip={id}")
}

/// Applies one frame of a clip to the residents of its zone, in the order
/// of the feed: `left`, `entered`, `moved`, `activity`.
fn apply(state: &mut ZoneState, frame: &ClipFrame) {
    for id in &frame.left {
        state.remove(id);
    }
    for view in &frame.entered {
        state.insert(view.id, view.clone());
    }
    for (id, x, y) in &frame.moved {
        if let Some(resident) = state.get_mut(id) {
            resident.pos = Pos::new(*x, *y);
        }
    }
    for (id, verb) in &frame.activity {
        if let Some(resident) = state.get_mut(id) {
            resident.activity.clone_from(verb);
        }
    }
}

/// An event whose clip the recorder pins when its frames are there.
#[derive(Clone, Debug)]
struct Waiting {
    event: Arc<FeedEvent>,
    line: String,
}

/// The recorder. See the module documentation.
#[derive(Debug)]
pub(crate) struct Recorder {
    limits: ClipsConfig,
    /// The frames that the recorder keeps, oldest first.
    ring: VecDeque<Arc<Digest>>,
    /// The residents of every zone before the oldest frame of the ring.
    base: HashMap<ZoneId, ZoneState>,
    waiting: Vec<Waiting>,
    pinned: BTreeMap<EventId, Arc<Clip>>,
    /// Pinned clips that the storage does not have yet.
    unwritten: Vec<Arc<Clip>>,
    /// Clips that left the pins and are still on the storage.
    unremoved: Vec<EventId>,
}

impl Recorder {
    /// Makes a recorder without frames.
    pub fn new(limits: &ClipsConfig) -> Recorder {
        Recorder {
            limits: limits.clone(),
            ring: VecDeque::new(),
            base: HashMap::new(),
            waiting: Vec::new(),
            pinned: BTreeMap::new(),
            unwritten: Vec::new(),
            unremoved: Vec::new(),
        }
    }

    fn on(&self) -> bool {
        self.limits.window_frames > 0
    }

    /// Starts again from a state of the world: the residents of every
    /// zone. The frames that the recorder had are from before a loss and
    /// go. The pins stay.
    pub fn reset(&mut self, zones: impl Iterator<Item = (ZoneId, Vec<ResidentView>)>) {
        self.ring.clear();
        self.waiting.clear();
        self.base.clear();
        if !self.on() {
            return;
        }
        for (zone, residents) in zones {
            self.base.insert(zone, residents.into_iter().map(|view| (view.id, view)).collect());
        }
    }

    /// Takes one frame of the feed. The oldest frame leaves when the
    /// recorder is full. Clips that waited for this frame are pinned.
    pub fn push(&mut self, digest: Arc<Digest>) {
        if !self.on() {
            return;
        }
        self.ring.push_back(digest);
        while self.ring.len() as u64 > self.limits.window_frames {
            let Some(old) = self.ring.pop_front() else { break };
            for zone in old.zones() {
                if let Some(frame) = old.zone_frame(zone) {
                    apply(self.base.entry(zone).or_default(), &frame);
                }
            }
        }
        let newest = self.newest();
        let after = self.limits.after_frames;
        let (ready, waiting): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting)
            .into_iter()
            .partition(|waiting| waiting.event.frame.saturating_add(after) <= newest);
        self.waiting = waiting;
        for Waiting { event, line } in ready {
            if let Ok(clip) = self.build(&event, &line) {
                self.keep(clip);
            }
        }
    }

    /// Tells the recorder about a new event. The recorder pins the clip of
    /// a very dramatic event by itself.
    pub fn note(&mut self, event: &Arc<FeedEvent>, line: &str) {
        if self.on() && event.zone.is_some() && event.drama >= self.limits.auto_pin_drama {
            self.wait_for(event, line);
        }
    }

    fn wait_for(&mut self, event: &Arc<FeedEvent>, line: &str) {
        if !self.waiting.iter().any(|waiting| waiting.event.id == event.id) {
            self.waiting.push(Waiting { event: Arc::clone(event), line: line.to_string() });
        }
    }

    fn newest(&self) -> u64 {
        self.ring.back().map_or(0, |digest| digest.frame)
    }

    /// Makes the clip of an event from the frames of the recorder.
    fn build(&self, event: &FeedEvent, line: &str) -> Result<Clip, NoClip> {
        if !self.on() {
            return Err(NoClip::Off);
        }
        let zone = event.zone.ok_or(NoClip::NoPlace)?;
        let oldest = self.ring.front().map(|digest| digest.frame).ok_or(NoClip::Gone)?;
        if event.frame < oldest || event.frame > self.newest() {
            return Err(NoClip::Gone);
        }
        let from = event.frame.saturating_sub(self.limits.before_frames).max(oldest);
        let end = event.frame.saturating_add(self.limits.after_frames);
        let to = end.min(self.newest());
        let mut state = self.base.get(&zone).cloned().unwrap_or_default();
        let mut frames = Vec::new();
        for digest in &self.ring {
            if digest.frame > to {
                break;
            }
            let Some(frame) = digest.zone_frame(zone) else {
                continue;
            };
            if digest.frame < from {
                apply(&mut state, &frame);
            } else {
                frames.push(frame);
            }
        }
        Ok(Clip {
            id: event.id,
            event: event.clone(),
            line: line.to_string(),
            zone,
            from,
            to,
            complete: self.newest() >= end,
            pinned: false,
            residents: state.into_values().collect(),
            frames,
            link: link_of(event.id),
        })
    }

    /// Pins a clip. The clip of the oldest event leaves when there are too
    /// many.
    fn keep(&mut self, mut clip: Clip) {
        if self.limits.max_pinned == 0 || self.pinned.contains_key(&clip.id) {
            return;
        }
        clip.pinned = true;
        let clip = Arc::new(clip);
        self.pinned.insert(clip.id, Arc::clone(&clip));
        self.unwritten.push(clip);
        while self.pinned.len() > self.limits.max_pinned {
            let Some((id, _)) = self.pinned.pop_first() else { break };
            self.unwritten.retain(|clip| clip.id != id);
            self.unremoved.push(id);
        }
    }

    /// Returns a pinned clip.
    pub fn pinned(&self, id: EventId) -> Option<Arc<Clip>> {
        self.pinned.get(&id).cloned()
    }

    /// Returns the clip of an event: the pinned one, or one from the
    /// frames of the recorder.
    ///
    /// # Errors
    ///
    /// Fails when the event has no clip.
    pub fn clip(&self, event: &FeedEvent, line: &str) -> Result<Clip, NoClip> {
        match self.pinned.get(&event.id) {
            Some(clip) => Ok((**clip).clone()),
            None => self.build(event, line),
        }
    }

    /// Pins the clip of an event, now or when its frames are there.
    ///
    /// # Errors
    ///
    /// Fails when the event has no clip.
    pub fn pin(&mut self, event: &Arc<FeedEvent>, line: &str) -> Result<Pin, NoClip> {
        if self.pinned.contains_key(&event.id) {
            return Ok(Pin::Done);
        }
        let clip = self.build(event, line)?;
        if clip.complete {
            self.keep(clip);
            Ok(Pin::Done)
        } else {
            self.wait_for(event, line);
            Ok(Pin::Waiting)
        }
    }

    /// Returns one line for each pinned clip, the newest event first.
    pub fn list(&self) -> Vec<ClipRef> {
        self.pinned
            .values()
            .rev()
            .map(|clip| ClipRef {
                id: clip.id,
                kind: clip.event.kind.clone(),
                line: clip.line.clone(),
                frame: clip.event.frame,
                clock: clip.event.clock,
                zone: clip.zone,
                drama: clip.event.drama,
                link: clip.link.clone(),
            })
            .collect()
    }

    /// Puts pinned clips from the storage into the recorder. Clips of
    /// frames after `max_frame` are from a time that the world does not
    /// have: they are left out and removed from the storage.
    pub fn restore(&mut self, clips: Vec<Clip>, max_frame: u64) {
        for clip in clips {
            if clip.event.frame > max_frame || self.limits.max_pinned == 0 {
                self.unremoved.push(clip.id);
            } else {
                self.pinned.insert(clip.id, Arc::new(Clip { pinned: true, ..clip }));
            }
        }
        while self.pinned.len() > self.limits.max_pinned {
            let Some((id, _)) = self.pinned.pop_first() else { break };
            self.unremoved.push(id);
        }
    }

    /// Returns the pinned clips that the storage does not have yet and the
    /// clips that it must remove, and forgets both lists.
    pub fn take_unwritten(&mut self) -> (Vec<Arc<Clip>>, Vec<EventId>) {
        (std::mem::take(&mut self.unwritten), std::mem::take(&mut self.unremoved))
    }
}

#[cfg(test)]
mod tests;
