//! What viewers send and receive.
//!
//! `docs/design/protocol.md`, section "Viewer feed", describes these types.
//! JSON field names are `snake_case`. A field that is empty is left out.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use unsup_protocol::{CityId, Clock, Pos, RealmId, ResidentId, ResidentView, Tick, ZoneId};

/// Version of the feed protocol. The `world` message carries it.
pub const PROTOCOL_VERSION: u32 = 1;

/// Id of an event in the feed: the frame that delivered the event and the
/// place of the event in that frame.
///
/// It serializes as text, for example `"120-3"`. Ids grow with time, so an
/// id is also the cursor of `/v1/feed`. The same world gives the same ids
/// again after a restart of the server.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct EventId {
    /// The frame that delivered the event.
    pub frame: u64,
    /// Place of the event among the public events of the frame, from 0.
    pub index: u32,
}

impl EventId {
    /// The cursor that is before every event. No event has this id, because
    /// frames are numbered from 1.
    pub const START: EventId = EventId { frame: 0, index: 0 };
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", self.frame, self.index)
    }
}

/// The text is not an event id.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BadEventId;

impl fmt::Display for BadEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an event id is two numbers with a dash, for example 120-3")
    }
}

impl std::error::Error for BadEventId {}

impl FromStr for EventId {
    type Err = BadEventId;

    fn from_str(text: &str) -> Result<EventId, BadEventId> {
        let (frame, index) = text.split_once('-').ok_or(BadEventId)?;
        // `parse` accepts a leading plus sign. An id has only digits.
        let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        if !digits(frame) || !digits(index) {
            return Err(BadEventId);
        }
        Ok(EventId {
            frame: frame.parse().map_err(|_| BadEventId)?,
            index: index.parse().map_err(|_| BadEventId)?,
        })
    }
}

impl Serialize for EventId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EventId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<EventId, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A public event, as viewers get it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedEvent {
    /// Id of the event in the feed.
    pub id: EventId,
    /// The frame that delivered the event.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// Tick of the realm that emitted the event.
    pub tick: Tick,
    /// The realm that emitted the event.
    pub realm: RealmId,
    /// The zone of the event: the zone that emitted it, or the zone of
    /// `pos` when the global realm emitted it. Absent for an event without a
    /// place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// Kind of the event: `<module>.<what_happened>`.
    pub kind: String,
    /// The resident that caused the event, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ResidentId>,
    /// Name of that resident when the event came. A resident chose this
    /// text: show it only as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_name: Option<String>,
    /// Where the event happened, if it has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<Pos>,
    /// How interesting the event is for viewers, 0 to 100.
    pub drama: u8,
    /// Payload of the event. It can hold text that a resident wrote.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

/// A message from a viewer on `/v1/spectate`. The tag is `op`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Add zones to the subscription. The server answers with one
    /// `snapshot` for each new zone.
    Subscribe {
        /// The zones to add.
        zones: Vec<ZoneId>,
    },
    /// Remove zones from the subscription. The server answers with
    /// `unsubscribed`.
    Unsubscribe {
        /// The zones to remove.
        zones: Vec<ZoneId>,
    },
    /// Ask for a `pong`.
    Ping,
}

/// A message from the server on `/v1/spectate`. The tag is `op`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ServerMessage {
    /// The state of the whole world. It is the first message, and it comes
    /// again after the viewer lost frames.
    World {
        /// Version of the feed protocol.
        protocol: u32,
        /// The frame that this state is from.
        frame: u64,
        /// Calendar time of that frame.
        clock: Clock,
        /// Number of residents in every zone, as `[zone, residents]`.
        populations: Vec<(ZoneId, u32)>,
        /// Frames that this viewer lost since the last `world` message.
        dropped: u64,
        /// Viewers that are connected now.
        viewers: u32,
    },
    /// Every resident of one zone. It comes after `subscribe`, and again
    /// after the viewer lost frames. It replaces what the viewer knows
    /// about the zone.
    Snapshot {
        /// The zone.
        zone: ZoneId,
        /// The frame that this state is from. Later `delta` messages of the
        /// zone have a higher frame.
        frame: u64,
        /// The residents, in ascending id order.
        residents: Vec<ResidentView>,
    },
    /// What changed in one subscribed zone in one frame. A zone without a
    /// change and without an event sends no `delta`.
    Delta {
        /// The frame.
        frame: u64,
        /// The zone.
        zone: ZoneId,
        /// Local tick of the zone after the frame.
        tick: Tick,
        /// Residents that changed position, as `[id, x, y]`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        moved: Vec<(ResidentId, i32, i32)>,
        /// Residents that are in the zone now and were not before, or that
        /// came back. No resident is in both `entered` and `left`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entered: Vec<ResidentView>,
        /// Residents that are not in the zone any more.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        left: Vec<ResidentId>,
        /// Residents whose activity changed, as `[id, verb]`. The verb is
        /// `null` when the resident does nothing.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        activity: Vec<(ResidentId, Option<String>)>,
        /// Public events of the zone in this frame.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<FeedEvent>,
    },
    /// The end of one frame, for every viewer. The `delta` messages of the
    /// frame come before it.
    Frame {
        /// The frame.
        frame: u64,
        /// Calendar time of the frame.
        clock: Clock,
        /// Zones whose number of residents changed, as `[zone, residents]`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        populations: Vec<(ZoneId, u32)>,
        /// The most dramatic public events of the frame, from the whole
        /// world.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        headlines: Vec<FeedEvent>,
        /// Viewers that are connected now.
        viewers: u32,
    },
    /// The answer to `unsubscribe`. No `delta` of these zones follows.
    Unsubscribed {
        /// The zones that were removed.
        zones: Vec<ZoneId>,
    },
    /// A request of the viewer was refused. Nothing changed.
    Error {
        /// Why the request was refused.
        error: ErrorBody,
    },
    /// The answer to `ping`.
    Pong,
}

/// The body of an error: a stable code and a sentence for people.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    /// A stable code, for example `unknown_zone`.
    pub code: String,
    /// What went wrong.
    pub message: String,
}

/// The answer of `/v1/feed`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedPage {
    /// Events in ascending id order.
    pub events: Vec<FeedEvent>,
    /// The cursor for the next request: send it as `after`.
    pub next: EventId,
    /// True when more events after `next` are ready now.
    pub more: bool,
    /// True when events between `after` and the first event of this page
    /// were lost: the cache did not keep them any more.
    pub gap: bool,
}

/// The answer of `/v1/residents/{id}/profile`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    /// Id of the resident.
    pub id: ResidentId,
    /// Display name. A resident chose this text: show it only as text.
    pub name: String,
    /// Name of the model that controls the resident, if the owner gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// False when the resident left the world. The place is then the last
    /// place.
    pub alive: bool,
    /// The zone of the resident.
    pub zone: ZoneId,
    /// Name of that zone.
    pub zone_name: String,
    /// The city of that zone.
    pub city: CityId,
    /// Name of that city.
    pub city_name: String,
    /// Position.
    pub pos: Pos,
    /// Verb of the running action, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// The frame that the profile is from. For a resident that left the
    /// world: the frame in which it left.
    pub frame: u64,
    /// Why the resident left the world, when the kernel said why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// The newest public events of the resident that the cache still has,
    /// in ascending id order.
    pub events: Vec<FeedEvent>,
}

/// One series of published state: the newest event of one kind and key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Published {
    /// Kind of the event. It ends in `_published`.
    pub kind: String,
    /// The `key` field of the data, as text. Absent when the data has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The frame that delivered the event.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// Payload of the event.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

/// The answer of `/v1/published/{kind}` and `/v1/stats/economy`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishedKind {
    /// Kind of the events.
    pub kind: String,
    /// Every series of the kind, in ascending key order. A series without a
    /// key comes first.
    pub series: Vec<Published>,
}

/// One line of `/v1/published`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedRef {
    /// Kind of the event.
    pub kind: String,
    /// Key of the series, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// The frame that delivered the newest event of the series.
    pub frame: u64,
}

/// The answer of `/v1/published`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedIndex {
    /// Every series, in ascending order of kind and key.
    pub published: Vec<PublishedRef>,
}

/// The answer of `/v1/spectate/stats`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedStats {
    /// The last frame that the cache holds.
    pub frame: u64,
    /// Residents in the world.
    pub residents: u64,
    /// Viewers that are connected now.
    pub viewers: u32,
    /// Frames that viewers lost since the start, summed over all viewers.
    pub dropped_frames: u64,
    /// Times that a viewer was resynchronized after it lost frames.
    pub resyncs: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn event_ids_are_text_and_keep_their_order() {
        let id = EventId { frame: 120, index: 3 };
        assert_eq!(id.to_string(), "120-3");
        assert_eq!("120-3".parse(), Ok(id));
        assert_eq!(serde_json::to_value(id).expect("json"), json!("120-3"));
        assert_eq!(serde_json::from_value::<EventId>(json!("120-3")).expect("an id"), id);
        // The order is by frame, then by index: not the order of the text.
        assert!(EventId { frame: 9, index: 50 } < EventId { frame: 10, index: 0 });
        assert!(EventId { frame: 10, index: 2 } < EventId { frame: 10, index: 10 });
        assert!(EventId::START < EventId { frame: 1, index: 0 });
    }

    #[test]
    fn text_that_is_not_an_event_id_is_refused() {
        for text in ["", "12", "12-", "-3", "a-3", "12-b", "1-2-3", "+1-2", "1- 2", "1.5-2"] {
            assert_eq!(text.parse::<EventId>(), Err(BadEventId), "{text:?}");
        }
        assert_eq!("99999999999999999999999-1".parse::<EventId>(), Err(BadEventId));
        assert!(serde_json::from_value::<EventId>(json!(12)).is_err());
    }

    #[test]
    fn client_messages_use_the_op_tag() {
        assert_eq!(
            serde_json::from_value::<ClientMessage>(json!({ "op": "subscribe", "zones": [1, 2] }))
                .expect("subscribe"),
            ClientMessage::Subscribe { zones: vec![ZoneId(1), ZoneId(2)] }
        );
        assert_eq!(
            serde_json::from_value::<ClientMessage>(json!({ "op": "ping" })).expect("ping"),
            ClientMessage::Ping
        );
        assert!(serde_json::from_value::<ClientMessage>(json!({ "op": "act" })).is_err());
        assert!(serde_json::from_value::<ClientMessage>(json!({ "op": "subscribe" })).is_err());
    }

    #[test]
    fn a_delta_leaves_out_what_is_empty() {
        let delta = ServerMessage::Delta {
            frame: 7,
            zone: ZoneId(2),
            tick: Tick(7),
            moved: vec![(ResidentId(5), 10, -3)],
            entered: Vec::new(),
            left: Vec::new(),
            activity: vec![(ResidentId(5), None)],
            events: Vec::new(),
        };
        let value = serde_json::to_value(&delta).expect("json");
        assert_eq!(
            value,
            json!({
                "op": "delta", "frame": 7, "zone": 2, "tick": 7,
                "moved": [[5, 10, -3]], "activity": [[5, null]],
            })
        );
        assert_eq!(serde_json::from_value::<ServerMessage>(value).expect("a delta"), delta);
    }
}
