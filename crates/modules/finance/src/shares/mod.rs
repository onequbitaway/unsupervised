//! Shares: an organization issues them, residents trade them in a global
//! order book per organization, and dividends go to the holders.
//!
//! A share is not a good. The resource [`Shares`] is the register of who
//! owns how many. Money for shares moves through the ledger, between bank
//! accounts, order escrows and the treasury of the organization.

mod desk;
mod engine;
mod state;
mod trade;

use serde_json::{Value, json};
use unsup_kernel::{Builder, ObserveOpts, Realm, ResidentId, Scope};

pub use self::desk::{IssueSharesArgs, PayDividendArgs};
pub(crate) use self::engine::cancel as cancel_order;
pub use self::state::{Company, Holding, ShareOrder, Shares};
pub use self::trade::{CancelSharesArgs, ShareOrderArgs, SharesBookArgs};
use crate::params::Params;
use crate::util::count;

/// Registers the share market.
pub(crate) fn register(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.resource::<Shares>();
    }
    desk::register(b, p);
    trade::register(b, p);
}

/// Returns the shares part of the `bank` section of a resident: its
/// holdings and its open share orders. `None` when it has neither.
pub(crate) fn observe(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    opts: &ObserveOpts,
) -> Option<Value> {
    let shares = realm.res::<Shares>();
    let shown = if opts.is_full() { usize::MAX } else { count(realm, p.observe_list_max) };
    let total = shares.portfolio(resident).count();
    let open = shares.orders_of(resident).count();
    if total == 0 && open == 0 {
        return None;
    }
    let holdings: Vec<Value> = shares
        .portfolio(resident)
        .take(shown)
        .map(|(org, holding)| {
            json!({ "org": org, "shares": holding.held, "locked": holding.locked })
        })
        .collect();
    let mut out = json!({ "holdings": holdings, "open_orders": open });
    if total > holdings.len() {
        out["more_holdings"] = json!(total - holdings.len());
    }
    if opts.is_full() {
        let orders: Vec<Value> = shares
            .orders_of(resident)
            .map(|order| {
                json!({
                    "order": order.id,
                    "org": order.org,
                    "side": order.side.name(),
                    "issue": order.issue,
                    "price": order.price,
                    "shares": order.qty,
                    "filled": order.filled,
                })
            })
            .collect();
        out["orders"] = json!(orders);
    }
    Some(out)
}
