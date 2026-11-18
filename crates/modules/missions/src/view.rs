//! How a mission looks in an answer, a notice, an event and an observation.
//!
//! The engine makes most of it from numbers. A user wrote the title, the
//! text, the name of a sponsor, and the event kinds and data fields of the
//! objectives. The last two passed the checks of `check.rs` and have no
//! spaces, but they are text of a user. All of it goes out as `Untrusted`.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ResidentId, Value};

use crate::model::{Mission, Missions, Objective, Poster, Taker};
use crate::text;

/// Returns the name of the mode of a mission.
pub(crate) fn mode(mission: &Mission) -> &'static str {
    if mission.def.team { "team" } else { "solo" }
}

/// Returns the progress of a taker: for each objective, the events counted
/// and the events needed.
pub(crate) fn progress(mission: &Mission, taker: &Taker) -> Value {
    let pairs: Vec<Value> = mission
        .def
        .objectives
        .iter()
        .zip(&taker.progress)
        .map(|(objective, done)| json!([done, objective.count]))
        .collect();
    Value::Array(pairs)
}

/// Returns one objective as JSON. `author` is the resident that wrote it.
fn objective(author: ResidentId, objective: &Objective) -> Value {
    let mut out = Map::new();
    out.insert("event".into(), text::untrusted(author, &objective.event));
    if !objective.fields.is_empty() {
        // The fields as the text of a JSON object, names in order.
        let fields: Map<String, Value> =
            objective.fields.iter().map(|(name, value)| (name.clone(), value.to_value())).collect();
        out.insert("fields".into(), text::untrusted(author, &Value::Object(fields).to_string()));
    }
    out.insert("count".into(), json!(objective.count));
    out.insert("actor".into(), json!(if objective.team { "team" } else { "assignee" }));
    Value::Object(out)
}

/// Returns every objective of a mission as JSON.
pub(crate) fn objectives(mission: &Mission) -> Value {
    let author = mission.author();
    Value::Array(mission.def.objectives.iter().map(|o| objective(author, o)).collect())
}

/// Puts the facts that every list shows into `out`: the id, the reward, the
/// mode, the deadline and how many takers the mission has.
fn core(mission: &Mission, out: &mut Map<String, Value>) {
    out.insert("mission".into(), json!(mission.id));
    out.insert("reward".into(), json!(mission.def.reward));
    out.insert("mode".into(), json!(mode(mission)));
    if let Some(org) = mission.def.org {
        out.insert("org".into(), json!(org));
    }
    out.insert("deadline".into(), json!(mission.deadline));
    out.insert("taken".into(), json!(mission.takers.len()));
    out.insert("takers".into(), json!(mission.def.takers));
}

/// Returns the short form of a mission with its title: for a list.
pub(crate) fn brief(mission: &Mission) -> Value {
    let mut out = Map::new();
    core(mission, &mut out);
    out.insert("title".into(), text::untrusted(mission.author(), &mission.def.title));
    out.insert("sponsored".into(), json!(mission.poster.resident().is_none()));
    out.insert("objectives".into(), json!(mission.def.objectives.len()));
    Value::Object(out)
}

/// Returns everything that a resident can know about a mission. With
/// `viewer`, the answer says how the viewer stands in the mission.
pub(crate) fn full(state: &Missions, mission: &Mission, viewer: Option<ResidentId>) -> Value {
    let mut out = Map::new();
    core(mission, &mut out);
    let author = mission.author();
    out.insert("title".into(), text::untrusted(author, &mission.def.title));
    if !mission.def.text.is_empty() {
        out.insert("text".into(), text::untrusted(author, &mission.def.text));
    }
    match &mission.poster {
        Poster::Resident { id, .. } => {
            out.insert("poster".into(), json!(id));
        }
        Poster::Sponsor { name } => {
            out.insert("sponsor".into(), text::untrusted(author, name));
        }
    }
    out.insert("posted".into(), json!(mission.posted));
    out.insert("completed".into(), json!(mission.completed()));
    out.insert("objectives".into(), objectives(mission));
    if let Some(viewer) = viewer
        && let Some(you) = standing(state, mission, viewer)
    {
        out.insert("you".into(), you);
    }
    Value::Object(out)
}

/// Returns how a resident stands in a mission: `posted`, `accepted` with
/// the progress, or `paid` with the amount. Nothing when the resident has
/// no part in it.
pub(crate) fn standing(state: &Missions, mission: &Mission, resident: ResidentId) -> Option<Value> {
    if mission.poster.resident() == Some(resident) {
        return Some(json!({ "status": "posted" }));
    }
    if let Some(key) = state.key_of(resident, mission.id)
        && let Some(taker) = mission.takers.get(&key)
    {
        let mut out = Map::new();
        out.insert("status".into(), json!("accepted"));
        if mission.def.team {
            out.insert("org".into(), json!(key));
            out.insert("assignee".into(), json!(taker.assignee));
            out.insert("members".into(), json!(taker.members.len()));
        }
        out.insert("progress".into(), progress(mission, taker));
        return Some(Value::Object(out));
    }
    paid_to(mission, resident)
        .map(|(cents, frame)| json!({ "status": "paid", "paid": cents, "frame": frame }))
}

/// Returns the cents that a mission paid to a resident and the frame of the
/// payment, when the resident was a taker, or a member of a team, that
/// completed the mission. A member that added no event has 0 cents.
pub(crate) fn paid_to(mission: &Mission, resident: ResidentId) -> Option<(i64, u64)> {
    mission.takers.values().find_map(|taker| {
        let receipt = taker.done.as_ref()?;
        if !taker.members.contains_key(&resident) {
            return None;
        }
        let cents = receipt.shares.iter().find(|(who, _)| *who == resident).map_or(0, |s| s.1);
        Some((cents, receipt.frame))
    })
}
