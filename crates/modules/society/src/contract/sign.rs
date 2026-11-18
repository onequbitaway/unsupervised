//! `contract_sign`.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, Realm, Value};

use super::args::SignArgs;
use super::engine::{ESCROW, activate};
use super::model::{Contracts, Side};
use super::offer::{no_contract, too_many};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::{pay, spec};

/// The global part of `contract_sign`.
#[derive(Serialize, Deserialize)]
struct Sign {
    contract: u64,
}

impl Call for Sign {
    const KIND: &'static str = "society.contract_sign";
}

fn sign(realm: &Realm, p: &Params, origin: &Origin, call: Sign) -> Result<Value, ActionError> {
    let mut contracts = realm.res_mut::<Contracts>();
    let signer = origin.resident;
    let contract = contracts.open.get(&call.contract).ok_or_else(|| no_contract(call.contract))?;
    if contract.signed {
        return Err(ActionError::conflict("The contract is signed already."));
    }
    let may_sign = match contract.other {
        Some(named) => named == signer,
        None => signer != contract.offerer,
    };
    if !may_sign {
        return Err(ActionError::not_allowed("This offer is not for you."));
    }
    if contracts.count_of(signer) >= params::count(realm, p.contracts_open_max) {
        return Err(too_many(realm, p));
    }
    let part = contract.part(Side::Counterparty);
    if origin.held < part {
        return Err(ActionError::insufficient(format!(
            "Your part of this contract is {part} cents. Give at least that as `stake`."
        ))
        .with_details(json!({ "needed": part, "available": origin.held })));
    }
    let locked = pay::transfer(realm, origin.escrow(), contract.escrow(), part, ESCROW);
    activate(realm, p, &mut contracts, call.contract, signer);
    let status = if contracts.open.contains_key(&call.contract) { "active" } else { "ended" };
    Ok(json!({ "contract": call.contract, "locked": locked, "status": status }))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<SignArgs, Sign>(
        b,
        spec(
            "contract_sign",
            "Sign a contract offer. Your part of the money is locked in escrow, and the engine carries out every clause.",
        ),
        p.global_timeout,
        |_realm, _ctx, args| {
            if args.stake < 0 {
                return Err(ActionError::invalid_args("`stake` must not be negative."));
            }
            Ok((Sign { contract: args.contract }, args.stake))
        },
        move |realm, origin, call| sign(realm, &p, origin, call),
    );
}
