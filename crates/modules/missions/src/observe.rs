//! The observation section `missions`: the missions that a resident works
//! on, with their progress, and a few new missions that it can take.

use std::cmp::Reverse;

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ObserveOpts, Realm, ResidentId, Value};

use crate::board::Boards;
use crate::model::{Mission, Missions};
use crate::params::{self, Params};
use crate::{text, view};

/// Returns one mission that the resident works on.
fn accepted(state: &Missions, mission: &Mission, resident: ResidentId, full: bool) -> Value {
    let mut out = Map::new();
    out.insert("mission".into(), json!(mission.id));
    out.insert("reward".into(), json!(mission.def.reward));
    out.insert("deadline".into(), json!(mission.deadline));
    if let Some(key) = state.key_of(resident, mission.id)
        && let Some(taker) = mission.takers.get(&key)
    {
        if mission.def.team {
            out.insert("org".into(), json!(key));
        }
        out.insert("progress".into(), view::progress(mission, taker));
    }
    if full {
        out.insert("title".into(), text::untrusted(mission.author(), &mission.def.title));
        out.insert("objectives".into(), view::objectives(mission));
    }
    Value::Object(out)
}

/// Returns one new mission that the resident can take.
fn fresh(mission: &Mission, full: bool) -> Value {
    let mut out = Map::new();
    out.insert("mission".into(), json!(mission.id));
    out.insert("reward".into(), json!(mission.def.reward));
    out.insert("deadline".into(), json!(mission.deadline));
    if mission.def.team {
        out.insert("mode".into(), json!("team"));
    }
    if full {
        out.insert("title".into(), text::untrusted(mission.author(), &mission.def.title));
    }
    Value::Object(out)
}

/// Builds the section, or nothing when the resident works on no mission
/// and no new mission is near.
///
/// `new` holds missions of the last `missions.new_minutes` game minutes
/// that the resident can take and that are near: a resident published them
/// in the city where the observer is, or a sponsor published them (a
/// sponsor mission has no place). The missions with the highest reward
/// come first.
pub(crate) fn section(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    opts: &ObserveOpts,
) -> Option<Value> {
    let state = realm.res::<Missions>();
    let now = realm.frame();
    let full = opts.is_full();
    let brief_max = params::count(realm, p.observe_max);

    let ids = state.accepted_by(resident);
    let shown = if full { ids.len() } else { brief_max };
    let list: Vec<Value> = ids
        .iter()
        .filter_map(|id| state.open.get(id))
        .take(shown)
        .map(|mission| accepted(&state, mission, resident, full))
        .collect();

    let city = realm
        .directory()
        .zone_of(resident)
        .and_then(|zone| realm.layout().zone(zone))
        .map(|zone| zone.city);
    // The organizations of the resident, as its zone last reported them.
    let boards = realm.res::<Boards>();
    let orgs = boards.records.get(&resident).map(|record| record.orgs.as_slice()).unwrap_or(&[]);
    let window = params::frames(realm, p.new_minutes);
    // Mission ids grow with time, so the newest missions come first here.
    let mut near: Vec<&Mission> = state
        .open
        .values()
        .rev()
        .take_while(|mission| now.saturating_sub(mission.posted) <= window)
        .filter(|mission| {
            mission.poster.resident() != Some(resident)
                && state.key_of(resident, mission.id).is_none()
                && !mission.is_full()
                && now < mission.deadline
                && mission.city.is_none_or(|place| Some(place) == city)
                && (!mission.def.team
                    || match mission.def.org {
                        Some(org) => orgs.contains(&org),
                        None => !orgs.is_empty(),
                    })
        })
        .collect();
    near.sort_by_key(|mission| (Reverse(mission.def.reward), Reverse(mission.id)));
    near.truncate(brief_max);

    if list.is_empty() && near.is_empty() {
        return None;
    }
    let mut out = Map::new();
    if !list.is_empty() {
        if ids.len() > list.len() {
            out.insert("more_accepted".into(), json!(ids.len() - list.len()));
        }
        out.insert("accepted".into(), Value::Array(list));
    }
    if !near.is_empty() {
        let new: Vec<Value> = near.into_iter().map(|mission| fresh(mission, full)).collect();
        out.insert("new".into(), Value::Array(new));
    }
    Some(Value::Object(out))
}
