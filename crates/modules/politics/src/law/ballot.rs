//! The verb `vote_law` and the count of a stage.
//!
//! The electorate of a stage is fixed when the stage begins: the residents
//! with `gov.vote` at the proposal (stage `vote`) or at the veto (stage
//! `override`). Only they vote in the stage. The quorum counts them, and a
//! vote stays when its member leaves the council or the legislature.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, GlobalCtx, Realm, Value};

use super::{enact, facts_with, find, wrong_stage};
use crate::calendar;
use crate::offices::{self, GOV_SIGN, GOV_VOTE};
use crate::params::{self, Params};
use crate::remote::spec;
use crate::state::Politics;
use crate::state::bill::{self, How, Stage, Why};

/// The vote of a member.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub(crate) enum Choice {
    /// In favour.
    Yes,
    /// Against.
    No,
}

impl Choice {
    /// Returns the name in results and events.
    const fn name(self) -> &'static str {
        match self {
            Choice::Yes => "yes",
            Choice::No => "no",
        }
    }
}

/// Arguments of `vote_law`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct VoteLawArgs {
    /// Id of the bill. The `politics` section of your observation lists the
    /// bills that wait for your vote.
    pub(crate) bill: u64,
    /// `yes` or `no`. After a veto, `yes` is a vote to override it.
    pub(crate) vote: Choice,
}

/// Ends the stage `vote` or `override` of a bill and decides what comes
/// next. Returns the stage of the bill after the count, or `None` when no
/// open bill has the id.
pub(crate) fn conclude(realm: &Realm, p: &Params, state: &mut Politics, id: u64) -> Option<Stage> {
    let bill = state.bills.get_mut(&id)?;
    let org = bill.gov.org();
    // The quorum counts the electorate of the stage, not the members of
    // now: a member that leaves does not make a decision easier.
    let members = bill.electorate(|| offices::members_with(realm, org, GOV_VOTE)).len();
    let (yes, no) = bill.count();
    let cast = yes + no;
    let quorate = cast >= bill::quorum(members, realm.param(p.quorum_permille));
    match bill.stage {
        Stage::Vote if !quorate => enact::reject(realm, p, state, id, Why::NoQuorum),
        Stage::Vote if yes <= no => enact::reject(realm, p, state, id, Why::VotedDown),
        Stage::Vote => {
            let executives = offices::members_with(realm, org, GOV_SIGN);
            if executives.is_empty() {
                // Nobody can sign. The bill is law at once.
                return enact::enact(realm, p, state, id, How::Vacant);
            }
            let minutes = u64::try_from(realm.param(p.bill_sign_minutes).max(1)).unwrap_or(1);
            bill.stage = Stage::Sign;
            bill.deadline = realm.frame() + minutes;
            bill.passed = Some((
                u32::try_from(yes).unwrap_or(u32::MAX),
                u32::try_from(no).unwrap_or(u32::MAX),
            ));
            let facts = facts_with(
                bill,
                json!({
                    "yes": yes,
                    "no": no,
                    "members": members,
                    "deadline": calendar::moment(realm, bill.deadline),
                }),
            );
            for executive in executives {
                realm.notify(executive, "politics.bill_passed", None, facts.clone());
            }
            realm.emit(
                EventMeta::public(
                    "politics.bill_passed",
                    params::drama(realm, p.drama_bill_passed),
                ),
                &facts,
            );
            let gov = bill.gov;
            state.touch_bills(gov);
            Some(Stage::Sign)
        }
        Stage::Override => {
            let enough =
                bill::reaches(yes, cast, realm.param(p.override_num), realm.param(p.override_den));
            if quorate && enough {
                enact::enact(realm, p, state, id, How::Override)
            } else {
                enact::reject(realm, p, state, id, Why::Vetoed)
            }
        }
        stage => Some(stage),
    }
}

fn run(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &VoteLawArgs,
) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    let bill = find(&state, args.bill)?;
    if !bill.stage.takes_votes() {
        return Err(wrong_stage(bill, "Members vote in the stages `vote` and `override`."));
    }
    let gov = bill.gov;
    let org = gov.org();
    offices::require(realm, gov, ctx.resident, GOV_VOTE)?;
    let Some(bill) = state.bills.get_mut(&args.bill) else {
        return Err(ActionError::internal("The bill left the register during the request."));
    };
    if !bill.electorate(|| offices::members_with(realm, org, GOV_VOTE)).contains(&ctx.resident) {
        return Err(ActionError::not_allowed(
            "You were not a member when this stage of the bill began. Only the members of that \
             moment vote in it.",
        ));
    }
    if bill.has_voted(ctx.resident) {
        return Err(ActionError::conflict("You voted on this bill in this stage already."));
    }
    match args.vote {
        Choice::Yes => bill.yes.insert(ctx.resident),
        Choice::No => bill.no.insert(ctx.resident),
    };
    let (yes, no) = bill.count();
    let stage = bill.stage;
    realm.emit(
        EventMeta::public("politics.bill_voted", params::drama(realm, p.drama_bill_voted))
            .actor(ctx.resident)
            .at(ctx.pos),
        &facts_with(
            bill,
            json!({
                "member": ctx.resident,
                "vote": args.vote.name(),
                "stage": stage.name(),
                "yes": yes,
                "no": no,
            }),
        ),
    );
    // The stage ends at once when nobody is left to vote: every member of
    // the electorate voted, or has no seat any more.
    let complete = bill
        .electorate
        .iter()
        .flatten()
        .all(|&member| bill.has_voted(member) || !offices::can(realm, org, member, GOV_VOTE));
    state.touch_bills(gov);
    let after = if complete { conclude(realm, p, &mut state, args.bill) } else { None };
    Ok(json!({
        "bill": args.bill,
        "vote": args.vote.name(),
        "yes": yes,
        "no": no,
        "stage": after.unwrap_or(stage).name(),
    }))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<VoteLawArgs>(
        spec(
            "vote_law",
            "Vote on a bill of your council or of the legislature. After a veto, vote on the override. You need `gov.vote`.",
        ),
        move |realm, ctx, args| run(realm, &p, ctx, args),
    );
}
