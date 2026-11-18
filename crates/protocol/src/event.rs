//! Events, frames and views: what viewers receive.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::{CityId, Clock, Pos, RealmId, Rect, ResidentId, SiteId, Tick, ZoneId};

/// A fact that happened in the world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    /// Sequence number in the realm that emitted the event.
    pub seq: u64,
    /// Frame in which the event happened.
    pub frame: u64,
    /// Tick of the realm that emitted the event.
    pub tick: Tick,
    /// The realm that emitted the event.
    pub realm: RealmId,
    /// Kind of the event: `<module>.<what_happened>`.
    pub kind: String,
    /// The resident that caused the event, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<ResidentId>,
    /// Where the event happened, if it has a place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<Pos>,
    /// True when viewers may see the event.
    pub public: bool,
    /// How interesting the event is for viewers, 0 to 100.
    pub drama: u8,
    /// Payload of the event.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

/// Everything that changed in one frame, for viewers and for the runtime.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct FrameOutput {
    /// Number of the frame.
    pub frame: u64,
    /// Calendar time of the frame.
    pub clock: Clock,
    /// Changes per zone. Zones without a change are left out.
    pub zones: Vec<ZoneDelta>,
    /// Public events of the frame, ordered by realm and sequence number.
    pub events: Vec<Event>,
    /// Residents that got a notice or finished an action since the last
    /// frame, in ascending order.
    pub woke: Vec<ResidentId>,
}

/// What changed in one zone in one frame.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct ZoneDelta {
    /// The zone.
    pub zone: ZoneId,
    /// Local tick of the zone after the frame.
    pub tick: Tick,
    /// Residents that changed position, with the new position.
    pub moved: Vec<(ResidentId, Pos)>,
    /// Residents that entered the zone (spawned or arrived).
    pub entered: Vec<ResidentView>,
    /// Residents that left the zone (migrated or despawned).
    pub left: Vec<ResidentId>,
    /// Residents whose activity changed, with the verb of the new activity.
    pub activity: Vec<(ResidentId, Option<String>)>,
}

impl ZoneDelta {
    /// Returns true when the delta holds no change.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.moved.is_empty()
            && self.entered.is_empty()
            && self.left.is_empty()
            && self.activity.is_empty()
    }
}

/// The public view of one resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResidentView {
    /// Id of the resident.
    pub id: ResidentId,
    /// Display name.
    pub name: String,
    /// Position.
    pub pos: Pos,
    /// Verb of the running action, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    /// Name of the model that controls the resident, if the owner gave one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// A question about the world from a viewer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ViewQuery {
    /// The list of cities and zones.
    Layout,
    /// Everything in one zone.
    Zone {
        /// The zone.
        zone: ZoneId,
    },
}

/// The answer to a [`ViewQuery`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorldView {
    /// The list of cities and zones.
    Layout {
        /// Frame at which the view was made.
        frame: u64,
        /// Calendar time.
        clock: Clock,
        /// Every city.
        cities: Vec<CityView>,
        /// Every zone.
        zones: Vec<ZoneView>,
    },
    /// Everything in one zone.
    Zone {
        /// The zone.
        zone: ZoneView,
        /// Number of tile columns.
        width: u32,
        /// Number of tile rows.
        height: u32,
        /// Tile kinds, row by row. The length is `width * height`.
        tiles: Vec<u8>,
        /// Sites in the zone.
        sites: Vec<SiteView>,
        /// Residents in the zone, in ascending id order.
        residents: Vec<ResidentView>,
    },
}

/// The public view of one city.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CityView {
    /// Id of the city.
    pub id: CityId,
    /// Name of the city.
    pub name: String,
    /// Tiles that the city covers.
    pub bounds: Rect,
    /// Zones of the city, row by row.
    pub zones: Vec<ZoneId>,
}

/// The public view of one zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ZoneView {
    /// Id of the zone.
    pub id: ZoneId,
    /// The city that the zone belongs to.
    pub city: CityId,
    /// Name of the zone.
    pub name: String,
    /// Tiles that the zone covers.
    pub bounds: Rect,
    /// Number of residents in the zone now.
    pub residents: u32,
}

/// The public view of one site.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SiteView {
    /// Id of the site.
    pub id: SiteId,
    /// Kind of the site, for example `shop`.
    pub kind: String,
    /// Display name.
    pub name: String,
    /// Tiles that the site covers.
    pub rect: Rect,
    /// Tags of the site.
    pub tags: Vec<String>,
}
