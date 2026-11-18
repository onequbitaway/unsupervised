//! The matching system of the exchange. It runs every tick in every zone
//! and trades the orders that cross, by price and then by time.

use std::collections::BTreeMap;

use serde_json::json;
use unsup_kernel::{Dest, EventMeta, Holder, ItemId, Money, OrgId, Pos, Realm};

use super::EXCHANGE_SITE;
use super::state::{Exchange, Side};
use crate::params::Params;
use crate::util::{drama, part};

/// What one order traded in one matching run.
#[derive(Default)]
struct Fill {
    qty: i64,
    value: i64,
}

/// What a matching run needs to know about its zone.
struct Run {
    city: u16,
    pos: Option<Pos>,
    fee_bp: i64,
    fee_escrow: Holder,
    drama: u8,
    fills: BTreeMap<u64, Fill>,
}

/// Trades every pair of orders that cross. Does nothing in a tick without
/// a new order.
pub(crate) fn run(realm: &Realm, p: &Params) {
    if realm.res::<Exchange>().dirty.is_empty() {
        return;
    }
    let Some(zone) = realm.zone_info() else {
        return;
    };
    let mut exchange = realm.res_mut::<Exchange>();
    if exchange.fee_escrow == 0 {
        exchange.fee_escrow = realm.alloc_id();
    }
    let mut run = Run {
        city: zone.city.0,
        pos: realm.sites().of_kind(EXCHANGE_SITE).next().map(|site| site.rect.centre()),
        fee_bp: realm.param(p.exchange_fee_bp),
        fee_escrow: Holder::Escrow(exchange.fee_escrow),
        drama: drama(realm, p.drama_routine),
        fills: BTreeMap::new(),
    };
    let dirty = std::mem::take(&mut exchange.dirty);
    for item in dirty {
        match_book(realm, &mut exchange, ItemId(item), &mut run);
    }
    // The fees of this run go to the city treasury in one wire.
    let fees = realm.ledger().money(run.fee_escrow);
    if fees.is_positive() {
        let city = Holder::Org(OrgId::city(zone.city));
        // It cannot fail: the amount is the balance of the escrow.
        let _ = realm.wire_money(run.fee_escrow, Dest::Global, city, fees, "exchange.fee");
    }
    for (id, fill) in &run.fills {
        let Some(order) = exchange.orders.get(id) else {
            continue;
        };
        realm.notify(
            order.trader,
            "finance.order_filled",
            None,
            json!({
                "order": id,
                "item": realm.defs().item_name(order.item),
                "side": order.side.name(),
                "qty": fill.qty,
                "value": fill.value,
                "unfilled": order.remaining(),
            }),
        );
    }
}

/// Trades the best bid against the best ask of one book until they no
/// longer cross.
fn match_book(realm: &Realm, exchange: &mut Exchange, item: ItemId, run: &mut Run) {
    loop {
        let Some(book) = exchange.books.get(&item.0) else {
            return;
        };
        let (Some(bid), Some(ask)) = book.best() else {
            return;
        };
        if bid.price < ask.price {
            return;
        }
        let (Some(buy), Some(sell)) =
            (exchange.orders.get(&bid.order).cloned(), exchange.orders.get(&ask.order).cloned())
        else {
            // A quote without an order: drop it and go on.
            exchange.unlist_quote(item, Side::Buy, bid.order);
            exchange.unlist_quote(item, Side::Sell, ask.order);
            continue;
        };
        // The order that was in the book first sets the price.
        let price = if bid.seq < ask.seq { bid.price } else { ask.price };
        let qty = buy.remaining().min(sell.remaining());
        let value = price * qty;
        let funded = {
            let ledger = realm.ledger();
            qty > 0
                && ledger.money(buy.escrow()).0 >= value
                && ledger.qty(sell.escrow(), item) >= qty
        };
        if !funded {
            // The escrows always cover the open part of an order. If one
            // does not, take both orders out of the book so that the loop
            // ends; their owners can still collect what the escrows hold.
            exchange.unlist(buy.id);
            exchange.unlist(sell.id);
            continue;
        }
        let fee = part(value, run.fee_bp);
        {
            // None of these can fail after the check above.
            let mut ledger = realm.ledger_mut();
            let _ = ledger.transfer_money(
                buy.escrow(),
                sell.escrow(),
                Money(value - fee),
                "exchange.trade",
            );
            let _ = ledger.transfer_money(buy.escrow(), run.fee_escrow, Money(fee), "exchange.fee");
            let _ = ledger.transfer_item(sell.escrow(), buy.escrow(), item, qty, "exchange.trade");
        }
        for id in [buy.id, sell.id] {
            let done = exchange.orders.get_mut(&id).is_some_and(|order| {
                order.filled += qty;
                order.remaining() == 0
            });
            if done {
                exchange.unlist(id);
            }
            let fill = run.fills.entry(id).or_default();
            fill.qty += qty;
            fill.value += value;
        }
        if let Some(book) = exchange.books.get_mut(&item.0) {
            book.last_price = price;
        }
        let mut meta = EventMeta::public("finance.order_filled", run.drama);
        meta.pos = run.pos;
        realm.emit(
            meta,
            &json!({
                "city": run.city,
                "item": realm.defs().item_name(item),
                "qty": qty,
                "price": price,
                "buyer": buy.trader,
                "seller": sell.trader,
            }),
        );
    }
}
