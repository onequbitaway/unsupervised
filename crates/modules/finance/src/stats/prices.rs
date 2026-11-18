//! Prices for the price index. They come from the events
//! `economy.goods_sold` and `finance.order_filled`.

use std::collections::BTreeMap;

use unsup_kernel::{Blueprint, Event};

use super::state::{PricePoint, Stats};
use crate::util::{BP, mul_div};

/// The tag of the items in the basket of the price index.
pub(crate) const BASKET_TAG: &str = "food";

/// Returns true when an item is in the basket of the price index.
fn in_basket(defs: &Blueprint, item: &str) -> bool {
    defs.item_id(item).and_then(|id| defs.item(id)).is_some_and(|def| def.has_tag(BASKET_TAG))
}

impl Stats {
    /// Counts one sale: `qty` units of an item at `price` cents each. A
    /// name that is not in the item catalogue is ignored.
    pub(crate) fn record_sale(&mut self, defs: &Blueprint, item: &str, qty: i64, price: i64) {
        if qty <= 0 || price <= 0 || defs.item_id(item).is_none() {
            return;
        }
        let track = self.prices.entry(item.to_string()).or_default();
        track.day_value = track.day_value.saturating_add(price.saturating_mul(qty));
        track.day_qty = track.day_qty.saturating_add(qty);
    }

    /// Counts the sale that an event reports. `price_key` names the field
    /// with the unit price.
    pub(crate) fn record_event(&mut self, defs: &Blueprint, event: &Event, price_key: &str) {
        let data = &event.data;
        if let (Some(item), Some(qty), Some(price)) =
            (data["item"].as_str(), data["qty"].as_i64(), data[price_key].as_i64())
        {
            self.record_sale(defs, item, qty, price);
        }
    }

    /// Ends a day of prices: the mean price of the day becomes the last
    /// price of each item that was sold, and its reference price when it
    /// has none. Returns the prices and the price index in basis points.
    ///
    /// The index is the mean of `last / reference` over the basket items
    /// that have a price. It is 10,000 when no basket item has a price.
    pub(crate) fn close_prices(&mut self, defs: &Blueprint) -> (BTreeMap<String, PricePoint>, i64) {
        let mut points = BTreeMap::new();
        let (mut ratios, mut basket_items) = (0i64, 0i64);
        for (item, track) in &mut self.prices {
            if track.day_qty > 0 {
                track.last = track.day_value / track.day_qty;
                if track.reference == 0 {
                    track.reference = track.last;
                }
                track.day_value = 0;
                track.day_qty = 0;
            }
            if track.reference <= 0 {
                continue;
            }
            let basket = in_basket(defs, item);
            if basket {
                ratios = ratios.saturating_add(mul_div(track.last, BP, track.reference));
                basket_items += 1;
            }
            points.insert(
                item.clone(),
                PricePoint { last: track.last, reference: track.reference, basket },
            );
        }
        let index = if basket_items == 0 { BP } else { ratios / basket_items };
        (points, index)
    }
}
