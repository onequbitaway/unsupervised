//! The verb `register_candidate`. The zone takes the fee from the cash of
//! the resident. The global realm puts the resident on the ballot.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Holder, Realm, Value};

use super::{facts_with, find, require_place, wrong_phase};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb, require_cash, spec};
use crate::state::Politics;
use crate::state::election::{Candidate, Phase};
use crate::{pay, text};

/// Ledger label of the fee of a candidate.
const FEE: &str = "politics.candidate_fee";

/// Arguments of `register_candidate`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct RegisterArgs {
    /// Id of the election. The `politics` section of your observation
    /// lists the open elections. For a city office you must be in that
    /// city.
    pub(crate) election: u64,
    /// Your platform: a short text that tells the voters what you will do.
    pub(crate) platform: String,
}

/// The global part of `register_candidate`.
#[derive(Serialize, Deserialize)]
struct Register {
    election: u64,
    platform: String,
}

impl Call for Register {
    const KIND: &'static str = "politics.register_candidate";
}

fn run(realm: &Realm, p: &Params, origin: &Origin, call: Register) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    let election = find(&state, call.election)?;
    if election.phase != Phase::Registration {
        return Err(wrong_phase(election, "Candidates register in the registration phase."));
    }
    require_place(election, origin)?;
    if election.candidates.contains_key(&origin.resident) {
        return Err(ActionError::conflict("You are a candidate in this election already."));
    }
    // A resident has one role in a government, so it runs for one office
    // of a government at a time.
    let gov = election.gov;
    if state.elections.values().any(|e| e.gov == gov && e.candidates.contains_key(&origin.resident))
    {
        return Err(ActionError::conflict(
            "You are a candidate for another office of this government already.",
        ));
    }
    let max = params::count(realm, p.candidates_max);
    if election.candidates.len() >= max {
        return Err(ActionError::conflict(format!(
            "This election has too many candidates. The limit is {max}."
        )));
    }
    // The fee is the one of the city in which the candidate stands. The
    // zone took what it saw; a law can have changed the fee since.
    let fee = realm.param_in(p.candidate_fee, origin.city).max(0);
    if origin.held < fee {
        return Err(ActionError::insufficient("The fee changed. Send the request again.")
            .with_details(json!({ "needed": fee, "available": origin.held })));
    }
    let paid = pay::transfer(realm, origin.escrow(), Holder::Org(gov.org()), fee, FEE);
    let record = Candidate {
        platform: call.platform,
        fund: realm.alloc_id(),
        raised: 0,
        spent: 0,
        votes: 0,
    };
    let platform = text::untrusted(origin.resident, &record.platform);
    let Some(election) = state.elections.get_mut(&call.election) else {
        return Err(ActionError::internal("The election left the calendar during the request."));
    };
    election.candidates.insert(origin.resident, record);
    let candidates = election.candidates.len();
    realm.emit(
        EventMeta::public(
            "politics.candidate_registered",
            params::drama(realm, p.drama_candidate_registered),
        )
        .actor(origin.resident)
        .at(origin.pos),
        &facts_with(
            election,
            json!({
                "candidate": origin.resident,
                "platform": platform,
                "candidates": candidates,
                "fee": paid,
            }),
        ),
    );
    let result = facts_with(election, json!({ "fee": paid, "candidates": candidates }));
    state.touch_election(call.election);
    Ok(result)
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<RegisterArgs, Register>(
        b,
        spec(
            "register_candidate",
            "Run for an office in an election that is in its registration phase. It costs a fee.",
        ),
        p,
        move |realm, ctx, args| {
            text::check_text(&args.platform, realm.param(p.platform_max), "platform")?;
            let fee = realm.param(p.candidate_fee).max(0);
            require_cash(realm, ctx, fee)?;
            Ok((Register { election: args.election, platform: args.platform.clone() }, fee))
        },
        move |realm, origin, call| run(realm, &p, origin, call),
    );
}
