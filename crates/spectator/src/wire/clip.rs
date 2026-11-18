//! Replay clips: the frames of one zone around one event.

use serde::{Deserialize, Serialize};
use unsup_protocol::{Clock, ResidentId, ResidentView, Tick, ZoneId};

use super::{EventId, FeedEvent};

/// One frame of a clip: what changed in the zone, in the form of a `delta`
/// message.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipFrame {
    /// The frame.
    pub frame: u64,
    /// Calendar time of the frame.
    pub clock: Clock,
    /// Local tick of the zone after the frame.
    pub tick: Tick,
    /// Residents that changed position, as `[id, x, y]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moved: Vec<(ResidentId, i32, i32)>,
    /// Residents that came into the zone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entered: Vec<ResidentView>,
    /// Residents that left the zone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left: Vec<ResidentId>,
    /// Residents whose activity changed, as `[id, verb]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub activity: Vec<(ResidentId, Option<String>)>,
    /// Public events of the zone in this frame.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<FeedEvent>,
}

/// The answer of `/v1/clips/{event}`: the frames of one zone around one
/// public event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Id of the event. It is also the id of the clip.
    pub id: EventId,
    /// The event.
    pub event: FeedEvent,
    /// One line about the event. Show it only as text.
    pub line: String,
    /// The zone of the clip.
    pub zone: ZoneId,
    /// The first frame of the clip.
    pub from: u64,
    /// The last frame of the clip that the service has now.
    pub to: u64,
    /// False while frames after the event are still to come.
    pub complete: bool,
    /// True when the clip is pinned: the service keeps it.
    pub pinned: bool,
    /// Every resident of the zone before the frame `from`, in ascending id
    /// order.
    pub residents: Vec<ResidentView>,
    /// The frames from `from` to `to` in which the zone changed or had an
    /// event, oldest first.
    pub frames: Vec<ClipFrame>,
    /// The address of the clip in the viewer, for example `/?clip=120-3`.
    pub link: String,
}

/// One line of `/v1/clips`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipRef {
    /// Id of the event and of the clip.
    pub id: EventId,
    /// Kind of the event.
    pub kind: String,
    /// One line about the event. Show it only as text.
    pub line: String,
    /// The frame of the event.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// The zone of the clip.
    pub zone: ZoneId,
    /// Drama score of the event.
    pub drama: u8,
    /// The address of the clip in the viewer.
    pub link: String,
}

/// The answer of `/v1/clips`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipList {
    /// The pinned clips, newest first.
    pub clips: Vec<ClipRef>,
}

/// The answer of `POST /v1/clips/{event}/pin`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinAnswer {
    /// Id of the clip.
    pub id: EventId,
    /// True when the service keeps the clip now. False while the clip
    /// waits for the frames after its event; the service then pins it when
    /// they are there.
    pub pinned: bool,
    /// The address of the clip in the viewer.
    pub link: String,
}
