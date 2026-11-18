//! `order_place`: put a limit order in the book of the exchange.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Builder, Dest, Holder, Idx, ItemId, Money, Realm, ResidentId};

use super::state::{Exchange, Order, Side};
use super::{CONTRABAND_TAG, EXCHANGE_SITE, busy};
use crate::estate::ExchangeOpened;
use crate::params::Params;
use crate::util::{count, require_site, spec};

/// Arguments of `order_place`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct OrderPlaceArgs {
    /// Name of the item to trade.
    pub item: String,
    /// `buy` or `sell`.
    pub side: Side,
    /// Number of units.
    pub qty: i64,
    /// Limit price of one unit in cents: the most that a buy order pays,
    /// the least that a sell order takes.
    pub price: i64,
}

/// Registers the verb `order_place`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.verb::<OrderPlaceArgs>(
        spec(
            "order_place",
            "Place a buy or sell order at the exchange. It locks your cash or goods. You must \
             be at the exchange.",
            false,
        ),
        move |realm, ctx, args| {
            check(realm, &p, ctx.idx, ctx.resident, args)?;
            Ok(busy(realm, &p))
        },
        move |realm, ctx, args| place(realm, &p, ctx.idx, ctx.resident, args),
    );
}

/// Checks an order. Returns the item.
fn check(
    realm: &Realm,
    p: &Params,
    idx: Idx,
    trader: ResidentId,
    args: &OrderPlaceArgs,
) -> Result<ItemId, ActionError> {
    require_site(realm, idx, EXCHANGE_SITE)?;
    let defs = realm.defs();
    let Some(item) = defs.item_id(&args.item) else {
        return Err(ActionError::invalid_args(format!("No item is named `{}`.", args.item)));
    };
    if defs.item(item).is_none_or(|def| def.has_tag(CONTRABAND_TAG)) {
        return Err(ActionError::not_allowed(format!(
            "The exchange does not trade `{}`.",
            args.item
        )));
    }
    let (qty_max, price_max) = (realm.param(p.order_qty_max), realm.param(p.order_price_max));
    if args.qty < 1 || args.qty > qty_max {
        return Err(ActionError::invalid_args(format!(
            "The quantity of an order is from 1 to {qty_max}."
        )));
    }
    if args.price < 1 || args.price > price_max {
        return Err(ActionError::invalid_args(format!(
            "The price of an order is from 1 to {price_max} cents."
        )));
    }
    let exchange = realm.res::<Exchange>();
    let most = count(realm, p.orders_max_per_trader);
    if exchange.orders_of(trader).count() >= most {
        return Err(ActionError::conflict(format!(
            "You have {most} orders at this exchange. Use `market_claim` or `order_cancel` \
             first."
        )));
    }
    if exchange.crosses_own(trader, item, args.side, args.price) {
        return Err(ActionError::conflict("This order would trade with your own order."));
    }
    let me = Holder::Resident(trader);
    let ledger = realm.ledger();
    // The limits keep the value of an order far below the range of the
    // ledger. This guards a data file that sets them too high.
    let Some(value) = args.qty.checked_mul(args.price) else {
        return Err(ActionError::invalid_args("The value of the order is too large."));
    };
    let (needed, available) = match args.side {
        Side::Buy => (value, ledger.money(me).0),
        Side::Sell => (args.qty, ledger.qty(me, item)),
    };
    if available < needed {
        let message = match args.side {
            Side::Buy => "You do not have the cash that this order locks.",
            Side::Sell => "You do not have the goods that this order locks.",
        };
        return Err(ActionError::insufficient(message)
            .with_details(json!({ "needed": needed, "available": available })));
    }
    Ok(item)
}

fn place(
    realm: &Realm,
    p: &Params,
    idx: Idx,
    trader: ResidentId,
    args: &OrderPlaceArgs,
) -> Result<Value, ActionError> {
    // Something can have changed while the trader stood at the counter.
    let item = check(realm, p, idx, trader, args)?;
    let id = realm.alloc_id();
    let (me, escrow) = (Holder::Resident(trader), Holder::Escrow(id));
    match args.side {
        Side::Buy => realm.ledger_mut().transfer_money(
            me,
            escrow,
            Money(args.qty * args.price),
            "exchange.lock",
        )?,
        Side::Sell => {
            realm.ledger_mut().transfer_item(me, escrow, item, args.qty, "exchange.lock")?
        }
    }
    let mut exchange = realm.res_mut::<Exchange>();
    exchange.insert(Order {
        id,
        trader,
        item,
        side: args.side,
        price: args.price,
        qty: args.qty,
        ..Order::default()
    });
    if !exchange.announced {
        // The global realm must know which zones have orders, so that it
        // can close the orders of a resident that leaves the world.
        exchange.announced = true;
        realm.send(Dest::Global, &ExchangeOpened {});
    }
    Ok(json!({
        "order": id,
        "item": args.item,
        "side": args.side.name(),
        "qty": args.qty,
        "price": args.price,
    }))
}
