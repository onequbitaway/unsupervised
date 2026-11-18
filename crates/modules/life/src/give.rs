//! Hand-to-hand transfer: the verb `give`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, EventMeta, Holder, Money, ResidentId, VerbSpec};

use crate::handles::Handles;

/// Kind of the notice that the receiver of a gift gets.
pub const GIFT_RECEIVED: &str = "life.gift_received";

/// Arguments of `give`. Give `money`, or `item` and `qty`, or both.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct GiveArgs {
    /// The resident that gets the gift. It must stand on your tile or on a
    /// tile next to it.
    pub to: ResidentId,
    /// Cents of cash to give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub money: Option<i64>,
    /// Name of the item to give.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Units of the item to give. 1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qty: Option<i64>,
}

/// Registers the verb `give`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<GiveArgs>(
        VerbSpec {
            verb: "give",
            module: "life",
            summary: "Give cash or goods to a resident on your tile or next to you.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| {
            if args.to == ctx.resident {
                return Err(ActionError::invalid_args("You cannot give to yourself."));
            }
            let money = args.money.unwrap_or(0);
            if money < 0 {
                return Err(ActionError::invalid_args("`money` must not be negative."));
            }
            let goods = match &args.item {
                Some(name) => {
                    let qty = args.qty.unwrap_or(1);
                    if qty < 1 {
                        return Err(ActionError::invalid_args("`qty` must be at least 1."));
                    }
                    let Some(item) = realm.defs().item_id(name) else {
                        return Err(ActionError::not_found(format!("No item is named `{name}`.")));
                    };
                    Some((item, qty))
                }
                None if args.qty.is_some() => {
                    return Err(ActionError::invalid_args("`qty` needs `item`."));
                }
                None => None,
            };
            if money == 0 && goods.is_none() {
                return Err(ActionError::invalid_args("Give `money`, or `item` and `qty`."));
            }
            let range = u32::try_from(realm.param(h.p.give_range)).unwrap_or(0);
            let here = realm.pos(ctx.idx);
            let near =
                realm.idx_of(args.to).is_some_and(|other| realm.pos(other).dist(here) <= range);
            if !near {
                return Err(ActionError::not_here(format!(
                    "Resident {} is not next to you.",
                    args.to
                )));
            }
            let (from, to) = (Holder::Resident(ctx.resident), Holder::Resident(args.to));
            {
                // Check both parts first, so that a gift is whole or not at
                // all.
                let mut ledger = realm.ledger_mut();
                let cash = ledger.money(from).cents();
                if cash < money {
                    return Err(ActionError::insufficient("You do not have that much cash.")
                        .with_details(json!({ "needed": money, "available": cash })));
                }
                if ledger.money(to).cents().checked_add(money).is_none() {
                    return Err(ActionError::invalid_args("The amount is too large."));
                }
                if let Some((item, qty)) = goods {
                    let carried = ledger.qty(from, item);
                    if carried < qty {
                        return Err(ActionError::insufficient("You do not carry that many.")
                            .with_details(json!({ "needed": qty, "available": carried })));
                    }
                    if ledger.qty(to, item).checked_add(qty).is_none() {
                        return Err(ActionError::invalid_args("The quantity is too large."));
                    }
                    ledger.transfer_item(from, to, item, qty, "gift")?;
                }
                ledger.transfer_money(from, to, Money(money), "gift")?;
            }
            let gift = json!({
                "from": ctx.resident,
                "to": args.to,
                "money": money,
                "item": args.item,
                "qty": goods.map_or(0, |(_, qty)| qty),
            });
            realm.notify(args.to, GIFT_RECEIVED, Some(ctx.resident), gift.clone());
            realm.emit(
                EventMeta::public("life.gift_given", h.drama(realm, h.p.drama_gift))
                    .actor(ctx.resident)
                    .at(here),
                &gift,
            );
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.give_ticks) })
        },
        |_realm, ctx, args| {
            Ok(json!({
                "to": args.to,
                "money": args.money.unwrap_or(0),
                "item": args.item,
                "qty": args.item.as_ref().map_or(0, |_| args.qty.unwrap_or(1)),
                "from": ctx.resident,
            }))
        },
    );
}
