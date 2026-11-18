//! What happens to missions when a resident goes: it leaves a mission, it
//! leaves an organization, or it leaves the world.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Event, Holder, Realm, ResidentId};

use crate::model::Missions;

/// Takes a resident out of a mission that it works on. Returns false when
/// it did not work on it.
///
/// A solo taker gives its place back and loses its progress. A member of a
/// team leaves the team: the progress of the team stays, and the events of
/// the member no longer count for a share. When the member spoke for the
/// team, the member with the lowest id speaks for it from now on. The last
/// member that leaves gives the place of the team back.
pub(crate) fn leave(state: &mut Missions, id: u64, resident: ResidentId) -> bool {
    let Some(key) = state.key_of(resident, id) else {
        return false;
    };
    state.unenrol(resident, id);
    let Some(mission) = state.open.get_mut(&id) else {
        return true;
    };
    let Some(taker) = mission.takers.get_mut(&key) else {
        return true;
    };
    taker.members.remove(&resident);
    match taker.members.keys().next().copied() {
        None => {
            mission.takers.remove(&key);
        }
        Some(next) if taker.assignee == resident => taker.assignee = next,
        Some(_) => {}
    }
    true
}

/// Reads `kernel.despawned`: a resident that left the world leaves every
/// mission that it worked on.
pub(crate) fn on_despawned(realm: &Realm, event: &Event) {
    let Some(gone) = event.actor else {
        return;
    };
    let mut state = realm.res_mut::<Missions>();
    for id in state.accepted_by(gone) {
        leave(&mut state, id, gone);
    }
}

/// Reads `life.resident_died`: remembers the heir of a resident that has
/// open missions, so that the unpaid reward goes to the heir.
pub(crate) fn on_died(realm: &Realm, event: &Event) {
    let Some(dead) = event.data["resident"].as_u64().map(ResidentId) else {
        return;
    };
    let Ok(heir) = serde_json::from_value::<Holder>(event.data["heir"].clone()) else {
        return;
    };
    let mut state = realm.res_mut::<Missions>();
    if heir != Holder::Resident(dead) && state.posted_by(dead) > 0 {
        state.heirs.insert(dead, heir);
    }
}

/// Reads `society.org_left`: a resident that left an organization leaves
/// the teams of that organization, and gets the news.
pub(crate) fn on_org_left(realm: &Realm, event: &Event) {
    let (Some(org), Some(resident)) =
        (event.data["org"].as_u64(), event.data["resident"].as_u64().map(ResidentId))
    else {
        return;
    };
    let mut state = realm.res_mut::<Missions>();
    let Some(list) = state.enrolled.get(&resident) else {
        return;
    };
    let teams: Vec<u64> = list
        .iter()
        .filter(|&(id, key)| *key == org && state.open.get(id).is_some_and(|m| m.def.team))
        .map(|(&id, _)| id)
        .collect();
    for id in teams {
        leave(&mut state, id, resident);
        realm.notify(
            resident,
            "missions.mission_ended",
            None,
            json!({ "mission": id, "reason": "left_organization" }),
        );
    }
}
