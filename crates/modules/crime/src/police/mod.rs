//! The verbs of the police.
//!
//! An officer is a resident with a power of the law in the city where it
//! acts: `law.arrest`, `law.search` or `law.jail`. The verbs check the
//! power with the memberships that the society module writes
//! (`realm.has_perm`).
//!
//! `search` and `arrest` do their work when they start, so that the target
//! cannot walk away during the action. The officer is then busy for the
//! time that the verb costs, and the action report has the result.

use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionCtx, ActionError, Idx, Realm, Value};

use crate::powers;
use crate::record::Record;

pub(crate) mod arrest;
pub(crate) mod investigate;
pub(crate) mod patrol;
pub(crate) mod release;
pub(crate) mod search;

/// Refuses a police verb of a resident that holds none of the powers in
/// the city where it stands.
pub(crate) fn need_power(realm: &Realm, idx: Idx, perms: &[&str]) -> Result<(), ActionError> {
    if powers::has_any_power(realm, idx, perms, powers::city_of(realm)) {
        return Ok(());
    }
    Err(ActionError::not_allowed(format!(
        "You need `{}` from the government of this city.",
        perms.join("` or `")
    ))
    .with_details(json!({ "permissions": perms })))
}

/// Keeps the result of an action that did its work when it started.
pub(crate) fn keep_result(realm: &Realm, ctx: &ActionCtx, result: &Value) {
    realm.col_mut::<Record>()[ctx.idx].put_result(ctx.action_id, result);
}

/// Returns the result that [`keep_result`] kept for this action.
pub(crate) fn kept_result(realm: &Realm, ctx: &ActionCtx) -> Value {
    realm.col_mut::<Record>()[ctx.idx].take_result(ctx.action_id).unwrap_or(Value::Null)
}
