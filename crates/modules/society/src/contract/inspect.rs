//! `contract_view`: read a contract before the signing, or later.
//!
//! An offer names the resident that can sign, and that resident gets the
//! clauses in a notice. An offer without a name is for everyone, so
//! everyone can read it here. A signed contract is private to its parties.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ActionError, Builder, GlobalCtx, Realm, Value, VerbSpec};

use super::args::ContractArgs;
use super::model::{Contracts, Side};
use super::offer::no_contract;
use super::view;
use crate::spec;

fn read(realm: &Realm, ctx: &GlobalCtx, args: &ContractArgs) -> Result<Value, ActionError> {
    let contracts = realm.res::<Contracts>();
    let contract = contracts.open.get(&args.contract).ok_or_else(|| no_contract(args.contract))?;
    let side = contract.side_of(ctx.resident);
    let for_everyone = !contract.signed && contract.other.is_none();
    if side.is_none() && !for_everyone {
        return Err(ActionError::not_allowed("This contract is not for you."));
    }
    // A reader that is no party yet reads the offer as the resident that
    // would sign it.
    let side = side.unwrap_or(Side::Counterparty);
    let mut out = Map::new();
    out.insert("contract".into(), json!(contract.id));
    out.insert("offerer".into(), json!(contract.offerer));
    out.insert("you_are".into(), json!(side.name()));
    out.insert("your_part".into(), json!(contract.part(side)));
    if contract.signed {
        out.insert("status".into(), json!("active"));
        if let Some(other) = contract.other {
            out.insert("counterparty".into(), json!(other));
        }
    } else {
        out.insert("status".into(), json!("offered"));
        out.insert("expires".into(), json!(contract.expires));
    }
    out.insert("clauses".into(), view::clauses(realm, contract));
    Ok(Value::Object(out))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder) {
    b.global_verb::<ContractArgs>(
        VerbSpec {
            free: true,
            ..spec(
                "contract_view",
                "Read the clauses of a contract of yours, or of an offer that you can sign.",
            )
        },
        read,
    );
}
