//! The chronicle: the life story of a resident and the history of a place.

use serde::{Deserialize, Serialize};
use unsup_protocol::{Clock, SiteId, ZoneId};

use super::EventId;

/// One event of the chronicle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeEntry {
    /// Id of the event in the feed. `/v1/clips/{event}` replays it while
    /// the service has the frames.
    pub event: EventId,
    /// The frame that delivered the event.
    pub frame: u64,
    /// Calendar time of that frame.
    pub clock: Clock,
    /// Kind of the event.
    pub kind: String,
    /// Drama score of the event.
    pub drama: u8,
    /// One line about the event, with the names of that time. Show it only
    /// as text.
    pub line: String,
    /// The zone of the event, if it has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// The site of the event, if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteId>,
}

/// The life story of a resident: what the chronicle kept, by chapter. Each
/// list is oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifeStory {
    /// The arrival in the world. Absent when the resident came before the
    /// service started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub born: Option<LifeEntry>,
    /// Jobs and businesses.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub jobs: Vec<LifeEntry>,
    /// Homes and travel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub homes: Vec<LifeEntry>,
    /// Contracts, loans, marriages and organizations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deals: Vec<LifeEntry>,
    /// Crimes, arrests and trials.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crimes: Vec<LifeEntry>,
    /// Public offices and powers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub offices: Vec<LifeEntry>,
    /// Other notable events.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moments: Vec<LifeEntry>,
    /// The end: the death, or the departure from the world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub death: Option<LifeEntry>,
    /// The age at the death, when the event said it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age: Option<u64>,
}

/// The answer of `/v1/places/{site}/history`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceHistory {
    /// Id of the site.
    pub site: SiteId,
    /// Name of the site now, when the cache has its zone. It can be text
    /// that a resident chose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The zone of the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// The newest events at the site that the chronicle kept, oldest first.
    pub events: Vec<LifeEntry>,
}
