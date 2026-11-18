//! The exchange: one order book per item in the zone of the `exchange`
//! site of a city.
//!
//! A trader must be at the exchange to place an order, to cancel it and to
//! collect what it bought or earned. An order locks cash or goods in its
//! escrow. The matching system trades crossing orders every tick, by price
//! and then by time.

mod book;
mod collect;
mod matching;
mod observe;
mod place;
mod state;

use unsup_kernel::{Begin, Builder, Realm, Scope, Stage};

pub use self::book::MarketBookArgs;
pub use self::collect::OrderCancelArgs;
pub(crate) use self::collect::{Collected, collect_order};
pub use self::place::OrderPlaceArgs;
pub use self::state::{Book, Exchange, Level, Order, Quote, Side};
use crate::params::Params;

/// The kind of site where the exchange verbs work.
pub(crate) const EXCHANGE_SITE: &str = "exchange";

/// The tag of items that are illegal to carry. The exchange does not trade
/// them.
pub(crate) const CONTRABAND_TAG: &str = "contraband";

/// The answer of `begin` for the verbs that take time at the exchange.
fn busy(realm: &Realm, p: &Params) -> Begin {
    Begin::Busy { ticks: u32::try_from(realm.param(p.exchange_ticks).max(1)).unwrap_or(1) }
}

/// Registers the exchange.
pub(crate) fn register(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Zone {
        b.resource::<Exchange>();
        b.system(Stage::Update, 0, "finance.exchange_match", move |realm| matching::run(realm, &p));
    }
    place::register(b, p);
    collect::register(b, p);
    book::register(b, p);
    observe::register(b, p);
}
