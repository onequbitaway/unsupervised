//! `market_book`: the best price levels of one item at the exchange.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Builder, Realm};

use super::EXCHANGE_SITE;
use super::state::{Exchange, Side};
use crate::params::Params;
use crate::util::{count, spec};

/// Arguments of `market_book`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct MarketBookArgs {
    /// Name of the item.
    pub item: String,
}

/// Registers the verb `market_book`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.instant_verb::<MarketBookArgs>(
        spec(
            "market_book",
            "Read the best buy and sell prices of an item at the exchange. You must be in the \
             district of the exchange.",
            true,
        ),
        move |realm, _ctx, args| book(realm, &p, args),
    );
}

fn book(realm: &Realm, p: &Params, args: &MarketBookArgs) -> Result<Value, ActionError> {
    if realm.sites().of_kind(EXCHANGE_SITE).next().is_none() {
        return Err(ActionError::not_here(
            "This district has no exchange. Go to the district of the exchange.",
        ));
    }
    let Some(item) = realm.defs().item_id(&args.item) else {
        return Err(ActionError::invalid_args(format!("No item is named `{}`.", args.item)));
    };
    let exchange = realm.res::<Exchange>();
    let depth = count(realm, p.book_depth);
    Ok(json!({
        "item": args.item,
        "bids": exchange.levels(item, Side::Buy, depth),
        "asks": exchange.levels(item, Side::Sell, depth),
        "last_price": exchange.book(item).map_or(0, |book| book.last_price()),
    }))
}
