//! The verbs of renting: `rent_out`, `rent` and `end_lease`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Holder, Money, Realm, ResidentId};

use super::Phase;
use crate::board;
use crate::events;
use crate::ids::Econ;
use crate::site::{self, ATTR_RENT};
use crate::state::leases::{Lease, Leases};
use crate::state::ties::{LeaseTie, TieChange, Ties};
use crate::tenancy::{self, NOTICE_LEASE_ENDED};

/// Notice to the owner of a home when a tenant moves in.
pub const NOTICE_TENANT_MOVED_IN: &str = "economy.tenant_moved_in";

/// Arguments of `rent_out`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct RentOutArgs {
    /// Rent in cents for one day at which the site takes new tenants.
    /// Absent: the site takes no new tenants. The leases that run go on.
    #[serde(default)]
    pub rent: Option<i64>,
}

/// Arguments of `rent`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct RentArgs {
    /// Greatest rent in cents for one day that you accept. The action
    /// fails with `conflict` when the rent is higher.
    #[serde(default)]
    pub max_rent: Option<i64>,
}

/// Arguments of `end_lease`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct EndLeaseArgs {
    /// The tenant whose lease ends. Only the owner of the site can name
    /// one. Absent: your own lease ends.
    #[serde(default)]
    pub tenant: Option<ResidentId>,
}

/// The verb `rent_out`: offers the home where the resident is to tenants,
/// or ends the offer.
pub(crate) fn rent_out(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &RentOutArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    if !econ.rules.kind(&here.kind).lettable {
        return Err(ActionError::not_allowed(format!("A {} takes no tenants.", here.kind)));
    }
    let most = realm.param(econ.ids.price_max);
    if args.rent.is_some_and(|rent| rent < 1 || rent > most) {
        return Err(ActionError::invalid_args(format!(
            "The rent must be from 1 to {most} cents. Leave it out to end the offer."
        )));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    if let Some(entry) = realm.sites_mut().get_mut(here.id) {
        match args.rent {
            Some(rent) => entry.set_attr(ATTR_RENT, rent),
            None => {
                entry.attrs.remove(ATTR_RENT);
            }
        }
    }
    board::touch(realm);
    Ok(json!({ "site": here.id, "rent": args.rent }))
}

/// The verb `rent`: becomes a tenant of the home where the resident is and
/// pays the rent of the first day.
pub(crate) fn rent(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &RentArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let rule = econ.rules.kind(&here.kind);
    let offer = site::asking_rent(&here, rule, realm.param(econ.ids.city_rent_bp));
    let Some(rent) = offer else {
        return Err(ActionError::not_allowed("This site takes no tenants."));
    };
    if here.owner == Some(Holder::Resident(ctx.resident)) {
        return Err(ActionError::conflict("You own this site."));
    }
    if here.is_tenant(ctx.resident) {
        return Err(ActionError::conflict("You rent this site already."));
    }
    if here.tenants.len() >= site::units(&here, rule) {
        return Err(ActionError::conflict("This site has no free unit."));
    }
    if args.max_rent.is_some_and(|max| rent > max) {
        return Err(ActionError::conflict(format!(
            "The rent is {rent} cents for one day. That is above your limit."
        ))
        .with_details(json!({ "rent": rent })));
    }
    let me = Holder::Resident(ctx.resident);
    let cash = realm.ledger().money(me).0;
    if cash < rent {
        return Err(ActionError::insufficient("You do not have the rent of the first day.")
            .with_details(json!({ "needed": rent, "available": cash })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    realm.ledger_mut().transfer_money(me, Holder::Site(here.id), Money(rent), "rent")?;
    if let Some(entry) = realm.sites_mut().get_mut(here.id) {
        entry.add_tenant(ctx.resident);
    }
    let lease = Lease { tenant: ctx.resident, rent, owed: 0, overdue: 0, asked: false };
    realm.res_mut::<Leases>().add(here.id, lease);
    let zone = realm.zone_info().map(|zone| zone.id).unwrap_or_default();
    let tie = LeaseTie { site: here.id, zone, rent };
    realm.col_mut::<Ties>()[ctx.idx].apply(TieChange::Leased(tie));
    board::touch(realm);
    events::lease_started(realm, here.id, realm.pos(ctx.idx), ctx.resident, rent);
    if let Some(Holder::Resident(owner)) = here.owner {
        realm.notify(
            owner,
            NOTICE_TENANT_MOVED_IN,
            Some(ctx.resident),
            json!({ "site": here.id, "rent": rent }),
        );
    }
    Ok(json!({ "site": here.id, "rent": rent }))
}

/// The verb `end_lease`: a tenant leaves, or the owner ends the lease of a
/// tenant.
pub(crate) fn end_lease(
    realm: &Realm,
    ctx: &ActionCtx,
    args: &EndLeaseArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let (tenant, by) = match args.tenant {
        Some(tenant) if tenant != ctx.resident => {
            if !site::controls(realm, ctx.idx, &here) {
                return Err(site::not_controller());
            }
            (tenant, "owner")
        }
        _ => (ctx.resident, "tenant"),
    };
    if realm.res::<Leases>().get(here.id, tenant).is_none() {
        return Err(ActionError::not_found(if by == "tenant" {
            "You do not rent this site."
        } else {
            "That resident does not rent this site."
        }));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    tenancy::end(realm, here.id, tenant);
    events::lease_ended(realm, here.id, realm.pos(ctx.idx), tenant, by);
    if by == "owner" {
        realm.notify(tenant, NOTICE_LEASE_ENDED, Some(ctx.resident), json!({ "site": here.id }));
    }
    Ok(json!({ "site": here.id, "tenant": tenant }))
}
