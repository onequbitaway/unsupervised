//! `order_cancel` and `market_claim`: take back what an order holds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Builder, Holder, Idx, ItemId, Money, NoArgs, Realm, ResidentId};

use super::state::Exchange;
use super::{EXCHANGE_SITE, busy};
use crate::params::Params;
use crate::util::{require_site, spec};

/// Arguments of `order_cancel`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct OrderCancelArgs {
    /// Id of the order, from the result of `order_place` or from the
    /// `market` section.
    pub order: u64,
}

/// What a trader collected: cash and goods by item.
#[derive(Default)]
pub(crate) struct Collected {
    pub(crate) cash: i64,
    pub(crate) items: BTreeMap<ItemId, i64>,
}

impl Collected {
    /// Writes the collection as JSON: cash, and goods by item name.
    pub(crate) fn to_value(&self, realm: &Realm) -> Value {
        let items: BTreeMap<&str, i64> =
            self.items.iter().map(|(&item, &qty)| (realm.defs().item_name(item), qty)).collect();
        json!({ "cash": self.cash, "items": items })
    }
}

/// Moves what an order holds for its trader from the escrow to `to`, a
/// holder of this zone. With `close`, the order first leaves the book, so
/// everything in the escrow moves. An order that is closed and empty is
/// forgotten.
pub(crate) fn collect_order(
    realm: &Realm,
    exchange: &mut Exchange,
    id: u64,
    to: Holder,
    close: bool,
    into: &mut Collected,
) {
    if close {
        exchange.unlist(id);
    }
    let Some(order) = exchange.orders.get(&id).cloned() else {
        return;
    };
    let (cash, goods) = order.claimable(realm);
    {
        // Neither can fail: the amounts are in the escrow.
        let mut ledger = realm.ledger_mut();
        let _ = ledger.transfer_money(order.escrow(), to, Money(cash), "exchange.claim");
        let _ = ledger.transfer_item(order.escrow(), to, order.item, goods, "exchange.claim");
    }
    into.cash += cash;
    if goods > 0 {
        *into.items.entry(order.item).or_insert(0) += goods;
    }
    if !order.open {
        exchange.remove(id);
    }
}

/// Registers the verbs `order_cancel` and `market_claim`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.verb::<OrderCancelArgs>(
        spec(
            "order_cancel",
            "Cancel one of your orders at the exchange and take back what it holds. You must \
             be at the exchange.",
            false,
        ),
        move |realm, ctx, args| {
            own_order(realm, ctx.idx, ctx.resident, args.order)?;
            Ok(busy(realm, &p))
        },
        |realm, ctx, args| cancel(realm, ctx.idx, ctx.resident, args.order),
    );
    b.verb::<NoArgs>(
        spec(
            "market_claim",
            "Collect the goods that your orders bought and the cash that they earned. You \
             must be at the exchange.",
            false,
        ),
        move |realm, ctx, _args| {
            require_site(realm, ctx.idx, EXCHANGE_SITE)?;
            let exchange = realm.res::<Exchange>();
            let waiting =
                exchange.orders_of(ctx.resident).any(|order| order.claimable(realm) != (0, 0));
            if !waiting {
                return Err(ActionError::not_found("You have nothing to collect here."));
            }
            Ok(busy(realm, &p))
        },
        |realm, ctx, _args| claim(realm, ctx.idx, ctx.resident),
    );
}

/// Checks that the trader is at the exchange and owns the order.
fn own_order(realm: &Realm, idx: Idx, trader: ResidentId, id: u64) -> Result<(), ActionError> {
    require_site(realm, idx, EXCHANGE_SITE)?;
    let exchange = realm.res::<Exchange>();
    match exchange.order(id) {
        Some(order) if order.trader == trader => Ok(()),
        _ => Err(ActionError::not_found(format!("You have no order {id} at this exchange."))),
    }
}

fn cancel(realm: &Realm, idx: Idx, trader: ResidentId, id: u64) -> Result<Value, ActionError> {
    own_order(realm, idx, trader, id)?;
    let mut exchange = realm.res_mut::<Exchange>();
    let (filled, unfilled) =
        exchange.order(id).map_or((0, 0), |order| (order.filled, order.remaining()));
    let mut collected = Collected::default();
    collect_order(realm, &mut exchange, id, Holder::Resident(trader), true, &mut collected);
    Ok(json!({
        "cancelled": id,
        "filled": filled,
        "unfilled": unfilled,
        "returned": collected.to_value(realm),
    }))
}

fn claim(realm: &Realm, idx: Idx, trader: ResidentId) -> Result<Value, ActionError> {
    require_site(realm, idx, EXCHANGE_SITE)?;
    let mut exchange = realm.res_mut::<Exchange>();
    let ids: Vec<u64> = exchange.orders_of(trader).map(|order| order.id).collect();
    let mut collected = Collected::default();
    for id in ids {
        collect_order(realm, &mut exchange, id, Holder::Resident(trader), false, &mut collected);
    }
    let open = exchange.orders_of(trader).count();
    Ok(json!({ "claimed": collected.to_value(realm), "open_orders": open }))
}
