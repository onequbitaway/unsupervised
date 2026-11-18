//! The show: the shot of the director and the stories.

use serde::{Deserialize, Serialize};
use unsup_protocol::{CityId, Clock, Pos, ResidentId, SiteId, ZoneId};

use super::EventId;

/// How near the camera is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Zoom {
    /// The whole country.
    Country,
    /// One city.
    City,
    /// One district at street level.
    Street,
    /// Near to one resident or one place.
    Close,
}

/// What the camera looks at. The tag is `type`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Focus {
    /// Nothing in particular: a wide view.
    None,
    /// One resident. The camera follows it.
    Resident {
        /// Id of the resident.
        id: ResidentId,
        /// Name of the resident. A resident chose this text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// One place.
    Place {
        /// The site, when the place is a site.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        site: Option<SiteId>,
        /// Name of the site. It can be text that a resident chose.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Where the place is.
        pos: Pos,
    },
}

/// The choice of the director: where the camera looks, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shot {
    /// Number of the shot. It grows by one with each cut. A message with
    /// the number of the shot on air changes its words only.
    pub seq: u64,
    /// The frame in which the shot started.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// The district of the shot, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// The city of the shot, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<CityId>,
    /// Where the camera looks at the start of the shot, if the shot has a
    /// place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<Pos>,
    /// What the camera looks at.
    pub focus: Focus,
    /// How near the camera is.
    pub zoom: Zoom,
    /// One line about the shot. It can hold names that residents chose:
    /// show it only as text.
    pub caption: String,
    /// True when the shot is breaking news.
    pub breaking: bool,
    /// The event that the caption is about, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<EventId>,
    /// Kind of that event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Drama score of that event, or 0.
    pub drama: u8,
    /// The story of that event, if it is part of one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub story: Option<u64>,
    /// The word for the kind of that story, for example `Crime`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The headline of that story. Show it only as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headline: Option<String>,
}

/// The answer of `/v1/director`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectorState {
    /// The last frame that the director saw.
    pub frame: u64,
    /// The shot on air.
    pub shot: Shot,
}

/// Where a story is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryStatus {
    /// The story can get more events.
    Open,
    /// The story is over.
    Closed,
}

/// A resident of a story.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryResident {
    /// Id of the resident.
    pub id: ResidentId,
    /// Name of the resident when the story last named it. A resident chose
    /// this text.
    pub name: String,
}

/// One event in the timeline of a story.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryBeat {
    /// Id of the event in the feed. `/v1/clips/{event}` replays it.
    pub event: EventId,
    /// The frame that delivered the event.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// Kind of the event.
    pub kind: String,
    /// Drama score of the event.
    pub drama: u8,
    /// One line about the event. Show it only as text.
    pub line: String,
    /// The zone of the event, if it has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// The resident that caused the event, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ResidentId>,
}

/// A running story: events that belong together.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    /// Id of the story. Ids grow with time, from 1.
    pub id: u64,
    /// Kind of the story, for example `crime`.
    pub kind: String,
    /// The word for the kind, for example `Crime`.
    pub label: String,
    /// The headline. Show it only as text.
    pub headline: String,
    /// One line about the newest event. Show it only as text.
    pub latest: String,
    /// Where the story is in its life.
    pub status: StoryStatus,
    /// The frame of the first event.
    pub started: u64,
    /// Calendar time of the first event.
    pub started_clock: Clock,
    /// The frame of the newest event.
    pub updated: u64,
    /// Calendar time of the newest event.
    pub updated_clock: Clock,
    /// The highest drama score of the events of the story.
    pub drama: u8,
    /// Number of events of the story, also those that left the timeline.
    pub events: u64,
    /// The residents of the story, in the order in which they came into it.
    pub residents: Vec<StoryResident>,
    /// The zone of the newest event that has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// The most dramatic event of the story.
    pub lead: EventId,
    /// The events of the story, oldest first.
    pub timeline: Vec<StoryBeat>,
}

/// The answer of `/v1/stories`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoriesPage {
    /// The last frame that the stories saw.
    pub frame: u64,
    /// The stories: open ones first, then by the frame of the newest
    /// event, newest first.
    pub stories: Vec<Story>,
}

/// A channel of the show on `/v1/spectate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    /// The shots of the director: `director` messages.
    Director,
    /// The stories: `stories` messages.
    Stories,
}
