//! Campaign funds and the verb `donate`.
//!
//! The fund of a candidate is an escrow in the global realm. Only `donate`
//! puts money in. Only `campaign` takes money out. What is left when the
//! election ends goes to the treasury of the government, so a fund never
//! becomes cash of the candidate.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Holder, OrgId, Realm, ResidentId, Value};

use super::{facts_with, find, takes_donations, wrong_phase};
use crate::params::{self, Params};
use crate::pay;
use crate::remote::{Call, Origin, remote_verb, require_cash, spec};
use crate::state::Politics;
use crate::state::election::Candidate;

/// Ledger label of a donation to a campaign fund.
const DONATION: &str = "politics.donation";
/// Ledger label of the rest of a campaign fund that goes to a treasury.
const CLOSED: &str = "politics.fund_closed";

/// Returns the holder of the campaign fund of a candidate.
pub(crate) fn holder(candidate: &Candidate) -> Holder {
    Holder::Escrow(candidate.fund)
}

/// Returns the cents in the campaign fund of a candidate.
pub(crate) fn balance(realm: &Realm, candidate: &Candidate) -> i64 {
    realm.ledger().money(holder(candidate)).cents()
}

/// Moves what is left of a campaign fund to the treasury of a government.
/// Returns the cents moved.
pub(crate) fn close(realm: &Realm, candidate: &Candidate, org: OrgId) -> i64 {
    let rest = balance(realm, candidate);
    pay::transfer(realm, holder(candidate), Holder::Org(org), rest, CLOSED)
}

/// Arguments of `donate`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct DonateArgs {
    /// Id of the election.
    pub(crate) election: u64,
    /// The candidate that gets the money. You can give to yourself.
    pub(crate) candidate: ResidentId,
    /// Cents of your cash for the campaign fund of the candidate. The
    /// fund pays only for `campaign`.
    pub(crate) amount: i64,
}

/// The global part of `donate`.
#[derive(Serialize, Deserialize)]
struct Donate {
    election: u64,
    candidate: ResidentId,
}

impl Call for Donate {
    const KIND: &'static str = "politics.donate";
}

fn run(realm: &Realm, p: &Params, origin: &Origin, call: Donate) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    let election = find(&state, call.election)?;
    if !takes_donations(election.phase) {
        return Err(wrong_phase(election, "Donations end when the polls open."));
    }
    let Some(election) = state.elections.get_mut(&call.election) else {
        return Err(ActionError::internal("The election left the calendar during the request."));
    };
    let Some(candidate) = election.candidates.get_mut(&call.candidate) else {
        return Err(ActionError::not_found(format!(
            "Resident {} is not a candidate in this election.",
            call.candidate
        )));
    };
    let amount = pay::transfer(realm, origin.escrow(), holder(candidate), origin.held, DONATION);
    candidate.raised += amount;
    let fund = balance(realm, candidate);
    let facts = json!({ "candidate": call.candidate, "donor": origin.resident, "amount": amount });
    realm.emit(
        EventMeta::public("politics.donation_made", params::drama(realm, p.drama_donation))
            .actor(origin.resident)
            .at(origin.pos),
        &facts_with(election, facts),
    );
    state.touch_election(call.election);
    Ok(json!({
        "election": call.election,
        "candidate": call.candidate,
        "amount": amount,
        "fund": fund,
    }))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<DonateArgs, Donate>(
        b,
        spec(
            "donate",
            "Give cash to the campaign fund of a candidate. The fund pays only for `campaign`.",
        ),
        p,
        |realm, ctx, args| {
            if args.amount <= 0 {
                return Err(ActionError::invalid_args("The amount must be greater than zero."));
            }
            require_cash(realm, ctx, args.amount)?;
            Ok((Donate { election: args.election, candidate: args.candidate }, args.amount))
        },
        move |realm, origin, call| run(realm, &p, origin, call),
    );
}
