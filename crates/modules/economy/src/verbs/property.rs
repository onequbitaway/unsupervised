//! The verbs `buy_property` and `sell_property`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Dest, Holder, Money, OrgId, Realm};

use super::Phase;
use crate::board;
use crate::events;
use crate::ids::Econ;
use crate::ownership;
use crate::site::{self, ATTR_PRICE, PERM_MANAGE};
use crate::tenancy;

/// Notice to the old owner of a site that was sold.
pub const NOTICE_PROPERTY_SOLD: &str = "economy.property_sold";

/// Arguments of `buy_property`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct BuyPropertyArgs {
    /// Greatest price in cents that you accept. The action fails with
    /// `conflict` when the price is higher.
    #[serde(default)]
    pub max_price: Option<i64>,
    /// Buy the site for this organization. You pay with your cash and need
    /// the permission `site.manage` there. Absent: you become the owner.
    #[serde(default)]
    pub org: Option<OrgId>,
}

/// Arguments of `sell_property`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct SellPropertyArgs {
    /// The price in cents at which the site is for sale. Absent: the site
    /// is not for sale any more.
    #[serde(default)]
    pub price: Option<i64>,
}

/// The verb `buy_property`: buys the site where the resident is.
pub(crate) fn buy_property(
    realm: &Realm,
    ctx: &ActionCtx,
    args: &BuyPropertyArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if site::is_public(&here) {
        return Err(ActionError::not_allowed("A public building is never sold."));
    }
    let buyer = match args.org {
        Some(org) => {
            if !realm.can(ctx.idx, org, PERM_MANAGE) {
                return Err(ActionError::not_allowed(
                    "You need the permission `site.manage` in that organization.",
                ));
            }
            Holder::Org(org)
        }
        None => Holder::Resident(ctx.resident),
    };
    if here.owner == Some(buyer) {
        return Err(ActionError::conflict("The buyer owns this site already."));
    }
    let Some(price) = site::asking_price(&here) else {
        return Err(ActionError::not_allowed("This site is not for sale."));
    };
    if args.max_price.is_some_and(|max| price > max) {
        return Err(ActionError::conflict(format!(
            "The site costs {price} cents. That is above your limit."
        ))
        .with_details(json!({ "price": price })));
    }
    let me = Holder::Resident(ctx.resident);
    let cash = realm.ledger().money(me).0;
    if cash < price {
        return Err(ActionError::insufficient("You do not have enough cash to buy this site.")
            .with_details(json!({ "needed": price, "available": cash })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    // Cash to a seller in this zone; a wire to a seller elsewhere and to a
    // treasury.
    match here.owner {
        Some(Holder::Resident(seller)) if realm.idx_of(seller).is_some() => {
            let seller = Holder::Resident(seller);
            realm.ledger_mut().transfer_money(me, seller, Money(price), "property")?;
        }
        Some(Holder::Resident(seller)) => {
            let to = Dest::Resident(seller);
            realm.wire_money(me, to, Holder::Resident(seller), Money(price), "property")?;
        }
        Some(holder) => realm.wire_money(me, Dest::Global, holder, Money(price), "property")?,
        None => {
            let city = site::city_holder(realm);
            realm.wire_money(me, Dest::Global, city, Money(price), "property")?;
        }
    }
    ownership::transfer(realm, here.id, buyer);
    let pos = realm.pos(ctx.idx);
    // An owner pays no rent for its own site.
    if buyer == me && tenancy::end(realm, here.id, ctx.resident) {
        events::lease_ended(realm, here.id, pos, ctx.resident, "tenant");
    }
    events::property_sold(realm, ctx.resident, pos, here.id, here.owner, buyer, price);
    if let Some(Holder::Resident(seller)) = here.owner {
        realm.notify(
            seller,
            NOTICE_PROPERTY_SOLD,
            Some(ctx.resident),
            json!({ "site": here.id, "price": price }),
        );
    }
    Ok(json!({ "site": here.id, "kind": here.kind, "price": price, "owner": buyer }))
}

/// The verb `sell_property`: offers the site where the resident is for
/// sale, or ends the offer.
pub(crate) fn sell_property(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &SellPropertyArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    if site::is_public(&here) {
        return Err(ActionError::not_allowed("A public building is never sold."));
    }
    let most = realm.param(econ.ids.price_max);
    if args.price.is_some_and(|price| price < 1 || price > most) {
        return Err(ActionError::invalid_args(format!(
            "The price must be from 1 to {most} cents. Leave it out to end the offer."
        )));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    if let Some(entry) = realm.sites_mut().get_mut(here.id) {
        match args.price {
            Some(price) => entry.set_attr(ATTR_PRICE, price),
            None => {
                entry.attrs.remove(ATTR_PRICE);
            }
        }
    }
    board::touch(realm);
    Ok(json!({ "site": here.id, "price": args.price }))
}
