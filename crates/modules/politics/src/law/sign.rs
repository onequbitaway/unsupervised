//! The verbs `sign_law` and `veto_law` of the executive.

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
use crate::state::bill::{How, Stage};

/// Arguments of `sign_law` and `veto_law`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct BillArgs {
    /// Id of a bill that the members passed. The `politics` section of
    /// your observation lists the bills that wait for you.
    pub(crate) bill: u64,
}

/// Checks that a bill waits for the executive and that the resident is
/// one.
fn check(realm: &Realm, state: &Politics, ctx: &GlobalCtx, id: u64) -> Result<(), ActionError> {
    let bill = find(state, id)?;
    if bill.stage != Stage::Sign {
        return Err(wrong_stage(bill, "The executive acts after the members passed the bill."));
    }
    offices::require(realm, bill.gov, ctx.resident, GOV_SIGN)
}

fn sign(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &BillArgs) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    check(realm, &state, ctx, args.bill)?;
    let stage = enact::enact_by(realm, p, &mut state, args.bill, How::Signed, Some(ctx.resident));
    Ok(json!({ "bill": args.bill, "stage": stage.map(Stage::name) }))
}

fn veto(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &BillArgs) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    check(realm, &state, ctx, args.bill)?;
    let Some(bill) = state.bills.get_mut(&args.bill) else {
        return Err(ActionError::internal("The bill left the register during the request."));
    };
    let minutes = u64::try_from(realm.param(p.bill_override_minutes).max(1)).unwrap_or(1);
    bill.stage = Stage::Override;
    bill.deadline = realm.frame() + minutes;
    bill.vetoed_by = Some(ctx.resident);
    // The override is a new vote.
    bill.yes.clear();
    bill.no.clear();
    let gov = bill.gov;
    let facts = facts_with(
        bill,
        json!({
            "by": ctx.resident,
            "sponsor": bill.sponsor,
            "deadline": calendar::moment(realm, bill.deadline),
        }),
    );
    for member in offices::members_with(realm, gov.org(), GOV_VOTE) {
        realm.notify(member, "politics.law_vetoed", Some(ctx.resident), facts.clone());
    }
    realm.emit(
        EventMeta::public("politics.law_vetoed", params::drama(realm, p.drama_law_vetoed))
            .actor(ctx.resident)
            .at(ctx.pos),
        &facts,
    );
    let result =
        json!({ "bill": args.bill, "stage": bill.stage.name(), "deadline": bill.deadline });
    state.touch_bills(gov);
    Ok(result)
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<BillArgs>(
        spec(
            "sign_law",
            "Sign a bill that the members passed. It becomes law at once. You need `gov.sign`.",
        ),
        move |realm, ctx, args| sign(realm, &p, ctx, args),
    );
    b.global_verb::<BillArgs>(
        spec(
            "veto_law",
            "Veto a bill that the members passed. The members can override with two thirds. You need `gov.sign`.",
        ),
        move |realm, ctx, args| veto(realm, &p, ctx, args),
    );
}
