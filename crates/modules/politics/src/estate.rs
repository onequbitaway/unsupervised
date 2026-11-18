//! What happens to the state of the module when a resident leaves the
//! world or leaves an office by itself.
//!
//! The society module removes a resident that left the world from every
//! organization, so its offices are empty. This file removes it from the
//! ballots.

use unsup_kernel::{Builder, Event, OrgId, Realm, ResidentId, Scope};

use crate::election::fund;
use crate::model::Gov;
use crate::state::Politics;

/// Removes a resident that left the world from every ballot. Its campaign
/// fund goes to the treasury of the government of the election. The
/// residents that voted for it can vote again.
fn on_death(realm: &Realm, dead: ResidentId) {
    let mut state = realm.res_mut::<Politics>();
    let ids: Vec<u64> = state.elections.keys().copied().collect();
    for id in ids {
        let Some(election) = state.elections.get_mut(&id) else {
            continue;
        };
        let Some(candidate) = election.candidates.remove(&dead) else {
            continue;
        };
        fund::close(realm, &candidate, election.gov.org());
        election.voters.retain(|_, choice| *choice != dead);
        if election.leader == Some(dead) {
            election.leader = None;
        }
        state.touch_election(id);
    }
    state.rallies.retain(|_, rally| rally.resident != dead);
}

/// Records that the members of a government organization changed. The
/// society module tells it with the event `society.org_left`: a resident
/// left by `org_leave`, or it left the world.
fn on_org_left(realm: &Realm, event: &Event) {
    let Some(org) = event.data["org"].as_u64().map(OrgId) else {
        return;
    };
    // The central banker is a post of the nation.
    let gov = if org == OrgId::CENTRAL_BANK { Some(Gov::Nation) } else { Gov::of_org(org) };
    if let Some(gov) = gov.filter(|gov| gov.city().is_none_or(|c| realm.layout().city(c).is_some()))
    {
        realm.res_mut::<Politics>().touch_government(gov);
    }
}

/// Registers the event handlers.
pub(crate) fn build(b: &mut Builder) {
    if b.scope() != Scope::Global {
        return;
    }
    b.on_event("kernel.despawned", |realm, event| {
        if let Some(dead) = event.actor {
            on_death(realm, dead);
        }
    });
    b.on_event("society.org_left", on_org_left);
}
