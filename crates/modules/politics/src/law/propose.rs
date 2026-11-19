//! The verb `propose_law`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, CityId, EventMeta, GlobalCtx, Realm, Value};

use super::{facts_with, reason_of};
use crate::calendar;
use crate::model::Gov;
use crate::offices::{self, GOV_PROPOSE, GOV_VOTE};
use crate::params::{self, Params};
use crate::remote::spec;
use crate::state::Politics;
use crate::state::bill::{Bill, Stage};
use crate::text;

/// Arguments of `propose_law`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct ProposeArgs {
    /// Name of a policy parameter, for example `economy.sales_tax_bp`. The
    /// catalogue marks the policy parameters and gives their bounds.
    pub(crate) param: String,
    /// The new value. It must be inside the bounds of the parameter.
    pub(crate) value: i64,
    /// The city whose council decides. The law then changes the parameter
    /// in that city only. Leave it out for a national law, which the
    /// legislature decides.
    #[serde(default)]
    pub(crate) city: Option<CityId>,
    /// A short text that says why. It is optional.
    #[serde(default)]
    pub(crate) reason: Option<String>,
}

fn run(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &ProposeArgs,
) -> Result<Value, ActionError> {
    let gov = match args.city {
        Some(city) if realm.layout().city(city).is_none() => {
            return Err(ActionError::not_found(format!("No city has the id {city}.")));
        }
        Some(city) => Gov::City(city),
        None => Gov::Nation,
    };
    offices::require(realm, gov, ctx.resident, GOV_PROPOSE)?;
    let defs = realm.defs();
    let Some(param) = defs.param_id(&args.param) else {
        return Err(ActionError::not_found(format!("No parameter is named `{}`.", args.param)));
    };
    let def = defs.param(param);
    if !def.policy {
        return Err(ActionError::not_allowed(format!(
            "`{}` is not a policy parameter. A law cannot change it.",
            args.param
        )));
    }
    if args.value < def.min || args.value > def.max {
        return Err(ActionError::invalid_args(format!(
            "The value of `{}` must be from {} to {}.",
            args.param, def.min, def.max
        ))
        .with_details(json!({ "min": def.min, "max": def.max })));
    }
    if let Some(reason) = &args.reason {
        text::check_text(reason, realm.param(p.bill_text_max), "reason")?;
    }
    let mut state = realm.res_mut::<Politics>();
    let max = params::count(realm, p.bills_open_max);
    if state.bills_of(gov).count() >= max {
        return Err(ActionError::conflict(format!(
            "This government has too many open bills. The limit is {max}."
        )));
    }
    if let Some(open) = state.bills_of(gov).find(|bill| bill.param == args.param) {
        return Err(ActionError::conflict(format!(
            "Bill {} changes this parameter and is still open.",
            open.id
        ))
        .with_details(json!({ "bill": open.id })));
    }
    let frame = realm.frame();
    let minutes = u64::try_from(realm.param(p.bill_voting_minutes).max(1)).unwrap_or(1);
    let members = offices::members_with(realm, gov.org(), GOV_VOTE);
    let bill = Bill {
        id: realm.alloc_id(),
        gov,
        param: args.param.clone(),
        value: args.value,
        old: params::value(realm, param, gov),
        sponsor: ctx.resident,
        reason: args.reason.clone(),
        proposed: frame,
        stage: Stage::Vote,
        deadline: frame + minutes,
        // The members of this moment vote on the bill.
        electorate: Some(members.iter().copied().collect()),
        yes: BTreeSet::new(),
        no: BTreeSet::new(),
        passed: None,
        vetoed_by: None,
    };
    let facts = facts_with(
        &bill,
        json!({
            "current": bill.old,
            "sponsor": bill.sponsor,
            "reason": reason_of(&bill),
            "deadline": calendar::moment(realm, bill.deadline),
        }),
    );
    for member in members {
        if member != ctx.resident {
            realm.notify(member, "politics.bill_proposed", Some(ctx.resident), facts.clone());
        }
    }
    realm.emit(
        EventMeta::public("politics.bill_proposed", params::drama(realm, p.drama_bill_proposed))
            .actor(ctx.resident)
            .at(ctx.pos),
        &facts,
    );
    let result =
        facts_with(&bill, json!({ "stage": bill.stage.name(), "deadline": bill.deadline }));
    state.bills.insert(bill.id, bill);
    state.touch_bills(gov);
    Ok(result)
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<ProposeArgs>(
        spec(
            "propose_law",
            "Propose a law: a new value for a policy parameter, for a city or the nation. You need `gov.propose`.",
        ),
        move |realm, ctx, args| run(realm, &p, ctx, args),
    );
}
