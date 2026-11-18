//! The verb `fence`: stolen goods become clean cash, at a discount.
//!
//! The goods leave the world through the sink `export.sale` and the money
//! enters it through the faucet `export.sale`: the same pair that the
//! depot of the economy module uses when it buys goods. The fence pays a
//! share of the depot price, so a thief gets less than an honest seller
//! and no chain through the fence makes more money than the depot does.
//!
//! The depot does not trade contraband. With the default numbers the
//! fence does not buy it either: a system buyer for contraband would be a
//! new source of money. The parameter `crime.fence_base_bp` can open one.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, EventMeta, Holder, Idx, ItemId, Money, Realm};

use super::spec;
use crate::calc;
use crate::hot;
use crate::ids::Handles;
use crate::loot;
use crate::names::{event, flow, tag};
use crate::powers;
use crate::record::Record;

/// Arguments of `fence`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct FenceArgs {
    /// Name of the stolen good to sell.
    pub item: String,
    /// Units to sell. Without it, you sell every stolen unit of the good
    /// that you still hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<i64>,
}

/// Returns the cents that the fence pays for one unit of a good:
/// `crime.fence_bp` of the price that the depot pays. For a good that the
/// depot does not trade, the reference is `crime.fence_base_bp` of the
/// normal price of the good; that share is 0 by default.
pub(crate) fn unit_price(realm: &Realm, h: &Handles, item: ItemId) -> i64 {
    let defs = realm.defs();
    let floor = defs.param_id(&format!("{}{}", flow::DEPOT_FLOOR, defs.item_name(item)));
    let reference = match floor {
        Some(param) => realm.param(param),
        None => calc::share(loot::unit_value(realm, item), realm.param(h.p.fence_base_share)),
    };
    calc::share(reference, realm.param(h.p.fence_share))
}

/// What a sale to the fence would be.
struct Deal {
    item: ItemId,
    qty: i64,
    price: i64,
}

/// Checks a sale to the fence.
fn plan(realm: &Realm, h: &Handles, idx: Idx, args: &FenceArgs) -> Result<Deal, ActionError> {
    if args.qty.is_some_and(|qty| qty < 1) {
        return Err(ActionError::invalid_args("`qty` must be at least 1."));
    }
    let Some(item) = realm.defs().item_id(&args.item) else {
        return Err(ActionError::not_found(format!("No item is named `{}`.", args.item)));
    };
    if powers::site_here(realm, idx).is_some_and(|site| site.has_tag(tag::PUBLIC)) {
        return Err(ActionError::not_here("No fence deals in a public building. Go elsewhere."));
    }
    hot::settle(realm, h, idx);
    let stolen = realm.col::<Record>()[idx].hot_of(Some(item));
    if stolen < 1 {
        return Err(ActionError::insufficient(
            "You hold no stolen units of this good. A fence buys only stolen goods.",
        ));
    }
    let price = unit_price(realm, h, item);
    if price < 1 {
        return Err(ActionError::not_allowed("No fence pays for this good."));
    }
    Ok(Deal { item, qty: args.qty.unwrap_or(stolen).min(stolen), price })
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<FenceArgs>(
        spec(
            "fence",
            "Sell stolen goods that you hold for clean cash, below the price of the depot. Not in a public building.",
        ),
        move |realm, ctx, args| {
            plan(realm, &h, ctx.idx, args)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.fence_ticks) })
        },
        move |realm, ctx, args| {
            let deal = plan(realm, &h, ctx.idx, args)?;
            let holder = Holder::Resident(ctx.resident);
            let paid = deal.price.saturating_mul(deal.qty);
            {
                let mut ledger = realm.ledger_mut();
                ledger.consume(holder, h.fenced_goods, deal.item, deal.qty)?;
                ledger.mint(h.fence_money, holder, Money(paid));
            }
            realm.col_mut::<Record>()[ctx.idx].take_hot(Some(deal.item), deal.qty);
            // The event has no actor: a sale to a fence is secret.
            realm.emit(
                EventMeta::public(event::FENCED, h.drama(realm, h.p.drama_fenced))
                    .at(realm.pos(ctx.idx)),
                &json!({ "city": powers::city_of(realm), "value": paid }),
            );
            Ok(json!({
                "item": args.item, "qty": deal.qty, "unit_price": deal.price, "paid": paid,
            }))
        },
    );
}
