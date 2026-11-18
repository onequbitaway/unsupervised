//! The verbs `till_collect` and `till_fund`: move cash between the till of
//! a site and its owner.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Dest, Holder, Money, Realm};

use super::Phase;
use crate::ids::Econ;
use crate::site;

/// Arguments of `till_collect`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct TillCollectArgs {
    /// Cents to take out of the till. Everything when absent.
    #[serde(default)]
    pub amount: Option<i64>,
}

/// Arguments of `till_fund`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TillFundArgs {
    /// Cents to put into the till.
    pub amount: i64,
}

fn check_amount(econ: &Econ, realm: &Realm, amount: i64) -> Result<(), ActionError> {
    let most = realm.param(econ.ids.price_max);
    if amount < 1 || amount > most {
        return Err(ActionError::invalid_args(format!("`amount` must be from 1 to {most} cents.")));
    }
    Ok(())
}

/// The verb `till_collect`: takes cash out of the till. The money of a site
/// that an organization owns goes to the treasury of the organization.
pub(crate) fn till_collect(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &TillCollectArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    let till = Holder::Site(here.id);
    let available = realm.ledger().money(till).0;
    let amount = args.amount.unwrap_or(available);
    if args.amount.is_some() {
        check_amount(econ, realm, amount)?;
    }
    if amount > available || amount < 1 {
        return Err(ActionError::insufficient("The till has less money.")
            .with_details(json!({ "needed": amount, "available": available })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let to = match here.owner {
        Some(Holder::Org(org)) => {
            let treasury = Holder::Org(org);
            realm.wire_money(till, Dest::Global, treasury, Money(amount), "till_collect")?;
            treasury
        }
        _ => {
            let me = Holder::Resident(ctx.resident);
            realm.ledger_mut().transfer_money(till, me, Money(amount), "till_collect")?;
            me
        }
    };
    Ok(json!({ "site": here.id, "amount": amount, "to": to, "till": available - amount }))
}

/// The verb `till_fund`: puts cash of the resident into the till.
pub(crate) fn till_fund(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &TillFundArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    check_amount(econ, realm, args.amount)?;
    let me = Holder::Resident(ctx.resident);
    let cash = realm.ledger().money(me).0;
    if cash < args.amount {
        return Err(ActionError::insufficient("You do not have enough cash.")
            .with_details(json!({ "needed": args.amount, "available": cash })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let till = Holder::Site(here.id);
    realm.ledger_mut().transfer_money(me, till, Money(args.amount), "till_fund")?;
    let now = realm.ledger().money(till).0;
    Ok(json!({ "site": here.id, "amount": args.amount, "till": now }))
}
