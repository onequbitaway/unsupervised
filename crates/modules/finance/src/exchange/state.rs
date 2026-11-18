//! The state of the exchange of one zone: orders and one book per item.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{Holder, ItemId, Realm, ResidentId, Resource};

/// The side of an order.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The trader wants to buy.
    #[default]
    Buy,
    /// The trader wants to sell.
    Sell,
}

impl Side {
    /// Returns the name of the side as agents write it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Side::Buy => "buy",
            Side::Sell => "sell",
        }
    }
}

/// One order at the exchange.
///
/// The escrow of the order, `Holder::Escrow(id)` in the ledger of the zone,
/// holds what the order locks and what it has earned: a buy order holds
/// the cash for the units that are not yet bought and the goods that it
/// bought; a sell order holds the goods that are not yet sold and the cash
/// that it earned.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Order {
    /// Id of the order and of its escrow.
    pub id: u64,
    /// The resident that placed the order.
    pub trader: ResidentId,
    /// The item.
    pub item: ItemId,
    /// Buy or sell.
    pub side: Side,
    /// Limit price of one unit in cents.
    pub price: i64,
    /// Units that the trader wants to trade.
    pub qty: i64,
    /// Units that are traded.
    pub filled: i64,
    /// Time priority: a lower number came first.
    pub seq: u64,
    /// True while the order is in the book.
    pub open: bool,
}

impl Order {
    /// Returns the units that are not yet traded.
    #[must_use]
    pub fn remaining(&self) -> i64 {
        self.qty - self.filled
    }

    /// Returns the escrow of the order.
    #[must_use]
    pub fn escrow(&self) -> Holder {
        Holder::Escrow(self.id)
    }

    /// Returns the cash that the order still locks.
    #[must_use]
    pub fn locked_cash(&self) -> i64 {
        if self.open && self.side == Side::Buy { self.remaining() * self.price } else { 0 }
    }

    /// Returns the goods that the order still locks.
    #[must_use]
    pub fn locked_goods(&self) -> i64 {
        if self.open && self.side == Side::Sell { self.remaining() } else { 0 }
    }

    /// Returns what the trader can collect now: cash and units of the item.
    #[must_use]
    pub fn claimable(&self, realm: &Realm) -> (i64, i64) {
        let ledger = realm.ledger();
        let cash = ledger.money(self.escrow()).0 - self.locked_cash();
        let goods = ledger.qty(self.escrow(), self.item) - self.locked_goods();
        (cash.max(0), goods.max(0))
    }
}

/// The place of an order in a book.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    /// Limit price in cents.
    pub price: i64,
    /// Time priority.
    pub seq: u64,
    /// Id of the order.
    pub order: u64,
}

/// One price level of a book, as `market_book` shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Level {
    /// Price in cents.
    pub price: i64,
    /// Units on offer at this price.
    pub qty: i64,
    /// Number of orders at this price.
    pub orders: u32,
}

/// The order book of one item. Each side is sorted so that the best quote
/// is the last one: the highest bid, the lowest ask, and the oldest order
/// among equal prices.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Book {
    pub(crate) bids: Vec<Quote>,
    pub(crate) asks: Vec<Quote>,
    /// Price of the last trade, or 0 before the first trade.
    pub(crate) last_price: i64,
}

impl Book {
    /// Adds a quote at its place by price and time.
    pub(crate) fn add(&mut self, side: Side, quote: Quote) {
        // A new quote has the highest `seq`, so among equal prices it goes
        // in front of the others, away from the end.
        match side {
            Side::Buy => {
                let at = self.bids.partition_point(|q| q.price < quote.price);
                self.bids.insert(at, quote);
            }
            Side::Sell => {
                let at = self.asks.partition_point(|q| q.price > quote.price);
                self.asks.insert(at, quote);
            }
        }
    }

    /// Removes the quote of an order.
    pub(crate) fn drop_order(&mut self, side: Side, order: u64) {
        let quotes = match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        };
        quotes.retain(|q| q.order != order);
    }

    /// Returns the bids, sorted so that the best bid is the last one.
    #[must_use]
    pub fn bids(&self) -> &[Quote] {
        &self.bids
    }

    /// Returns the asks, sorted so that the best ask is the last one.
    #[must_use]
    pub fn asks(&self) -> &[Quote] {
        &self.asks
    }

    /// Returns the best bid and the best ask.
    pub(crate) fn best(&self) -> (Option<Quote>, Option<Quote>) {
        (self.bids.last().copied(), self.asks.last().copied())
    }

    /// Returns the price of the last trade, or 0 before the first trade.
    #[must_use]
    pub fn last_price(&self) -> i64 {
        self.last_price
    }

    /// Returns the best price levels of one side, best first. `remaining`
    /// gives the open quantity of an order.
    pub(crate) fn levels(
        &self,
        side: Side,
        depth: usize,
        remaining: impl Fn(u64) -> i64,
    ) -> Vec<Level> {
        let quotes = match side {
            Side::Buy => &self.bids,
            Side::Sell => &self.asks,
        };
        let mut levels: Vec<Level> = Vec::new();
        for quote in quotes.iter().rev() {
            let qty = remaining(quote.order);
            if let Some(level) = levels.last_mut().filter(|level| level.price == quote.price) {
                level.qty += qty;
                level.orders += 1;
            } else if levels.len() >= depth {
                break;
            } else {
                levels.push(Level { price: quote.price, qty, orders: 1 });
            }
        }
        levels
    }
}

/// The exchange of one zone. A zone without an `exchange` site has an
/// empty one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Exchange {
    /// Every order that is open or holds something to collect, by id.
    pub(crate) orders: BTreeMap<u64, Order>,
    /// The books, by item id.
    pub(crate) books: BTreeMap<u16, Book>,
    /// The order ids of every trader, oldest first.
    pub(crate) traders: BTreeMap<u64, Vec<u64>>,
    /// The time priority of the next order.
    pub(crate) next_seq: u64,
    /// Items whose book changed since the last matching run.
    pub(crate) dirty: BTreeSet<u16>,
    /// The escrow that collects the fees of one matching run. 0 until the
    /// first trade.
    pub(crate) fee_escrow: u64,
    /// True after the zone told the global realm that it has orders.
    pub(crate) announced: bool,
}

impl Resource for Exchange {
    const NAME: &'static str = "finance.exchange";
}

impl Exchange {
    /// Returns one order.
    #[must_use]
    pub fn order(&self, id: u64) -> Option<&Order> {
        self.orders.get(&id)
    }

    /// Returns every order, ordered by id.
    pub fn orders(&self) -> impl Iterator<Item = &Order> + '_ {
        self.orders.values()
    }

    /// Returns every book with its item, ordered by item.
    pub fn books(&self) -> impl Iterator<Item = (ItemId, &Book)> + '_ {
        self.books.iter().map(|(&item, book)| (ItemId(item), book))
    }

    /// Returns the orders of a trader, oldest first.
    pub fn orders_of(&self, trader: ResidentId) -> impl Iterator<Item = &Order> + '_ {
        self.traders.get(&trader.0).into_iter().flatten().filter_map(|id| self.orders.get(id))
    }

    /// Returns the book of an item, if the item was ever traded here.
    #[must_use]
    pub fn book(&self, item: ItemId) -> Option<&Book> {
        self.books.get(&item.0)
    }

    /// Returns true when a new order would match an open order of the same
    /// trader.
    pub(crate) fn crosses_own(
        &self,
        trader: ResidentId,
        item: ItemId,
        side: Side,
        price: i64,
    ) -> bool {
        self.orders_of(trader).any(|o| {
            o.open
                && o.item == item
                && o.side != side
                && match side {
                    Side::Buy => o.price <= price,
                    Side::Sell => o.price >= price,
                }
        })
    }

    /// Adds a new open order to the book. Returns its time priority.
    pub(crate) fn insert(&mut self, mut order: Order) -> u64 {
        self.next_seq += 1;
        order.seq = self.next_seq;
        order.open = true;
        let quote = Quote { price: order.price, seq: order.seq, order: order.id };
        self.books.entry(order.item.0).or_default().add(order.side, quote);
        self.dirty.insert(order.item.0);
        self.traders.entry(order.trader.0).or_default().push(order.id);
        self.orders.insert(order.id, order);
        self.next_seq
    }

    /// Takes an order out of the book. The order stays until it is
    /// collected.
    pub(crate) fn unlist(&mut self, id: u64) {
        let Some(order) = self.orders.get_mut(&id) else {
            return;
        };
        if order.open {
            order.open = false;
            if let Some(book) = self.books.get_mut(&order.item.0) {
                book.drop_order(order.side, id);
            }
        }
    }

    /// Removes a quote whose order is gone.
    pub(crate) fn unlist_quote(&mut self, item: ItemId, side: Side, order: u64) {
        if !self.orders.contains_key(&order)
            && let Some(book) = self.books.get_mut(&item.0)
        {
            book.drop_order(side, order);
        }
    }

    /// Forgets an order that is closed and collected.
    pub(crate) fn remove(&mut self, id: u64) -> Option<Order> {
        let order = self.orders.remove(&id)?;
        if let Some(list) = self.traders.get_mut(&order.trader.0) {
            list.retain(|&other| other != id);
            if list.is_empty() {
                self.traders.remove(&order.trader.0);
            }
        }
        Some(order)
    }

    /// Returns the best price levels of one side of a book, best first.
    #[must_use]
    pub fn levels(&self, item: ItemId, side: Side, depth: usize) -> Vec<Level> {
        self.books.get(&item.0).map_or_else(Vec::new, |book| {
            book.levels(side, depth, |id| self.orders.get(&id).map_or(0, Order::remaining))
        })
    }
}
