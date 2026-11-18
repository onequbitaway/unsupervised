//! The treasury verbs. `org_deposit` moves cash of a resident to the
//! treasury. `org_pay` moves money of the treasury to a resident or to
//! another organization.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Builder, EventMeta, Holder, Money, OrgId, Realm, ResidentId, Value,
};

use super::args::{OrgDepositArgs, OrgPayArgs};
use super::mirror::describe_with;
use super::state::Orgs;
use super::verbs::{no_org, require};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::{pay, perms, spec};

/// Ledger label of a deposit into a treasury.
const DEPOSIT: &str = "org.deposit";
/// Ledger label of a payment from a treasury.
const PAY: &str = "org.pay";

/// The global part of `org_deposit`.
#[derive(Serialize, Deserialize)]
struct Deposit {
    org: OrgId,
}

impl Call for Deposit {
    const KIND: &'static str = "society.org_deposit";
}

/// Who gets a payment from a treasury.
#[derive(Clone, Copy, Serialize, Deserialize)]
enum Payee {
    Resident(ResidentId),
    Org(OrgId),
}

/// The global part of `org_pay`.
#[derive(Serialize, Deserialize)]
struct Pay {
    org: OrgId,
    amount: i64,
    to: Payee,
}

impl Call for Pay {
    const KIND: &'static str = "society.org_pay";
}

fn deposit(
    realm: &Realm,
    p: &Params,
    origin: &Origin,
    call: Deposit,
) -> Result<Value, ActionError> {
    let orgs = realm.res::<Orgs>();
    let org = orgs.get(call.org).ok_or_else(|| no_org(call.org))?;
    let amount = pay::transfer(realm, origin.escrow(), Holder::Org(call.org), origin.held, DEPOSIT);
    realm.emit(
        EventMeta::public("society.org_deposited", params::drama(realm, p.drama_treasury))
            .actor(origin.resident)
            .at(origin.pos),
        &describe_with(org, json!({ "from": origin.resident, "amount": amount })),
    );
    Ok(json!({ "org": call.org, "amount": amount }))
}

fn pay_out(realm: &Realm, p: &Params, origin: &Origin, call: Pay) -> Result<Value, ActionError> {
    let orgs = realm.res::<Orgs>();
    let org = orgs.get(call.org).ok_or_else(|| no_org(call.org))?;
    // The zone checked its copy of the memberships. The registry decides.
    require(org, origin.resident, perms::TREASURY_SPEND)?;
    let treasury = Holder::Org(call.org);
    let available = realm.ledger().money(treasury).cents();
    if available < call.amount {
        return Err(ActionError::insufficient("The treasury does not have that much.")
            .with_details(json!({ "needed": call.amount, "available": available })));
    }
    let to = match call.to {
        Payee::Resident(resident) => {
            if !realm.directory().is_alive(resident) {
                return Err(ActionError::not_found(format!("No resident has the id {resident}.")));
            }
            pay::wire_to_resident(realm, treasury, resident, call.amount, PAY);
            realm.notify(
                resident,
                "society.org_payment",
                Some(origin.resident),
                describe_with(org, json!({ "amount": call.amount })),
            );
            Holder::Resident(resident)
        }
        Payee::Org(other) => {
            if other == call.org {
                return Err(ActionError::invalid_args("An organization cannot pay itself."));
            }
            if orgs.get(other).is_none() {
                return Err(no_org(other));
            }
            pay::transfer(realm, treasury, Holder::Org(other), call.amount, PAY);
            Holder::Org(other)
        }
    };
    realm.emit(
        EventMeta::public("society.org_paid", params::drama(realm, p.drama_treasury))
            .actor(origin.resident)
            .at(origin.pos),
        &describe_with(org, json!({ "by": origin.resident, "to": to, "amount": call.amount })),
    );
    Ok(json!({
        "org": call.org,
        "amount": call.amount,
        "treasury": realm.ledger().money(treasury),
    }))
}

/// Checks that an amount of money is greater than zero.
pub(crate) fn positive(amount: i64) -> Result<(), ActionError> {
    if Money(amount).is_positive() {
        Ok(())
    } else {
        Err(ActionError::invalid_args("The amount must be greater than zero."))
    }
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<OrgDepositArgs, Deposit>(
        b,
        spec("org_deposit", "Put cash that you carry in the treasury of an organization."),
        p.global_timeout,
        |_realm, _ctx, args| {
            positive(args.amount)?;
            Ok((Deposit { org: args.org }, args.amount))
        },
        move |realm, origin, call| deposit(realm, &p, origin, call),
    );
    remote_verb::<OrgPayArgs, Pay>(
        b,
        spec(
            "org_pay",
            "Pay from the treasury of an organization to a resident or to another organization. You need `treasury.spend`.",
        ),
        p.global_timeout,
        |realm, ctx, args| {
            positive(args.amount)?;
            let to = match (args.to_resident, args.to_org) {
                (Some(resident), None) => Payee::Resident(resident),
                (None, Some(org)) => Payee::Org(org),
                _ => {
                    return Err(ActionError::invalid_args(
                        "Give `to_resident` or `to_org`, and not both.",
                    ));
                }
            };
            if !realm.can(ctx.idx, args.org, perms::TREASURY_SPEND) {
                return Err(ActionError::not_allowed(format!(
                    "You need the permission `treasury.spend` in organization {}.",
                    args.org
                )));
            }
            Ok((Pay { org: args.org, amount: args.amount, to }, 0))
        },
        move |realm, origin, call| pay_out(realm, &p, origin, call),
    );
}
