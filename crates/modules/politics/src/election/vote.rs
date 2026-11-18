//! The verb `vote`. The zone checks the age of the voter. The global realm
//! checks the place and puts the vote in the count.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, Realm, ResidentId, Value};

use super::{find, require_place, wrong_phase};
use crate::params::Params;
use crate::remote::{Call, Origin, remote_verb, require_age, spec};
use crate::state::Politics;
use crate::state::election::Phase;

/// Arguments of `vote`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct VoteArgs {
    /// Id of the election. The polls must be open. For a city office you
    /// must be in that city.
    pub(crate) election: u64,
    /// The candidate that gets your vote. You have one vote for each
    /// office.
    pub(crate) candidate: ResidentId,
}

/// The global part of `vote`.
#[derive(Serialize, Deserialize)]
struct Vote {
    election: u64,
    candidate: ResidentId,
}

impl Call for Vote {
    const KIND: &'static str = "politics.vote";
}

fn run(realm: &Realm, origin: &Origin, call: Vote) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    let election = find(&state, call.election)?;
    if election.phase != Phase::Voting {
        return Err(wrong_phase(election, "Residents vote while the polls are open."));
    }
    require_place(election, origin)?;
    if !election.candidates.contains_key(&call.candidate) {
        return Err(ActionError::not_found(format!(
            "Resident {} is not a candidate in this election.",
            call.candidate
        )));
    }
    if election.voters.contains_key(&origin.resident) {
        return Err(ActionError::conflict("You voted in this election already."));
    }
    // One vote for each office: a resident that voted for the mayor of one
    // city cannot vote for the mayor of the next city on the same day.
    let office = election.office;
    let elsewhere = state.elections.values().any(|other| {
        other.id != call.election
            && other.office == office
            && other.voters.contains_key(&origin.resident)
    });
    if elsewhere {
        return Err(ActionError::conflict(
            "You voted for this office in another election that is still open.",
        ));
    }
    let Some(election) = state.elections.get_mut(&call.election) else {
        return Err(ActionError::internal("The election left the calendar during the request."));
    };
    election.voters.insert(origin.resident, call.candidate);
    if let Some(candidate) = election.candidates.get_mut(&call.candidate) {
        candidate.votes += 1;
    }
    let result = json!({
        "election": call.election,
        "office": office.name(),
        "candidate": call.candidate,
        "turnout": election.voters.len(),
    });
    state.touch_election(call.election);
    Ok(result)
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<VoteArgs, Vote>(
        b,
        spec(
            "vote",
            "Vote for a candidate in an election whose polls are open. One vote for each office.",
        ),
        p,
        move |realm, ctx, args| {
            require_age(realm, &p, ctx)?;
            Ok((Vote { election: args.election, candidate: args.candidate }, 0))
        },
        run,
    );
}
