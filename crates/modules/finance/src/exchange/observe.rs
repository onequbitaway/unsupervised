//! The observation section `market`: the orders of the resident at the
//! exchange where it stands, and what it can collect there.

use std::collections::BTreeMap;

use serde_json::{Value, json};
use unsup_kernel::{Builder, Idx, ObserveOpts, Realm};

use super::EXCHANGE_SITE;
use super::state::Exchange;
use crate::params::Params;
use crate::util::{count, site_of_kind};

/// Registers the zone observer of the section `market`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.observe("market", 31, move |realm, idx, opts| section(realm, &p, idx, opts));
}

fn section(realm: &Realm, p: &Params, idx: Idx, opts: &ObserveOpts) -> Option<Value> {
    site_of_kind(realm, idx, EXCHANGE_SITE)?;
    let trader = realm.id_of(idx);
    let exchange = realm.res::<Exchange>();
    let shown = if opts.is_full() { usize::MAX } else { count(realm, p.observe_list_max) };
    let mut orders = Vec::new();
    let mut total = 0usize;
    let mut cash = 0i64;
    let mut items: BTreeMap<&str, i64> = BTreeMap::new();
    for order in exchange.orders_of(trader) {
        total += 1;
        let name = realm.defs().item_name(order.item);
        let (claim_cash, claim_goods) = order.claimable(realm);
        cash += claim_cash;
        if claim_goods > 0 {
            *items.entry(name).or_insert(0) += claim_goods;
        }
        if orders.len() < shown {
            orders.push(json!({
                "order": order.id,
                "item": name,
                "side": order.side.name(),
                "price": order.price,
                "qty": order.qty,
                "filled": order.filled,
                "open": order.open,
            }));
        }
    }
    if total == 0 {
        return None;
    }
    let mut out = json!({ "orders": orders, "claimable": { "cash": cash, "items": items } });
    if total > orders.len() {
        out["more_orders"] = json!(total - orders.len());
    }
    Some(out)
}
