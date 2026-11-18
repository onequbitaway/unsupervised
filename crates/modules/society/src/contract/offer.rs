//! `contract_offer`, `contract_cancel` and `contract_reject`.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, GlobalCtx, Realm, ResidentId, Value};

use super::args::{ContractArgs, OfferArgs, check_clauses, part_of};
use super::engine::{ESCROW, close_offer};
use super::model::{Clause, Contract, Contracts, Side};
use super::view;
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::{pay, spec};

/// The global part of `contract_offer`.
#[derive(Serialize, Deserialize)]
struct Offer {
    to: Option<ResidentId>,
    clauses: Vec<Clause>,
    expires_in: u32,
}

impl Call for Offer {
    const KIND: &'static str = "society.contract_offer";
}

/// The error for a resident with too many open contracts.
pub(crate) fn too_many(realm: &Realm, p: &Params) -> ActionError {
    ActionError::conflict(format!(
        "You have too many open contracts. The limit is {}.",
        params::count(realm, p.contracts_open_max)
    ))
}

/// The error for a contract id that the engine does not have.
pub(crate) fn no_contract(id: u64) -> ActionError {
    ActionError::not_found(format!("No open contract has the id {id}."))
}

fn offer(realm: &Realm, p: &Params, origin: &Origin, call: Offer) -> Result<Value, ActionError> {
    let mut contracts = realm.res_mut::<Contracts>();
    if let Some(to) = call.to
        && !realm.directory().is_alive(to)
    {
        return Err(ActionError::not_found(format!("No resident has the id {to}.")));
    }
    if contracts.count_of(origin.resident) >= params::count(realm, p.contracts_open_max) {
        return Err(too_many(realm, p));
    }
    let id = realm.alloc_id();
    let contract = Contract {
        id,
        offerer: origin.resident,
        other: call.to,
        signed: false,
        clauses: call.clauses,
        expires: realm.frame() + u64::from(call.expires_in),
    };
    let locked = pay::transfer(realm, origin.escrow(), contract.escrow(), origin.held, ESCROW);
    let your_part = contract.part(Side::Counterparty);
    if let Some(to) = call.to {
        realm.notify(
            to,
            "society.contract_offered",
            Some(origin.resident),
            json!({
                "contract": id,
                "your_part": your_part,
                "expires": contract.expires,
                "clauses": view::clauses(realm, &contract),
            }),
        );
    }
    let expires = contract.expires;
    contracts.add_offer(contract);
    Ok(json!({ "contract": id, "locked": locked, "expires": expires }))
}

/// Finds an offer that is not signed.
fn open_offer(contracts: &Contracts, id: u64) -> Result<&Contract, ActionError> {
    match contracts.open.get(&id) {
        Some(contract) if !contract.signed => Ok(contract),
        Some(_) => Err(ActionError::conflict("The contract is signed. Nobody can stop it now.")),
        None => Err(no_contract(id)),
    }
}

fn cancel(realm: &Realm, ctx: &GlobalCtx, args: &ContractArgs) -> Result<Value, ActionError> {
    let mut contracts = realm.res_mut::<Contracts>();
    if open_offer(&contracts, args.contract)?.offerer != ctx.resident {
        return Err(ActionError::not_allowed(
            "Only the resident that made the offer can cancel it.",
        ));
    }
    close_offer(realm, &mut contracts, args.contract, "cancelled", Some(ctx.resident));
    Ok(json!({ "contract": args.contract, "status": "cancelled" }))
}

fn reject(realm: &Realm, ctx: &GlobalCtx, args: &ContractArgs) -> Result<Value, ActionError> {
    let mut contracts = realm.res_mut::<Contracts>();
    if open_offer(&contracts, args.contract)?.other != Some(ctx.resident) {
        return Err(ActionError::not_allowed(
            "Only the resident that the offer names can reject it.",
        ));
    }
    close_offer(realm, &mut contracts, args.contract, "rejected", Some(ctx.resident));
    Ok(json!({ "contract": args.contract, "status": "rejected" }))
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<OfferArgs, Offer>(
        b,
        spec(
            "contract_offer",
            "Offer a contract with clauses. Your part of the money is locked in escrow at once.",
        ),
        p.global_timeout,
        move |realm, ctx, args| {
            if args.to == Some(ctx.resident) {
                return Err(ActionError::invalid_args("You cannot make a contract with yourself."));
            }
            let clauses = check_clauses(realm, &p, &args.clauses)?;
            let ttl_max = realm.param(p.contract_offer_ttl_max);
            let expires_in = match args.expires_in {
                Some(minutes) => i64::from(minutes),
                None => realm.param(p.contract_offer_ttl).min(ttl_max),
            };
            if expires_in < 1 || expires_in > ttl_max {
                return Err(ActionError::invalid_args(format!(
                    "`expires_in` must be from 1 to {ttl_max} game minutes."
                )));
            }
            // Both sums must fit, also the one that the signer will lock.
            part_of(&clauses, Side::Counterparty)?;
            let part = part_of(&clauses, Side::Offerer)?;
            let expires_in = u32::try_from(expires_in).unwrap_or(u32::MAX);
            Ok((Offer { to: args.to, clauses, expires_in }, part))
        },
        move |realm, origin, call| offer(realm, &p, origin, call),
    );
    b.global_verb::<ContractArgs>(
        spec("contract_cancel", "Take back a contract offer of yours that nobody signed. You get your locked money back."),
        cancel,
    );
    b.global_verb::<ContractArgs>(
        spec("contract_reject", "Refuse a contract offer that names you."),
        reject,
    );
}
