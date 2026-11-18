//! Leaving goods in a site and taking them back: the verbs `drop` and
//! `pick_up`.
//!
//! Goods that lie in a site are the stock of the site
//! (`Holder::Site`). Everybody can leave goods in a site. Only a resident
//! that owns or rents the site, or that manages it for the organization
//! that owns it, can take goods out.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionCtx, ActionError, Begin, Builder, Holder, ItemId, Realm, Site, VerbSpec};

use crate::handles::Handles;
use crate::sites::site_here;

/// The permission that lets a member of an organization take goods out of
/// a site of the organization.
const SITE_MANAGE: &str = "site.manage";

/// Arguments of `drop` and `pick_up`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct CarryArgs {
    /// Name of the item.
    pub item: String,
    /// Units of the item. 1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<i64>,
}

/// Checks the arguments and the place. Returns the site where the resident
/// stands, the item and the quantity.
fn check(
    realm: &Realm,
    ctx: &ActionCtx,
    args: &CarryArgs,
) -> Result<(Site, ItemId, i64), ActionError> {
    let qty = args.qty.unwrap_or(1);
    if qty < 1 {
        return Err(ActionError::invalid_args("`qty` must be at least 1."));
    }
    let Some(item) = realm.defs().item_id(&args.item) else {
        return Err(ActionError::not_found(format!("No item is named `{}`.", args.item)));
    };
    let Some(place) = site_here(realm, ctx.idx) else {
        return Err(ActionError::not_here("You must be inside a site."));
    };
    Ok((place, item, qty))
}

/// Returns true when a resident can take goods out of a site.
fn can_take(realm: &Realm, ctx: &ActionCtx, place: &Site) -> bool {
    match place.owner {
        Some(Holder::Resident(owner)) if owner == ctx.resident => true,
        Some(Holder::Org(org)) if realm.can(ctx.idx, org, SITE_MANAGE) => true,
        _ => place.is_tenant(ctx.resident),
    }
}

/// Registers the verbs `drop` and `pick_up`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<CarryArgs>(
        VerbSpec {
            verb: "drop",
            module: "life",
            summary: "Leave goods that you carry in the site where you stand. They join the stock of the site.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| {
            let (place, item, qty) = check(realm, ctx, args)?;
            realm.ledger_mut().transfer_item(
                Holder::Resident(ctx.resident),
                Holder::Site(place.id),
                item,
                qty,
                "drop",
            )?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.carry_ticks) })
        },
        |_realm, _ctx, args| Ok(json!({ "item": args.item, "qty": args.qty.unwrap_or(1) })),
    );
    b.verb::<CarryArgs>(
        VerbSpec {
            verb: "pick_up",
            module: "life",
            summary: "Take goods from the stock of the site where you stand. You must own, rent or manage the site.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| {
            let (place, item, qty) = check(realm, ctx, args)?;
            if !can_take(realm, ctx, &place) {
                return Err(ActionError::not_allowed(format!(
                    "You do not own, rent or manage {}.",
                    place.name
                )));
            }
            realm.ledger_mut().transfer_item(
                Holder::Site(place.id),
                Holder::Resident(ctx.resident),
                item,
                qty,
                "pick_up",
            )?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.carry_ticks) })
        },
        |_realm, _ctx, args| Ok(json!({ "item": args.item, "qty": args.qty.unwrap_or(1) })),
    );
}
