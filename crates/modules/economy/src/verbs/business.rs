//! The verbs `open_business` and `close_business`.
//!
//! A business site (a shop, a restaurant, a workshop, a factory, an office
//! or a warehouse) needs a permit before it can trade, produce or hire. The
//! permit also says which of those kinds the site is.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Dest, Holder, Money, NoArgs, Realm};

use super::Phase;
use crate::board;
use crate::events;
use crate::ids::Econ;
use crate::ownership;
use crate::site::{self, TAG_OPEN};

/// Arguments of `open_business`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct OpenBusinessArgs {
    /// What the site becomes: `shop`, `restaurant`, `workshop`, `factory`,
    /// `office` or `warehouse`. Absent: the site keeps its kind.
    #[serde(default)]
    pub kind: Option<String>,
}

/// The verb `open_business`: pays the permit fee to the city and opens the
/// site where the resident is as a business.
pub(crate) fn open_business(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &OpenBusinessArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    let kinds = econ.rules.business_kinds().join(", ");
    if !econ.rules.is_business(&here.kind) {
        return Err(ActionError::not_allowed(format!(
            "A {} cannot be a business. These sites can: {kinds}.",
            here.kind
        )));
    }
    let kind = args.kind.as_deref().unwrap_or(&here.kind);
    if !econ.rules.is_business(kind) {
        return Err(ActionError::invalid_args(format!(
            "`{kind}` is not a business kind. Use one of: {kinds}."
        )));
    }
    let open = site::is_open(&here);
    if open && kind == here.kind {
        return Err(ActionError::conflict(format!("This site is an open {kind} already.")));
    }
    let fee = realm.param(econ.ids.permit_fee);
    let me = Holder::Resident(ctx.resident);
    let cash = realm.ledger().money(me).0;
    if cash < fee {
        return Err(ActionError::insufficient("You do not have enough cash for the permit fee.")
            .with_details(json!({ "needed": fee, "available": cash })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    realm.wire_money(me, Dest::Global, site::city_holder(realm), Money(fee), "permit_fee")?;
    if open {
        // The old business ends: its prices and jobs do not fit the new
        // kind.
        ownership::close_business(realm, here.id, "converted");
    }
    if let Some(entry) = realm.sites_mut().get_mut(here.id) {
        entry.kind = kind.to_string();
        entry.add_tag(TAG_OPEN);
    }
    board::touch(realm);
    events::business_opened(realm, ctx.resident, here.id, realm.pos(ctx.idx), here.owner, kind);
    Ok(json!({ "site": here.id, "kind": kind, "fee": fee }))
}

/// The verb `close_business`: gives up the permit of the site where the
/// resident is. The price list and the jobs end. The stock and the till
/// stay.
pub(crate) fn close_business(
    realm: &Realm,
    ctx: &ActionCtx,
    _args: &NoArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    if !site::is_open(&here) {
        return Err(ActionError::conflict("This site has no permit."));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    ownership::close_business(realm, here.id, "owner");
    Ok(json!({ "site": here.id, "kind": here.kind }))
}
