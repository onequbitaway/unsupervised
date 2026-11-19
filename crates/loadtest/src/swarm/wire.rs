//! What an agent reads from the answers of the server.

use serde::Deserialize;
use serde_json::Value;
use unsup_protocol::{
    ActionAck, ActionReport, CityId, ItemInfo, Observation, Pos, ResidentId, SiteId,
};

use crate::scene::{Carried, ListScene, NearSite, Neighbour};

/// The answer to `POST /v1/act`.
#[derive(Clone, Debug, Deserialize)]
pub struct ActAnswer {
    /// One acknowledgement for each request that the server sent to the
    /// simulation.
    pub acks: Vec<ActionAck>,
    /// The observation after the action. `None` when the request asked
    /// for none (`observe=false`).
    #[serde(default)]
    pub observation: Option<Observation>,
}

/// How an action ended, from its report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ended {
    /// Name of the verb.
    pub verb: String,
    /// The error code when the action failed. `None`: it finished.
    pub failed: Option<String>,
}

impl Ended {
    /// Reads the end of an action from its report.
    #[must_use]
    pub fn of(report: &ActionReport) -> Ended {
        let failed = match (&report.error, report.ok) {
            (_, true) => None,
            (Some(error), false) => Some(error.code.as_str().to_string()),
            (None, false) => Some("unknown".to_string()),
        };
        Ended { verb: report.verb.clone(), failed }
    }

    /// Reads the ends of the actions in the `reports` section of an
    /// observation. A report that is not that of the protocol is left out.
    #[must_use]
    pub fn list(observation: &Observation) -> Vec<Ended> {
        let reports = observation.sections.get("reports").and_then(Value::as_array);
        reports
            .into_iter()
            .flatten()
            .filter_map(|report| ActionReport::deserialize(report).ok())
            .map(|report| Ended::of(&report))
            .collect()
    }
}

fn pos_of(value: &Value) -> Option<Pos> {
    let x = i32::try_from(value.get("x")?.as_i64()?).ok()?;
    let y = i32::try_from(value.get("y")?.as_i64()?).ok()?;
    Some(Pos::new(x, y))
}

/// Updates the scene of an agent from an observation: its position, its
/// city, the site where it stands, its cash, its goods, its attributes, and
/// the residents and sites that it sees. `items` is the item list of the
/// catalogue. A section that the observation does not have leaves its part
/// of the scene as it was.
///
/// Returns false when the observation has no position; the scene is then
/// not usable for a generator that needs one.
pub fn update_scene(scene: &mut ListScene, observation: &Observation, items: &[ItemInfo]) -> bool {
    scene.me = observation.resident;
    let mut has_pos = false;
    if let Some(me) = observation.sections.get("self") {
        if let Some(pos) = me.get("pos").and_then(pos_of) {
            scene.pos = pos;
            has_pos = true;
        }
        if let Some(city) = me.get("city").and_then(Value::as_u64) {
            scene.city = u16::try_from(city).ok().map(CityId);
        }
        scene.site_kind = me
            .get("site")
            .and_then(|site| site.get("kind"))
            .and_then(Value::as_str)
            .map(str::to_string);
        scene.cash = me.get("cash").and_then(Value::as_i64);
        scene.carried.clear();
        for (name, qty) in me.get("items").and_then(Value::as_object).into_iter().flatten() {
            let item = items.iter().find(|item| &item.name == name);
            if let (Some(item), Some(qty)) = (item, qty.as_i64().filter(|qty| *qty > 0)) {
                scene.carried.push(Carried { item: item.id, qty });
            }
        }
        scene.attrs.clear();
        for (name, value) in me.get("attrs").and_then(Value::as_object).into_iter().flatten() {
            if let Some(value) = value.as_i64() {
                scene.attrs.insert(name.clone(), value);
            }
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use serde_json::json;
    use unsup_protocol::{ItemId, Tick};

    use super::*;
    use crate::scene::Scene;

    fn observation(sections: Value) -> Observation {
        Observation {
            frame: 1,
            tick: Tick(1),
            clock: Default::default(),
            resident: ResidentId(7),
            sections: serde_json::from_value(sections).expect("a map of sections"),
            notices: Vec::new(),
            next_notice: 3,
        }
    }

    fn item(id: u16, name: &str) -> ItemInfo {
        ItemInfo {
            id: ItemId(id),
            name: name.into(),
            weight: 1,
            tags: Vec::new(),
            attrs: BTreeMap::new(),
            summary: String::new(),
        }
    }

    #[test]
    fn the_scene_follows_the_self_and_nearby_sections() {
        let items = [item(1, "vegetables"), item(6, "bread")];
        let mut scene = ListScene::default();
        let seen = observation(json!({
            "self": {
                "id": 7, "pos": { "x": 12, "y": -3 }, "zone": 2, "city": 1,
                "site": { "id": 40, "name": "Depot", "kind": "depot" },
                "cash": 1800,
                "items": { "bread": 2, "vegetables": 0, "gold": 5 },
                "attrs": { "need.food": 950, "need.energy": 400 },
            },
            "nearby": {
                "residents": [{ "id": 9, "name": "b", "pos": { "x": 13, "y": -3 } }],
                "sites": [{ "id": 41, "name": "Bank", "kind": "bank", "pos": { "x": 20, "y": 0 } }],
            },
        }));
        assert!(update_scene(&mut scene, &seen, &items));
        assert_eq!((scene.me, scene.pos), (ResidentId(7), Pos::new(12, -3)));
        assert_eq!(scene.city, Some(CityId(1)));
        assert_eq!(scene.site_kind(), Some("depot"));
        assert_eq!(scene.cash(), Some(1800));
        // Goods with no unit and goods that the catalogue does not have
        // are not carried.
        assert_eq!(scene.carried, [Carried { item: ItemId(6), qty: 2 }]);
        assert_eq!(scene.attr("need.food"), Some(950));
        assert_eq!(scene.attr("need.mood"), None);
        assert_eq!(scene.residents, [Neighbour { id: ResidentId(9), pos: Pos::new(13, -3) }]);
        assert_eq!(scene.sites.len(), 1);
        assert_eq!((scene.sites[0].id, scene.sites[0].kind.as_str()), (SiteId(41), "bank"));

        // The resident left the site and spent its cash. The observation
        // has no `nearby` section: that part of the scene stays.
        let later = observation(json!({
            "self": { "id": 7, "pos": { "x": 30, "y": 0 }, "city": 1, "cash": 0, "items": {}, "attrs": {} },
        }));
        assert!(update_scene(&mut scene, &later, &items));
        assert_eq!(scene.site_kind(), None);
        assert_eq!(scene.cash(), Some(0));
        assert!(scene.carried.is_empty());
        assert_eq!(scene.residents.len(), 1);

        // Without the `self` section the scene has no fresh position.
        assert!(!update_scene(&mut scene, &observation(json!({ "reports": [] })), &items));
    }

    #[test]
    fn the_ends_of_actions_are_read_from_the_reports() {
        let seen = observation(json!({
            "reports": [
                { "seq": 1, "action_id": 5, "verb": "work", "tick": 9, "ok": true, "result": { "wage": 2000 } },
                { "seq": 2, "action_id": 6, "verb": "move_to", "tick": 9, "ok": false,
                  "error": { "code": "timeout", "message": "too far" } },
                { "not": "a report" },
            ],
        }));
        assert_eq!(
            Ended::list(&seen),
            [
                Ended { verb: "work".into(), failed: None },
                Ended { verb: "move_to".into(), failed: Some("timeout".into()) },
            ]
        );
        assert!(Ended::list(&observation(json!({}))).is_empty());
    }
}
