//! The price lists of the shops of a zone.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{ItemId, Resource, SiteId};

/// The prices of one good at one shop, in cents for one unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Price {
    /// What a customer pays. `None`: the shop does not sell the good.
    pub(crate) sell: Option<i64>,
    /// What the shop pays to a resident. `None`: the shop does not buy the
    /// good.
    pub(crate) buy: Option<i64>,
}

/// The price lists of one zone, by site and good.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct PriceBook {
    /// The price list of every shop that has one.
    pub(crate) sites: BTreeMap<SiteId, BTreeMap<ItemId, Price>>,
}

impl Resource for PriceBook {
    const NAME: &'static str = "economy.prices";
}

impl PriceBook {
    /// Returns the prices of a good at a shop.
    pub(crate) fn get(&self, site: SiteId, item: ItemId) -> Price {
        self.sites.get(&site).and_then(|list| list.get(&item)).copied().unwrap_or_default()
    }

    /// Sets the prices of a good at a shop. An entry without a price is
    /// removed.
    pub(crate) fn set(&mut self, site: SiteId, item: ItemId, price: Price) {
        if price == Price::default() {
            if let Some(list) = self.sites.get_mut(&site) {
                list.remove(&item);
                if list.is_empty() {
                    self.sites.remove(&site);
                }
            }
        } else {
            self.sites.entry(site).or_default().insert(item, price);
        }
    }

    /// Removes the price list of a shop.
    pub(crate) fn clear(&mut self, site: SiteId) {
        self.sites.remove(&site);
    }
}
