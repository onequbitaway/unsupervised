//! What an agent reads from the answers of the server.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use unsup_protocol::{ActionAck, Observation, Pos, Rect, ResidentId, SiteId, WorldView};

use crate::scene::{ListScene, NearSite, Neighbour};

/// The answer to `POST /v1/act`.
#[derive(Clone, Debug, Deserialize)]
pub struct ActAnswer {
    /// One acknowledgement for each request.
    pub acks: Vec<ActionAck>,
    /// The observation after the action. A server may leave it out.
    #[serde(default)]
    pub observation: Option<Observation>,
}

/// The tiles of each city, from `GET /v1/world`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CityBounds {
    by_city: BTreeMap<u64, Rect>,
}

impl CityBounds {
    /// Reads the bounds from the layout of the world. Another view gives
    /// no bounds.
    #[must_use]
    pub fn from_view(view: &WorldView) -> CityBounds {
        let by_city = match view {
            WorldView::Layout { cities, .. } => {
                cities.iter().map(|city| (u64::from(city.id.0), city.bounds)).collect()
            }
            WorldView::Zone { .. } => BTreeMap::new(),
        };
        CityBounds { by_city }
    }

    /// Returns the tiles of a city.
    #[must_use]
    pub fn of(&self, city: u64) -> Option<Rect> {
        self.by_city.get(&city).copied()
    }
}

fn pos_of(value: &Value) -> Option<Pos> {
    let x = i32::try_from(value.get("x")?.as_i64()?).ok()?;
    let y = i32::try_from(value.get("y")?.as_i64()?).ok()?;
    Some(Pos::new(x, y))
}

/// Updates the scene of an agent from an observation: its position, its
/// city, and the residents and sites that it sees. A section that the
/// observation does not have leaves its part of the scene as it was.
///
/// Returns false when the observation has no position; the scene is then
/// not usable for a generator that needs one.
pub fn update_scene(scene: &mut ListScene, observation: &Observation, cities: &CityBounds) -> bool {
    scene.me = observation.resident;
    let mut has_pos = false;
    if let Some(me) = observation.sections.get("self") {
        if let Some(pos) = me.get("pos").and_then(pos_of) {
            scene.pos = pos;
            has_pos = true;
        }
        if let Some(city) = me.get("city").and_then(Value::as_u64) {
            scene.bounds = cities.of(city);
        }
    }
    if let Some(nearby) = observation.sections.get("nearby") {
        scene.residents.clear();
        for entry in nearby.get("residents").and_then(Value::as_array).into_iter().flatten() {
            let id = entry.get("id").and_then(Value::as_u64);
            if let (Some(id), Some(pos)) = (id, entry.get("pos").and_then(pos_of)) {
                scene.residents.push(Neighbour { id: ResidentId(id), pos });
            }
        }
        scene.sites.clear();
        for entry in nearby.get("sites").and_then(Value::as_array).into_iter().flatten() {
            let id = entry.get("id").and_then(Value::as_u64);
            let kind = entry.get("kind").and_then(Value::as_str);
            if let (Some(id), Some(kind), Some(pos)) = (id, kind, entry.get("pos").and_then(pos_of))
            {
                scene.sites.push(NearSite { id: SiteId(id), kind: kind.to_string(), pos });
            }
        }
    }
    has_pos
}

/// Returns true when an observation holds the report of an ended action.
#[must_use]
pub fn has_report(observation: &Observation) -> bool {
    observation
        .sections
        .get("reports")
        .and_then(Value::as_array)
        .is_some_and(|reports| !reports.is_empty())
}
