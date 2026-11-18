//! The limits of the service: sizes, counts and times.

use serde::Deserialize;

/// Limits of the event feed.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeedConfig {
    /// Public events that the cache keeps, with any drama score.
    pub recent_events: usize,
    /// Notable events that the cache keeps.
    pub notable_events: usize,
    /// Lowest drama score of a notable event.
    pub notable_drama: u8,
    /// Lowest drama score of a headline.
    pub headline_drama: u8,
    /// Events in one answer of `/v1/feed` when the query has no `limit`.
    pub page_default: usize,
    /// Greatest `limit` of `/v1/feed`.
    pub page_max: usize,
}

/// Limits of one viewer connection.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocketConfig {
    /// Messages that wait for one viewer.
    pub client_queue: usize,
    /// Frames that the task of one viewer can fall behind.
    pub frame_buffer: usize,
    /// Zones that one viewer can subscribe to.
    pub max_zones: usize,
    /// Greatest size of a message from a viewer, in bytes.
    pub max_message_bytes: usize,
    /// Headlines in one `frame` message.
    pub headlines_per_frame: usize,
    /// Events in one `delta` message.
    pub zone_events_per_frame: usize,
    /// Real milliseconds that one message can take to leave.
    pub send_timeout_ms: u64,
}

/// Limits of resident profiles.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    /// Events in one profile.
    pub events: usize,
    /// Residents that left the world and still have a profile.
    pub departed: usize,
}

/// Limits of the zone maps.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapConfig {
    /// Frames after which a request reads the tiles and sites of a zone
    /// again.
    pub refresh_frames: u64,
}

/// How the cache starts.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartConfig {
    /// Real milliseconds before the cache tries again to read the world.
    pub retry_ms: u64,
}

/// Limits of the published state.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedConfig {
    /// Greatest number of series (kind and key) that the cache keeps.
    pub max_series: usize,
    /// Greatest length of a key, in bytes.
    pub max_key_bytes: usize,
    /// The kind that `/v1/stats/economy` serves.
    pub stats_kind: String,
    /// The kind that `/v1/leaderboards/{board}` serves.
    pub leaderboard_kind: String,
}

/// The delay of everything that viewers get.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamConfig {
    /// Frames that everything for viewers is behind the simulation.
    pub delay_frames: u64,
}

/// Limits of the stories.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoriesConfig {
    /// Open stories that the service keeps.
    pub max_open: usize,
    /// Closed stories that the service keeps.
    pub max_closed: usize,
    /// Entries in the timeline of one story.
    pub timeline: usize,
    /// Residents that one story lists.
    pub residents: usize,
    /// Residents that one story remembers.
    pub remembered: usize,
    /// Frames without a new event after which a story closes.
    pub close_frames: u64,
    /// Stories in one `stories` message of a frame.
    pub updates_per_frame: usize,
    /// Stories in one answer of `/v1/stories` when the query has no
    /// `limit`.
    pub page_default: usize,
    /// Greatest `limit` of `/v1/stories`.
    pub page_max: usize,
}

/// Limits of the chronicle.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChronicleConfig {
    /// Lowest drama score of an event that the chronicle keeps.
    pub min_drama: u8,
    /// Events that the chronicle keeps for one resident.
    pub resident_events: usize,
    /// Events that the chronicle keeps for one place.
    pub place_events: usize,
    /// Residents with a history.
    pub residents: usize,
    /// Places with a history.
    pub places: usize,
    /// The chronicle forgets one more resident or place than it must for
    /// each this many of the limit.
    pub forget_one_in: usize,
    /// Events in one answer of `/v1/places/{site}/history`.
    pub place_page: usize,
    /// Frames between two writes of the chronicle to the storage.
    pub flush_frames: u64,
    /// Records in the journal after which the service writes it again.
    pub compact_records: usize,
}

/// Limits of the replay clips.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClipsConfig {
    /// Frames of the feed that the service keeps for replay. 0 turns
    /// clips off.
    pub window_frames: u64,
    /// Frames of one clip before the frame of its event.
    pub before_frames: u64,
    /// Frames of one clip after the frame of its event.
    pub after_frames: u64,
    /// Pinned clips that the service keeps.
    pub max_pinned: usize,
    /// Lowest drama score of an event whose clip the service pins by
    /// itself. 101: it pins none.
    pub auto_pin_drama: u8,
    /// True: `POST /v1/clips/{event}/pin` pins a clip for any viewer.
    pub public_pin: bool,
}
