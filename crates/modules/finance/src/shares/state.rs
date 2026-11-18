//! The state of the share market: holdings and one order book per
//! organization. It lives in the global realm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Holder, OrgId, ResidentId, Resource};

use crate::exchange::{Book, Level, Quote, Side};

/// One order for the shares of an organization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShareOrder {
    /// Id of the order. A buy order keeps its cash in `Holder::Escrow(id)`.
    pub id: u64,
    /// The organization whose shares the order trades.
    pub org: OrgId,
    /// The resident that placed the order.
    pub placed_by: ResidentId,
    /// True for an order that sells new shares of the organization. The
    /// money goes to the treasury of the organization.
    pub issue: bool,
    /// Buy or sell.
    pub side: Side,
    /// Limit price of one share in cents.
    pub price: i64,
    /// Shares that the order wants to trade.
    pub qty: i64,
    /// Shares that are traded.
    pub filled: i64,
    /// Time priority: a lower number came first.
    pub seq: u64,
}

impl ShareOrder {
    /// Returns the shares that are not yet traded.
    #[must_use]
    pub fn remaining(&self) -> i64 {
        self.qty - self.filled
    }

    /// Returns the escrow of the order.
    #[must_use]
    pub fn escrow(&self) -> Holder {
        Holder::Escrow(self.id)
    }
}

/// The shares of one resident in one organization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Holding {
    /// Shares that the resident owns.
    pub held: i64,
    /// The part of `held` that open sell orders lock.
    pub locked: i64,
}

/// The shares of one organization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Company {
    /// Holdings by resident.
    pub(crate) holders: BTreeMap<u64, Holding>,
    /// Shares that residents own: the sum of all holdings.
    pub(crate) outstanding: i64,
    /// New shares that the organization offers and nobody has bought yet.
    pub(crate) on_offer: i64,
    /// The order book.
    pub(crate) book: Book,
    /// Dividends that the organization has paid, in cents.
    pub(crate) dividends_paid: i64,
}

impl Company {
    /// Returns the shares that residents own.
    #[must_use]
    pub fn outstanding(&self) -> i64 {
        self.outstanding
    }

    /// Returns the new shares on offer that nobody has bought yet.
    #[must_use]
    pub fn on_offer(&self) -> i64 {
        self.on_offer
    }

    /// Returns the holding of a resident.
    #[must_use]
    pub fn holding(&self, resident: ResidentId) -> Holding {
        self.holders.get(&resident.0).copied().unwrap_or_default()
    }

    /// Returns every holding, ordered by resident.
    pub fn holders(&self) -> impl Iterator<Item = (ResidentId, Holding)> + '_ {
        self.holders.iter().map(|(&id, &holding)| (ResidentId(id), holding))
    }

    /// Returns the price of the last trade, or 0 before the first trade.
    #[must_use]
    pub fn last_price(&self) -> i64 {
        self.book.last_price()
    }

    /// Returns the order book.
    #[must_use]
    pub fn book(&self) -> &Book {
        &self.book
    }

    /// Returns the dividends that the organization has paid, in cents.
    #[must_use]
    pub fn dividends_paid(&self) -> i64 {
        self.dividends_paid
    }
}

/// The share market: every company and every open order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shares {
    /// Companies by organization id.
    pub(crate) companies: BTreeMap<u64, Company>,
    /// Open orders by id.
    pub(crate) orders: BTreeMap<u64, ShareOrder>,
    /// Ids of the open orders that each resident placed, oldest first.
    pub(crate) placed: BTreeMap<u64, Vec<u64>>,
    /// The organizations in which each resident holds shares.
    pub(crate) portfolios: BTreeMap<u64, Vec<u64>>,
    /// The time priority of the next order.
    pub(crate) next_seq: u64,
}

impl Resource for Shares {
    const NAME: &'static str = "finance.shares";
}

impl Shares {
    /// Returns the shares of an organization, if it ever issued any.
    #[must_use]
    pub fn company(&self, org: OrgId) -> Option<&Company> {
        self.companies.get(&org.0)
    }

    /// Returns every company with its organization, ordered by
    /// organization.
    pub fn companies(&self) -> impl Iterator<Item = (OrgId, &Company)> + '_ {
        self.companies.iter().map(|(&org, company)| (OrgId(org), company))
    }

    /// Returns every open order, ordered by id.
    pub fn orders(&self) -> impl Iterator<Item = &ShareOrder> + '_ {
        self.orders.values()
    }

    /// Returns one open order.
    #[must_use]
    pub fn order(&self, id: u64) -> Option<&ShareOrder> {
        self.orders.get(&id)
    }

    /// Returns the open orders that a resident placed, oldest first.
    pub fn orders_of(&self, resident: ResidentId) -> impl Iterator<Item = &ShareOrder> + '_ {
        self.placed.get(&resident.0).into_iter().flatten().filter_map(|id| self.orders.get(id))
    }

    /// Returns the organizations in which a resident holds shares, with the
    /// holding, ordered by organization.
    pub fn portfolio(&self, resident: ResidentId) -> impl Iterator<Item = (OrgId, Holding)> + '_ {
        self.portfolios.get(&resident.0).into_iter().flatten().filter_map(move |org| {
            let holding = self.companies.get(org)?.holding(resident);
            Some((OrgId(*org), holding))
        })
    }

    /// Returns the best price levels of one side of the book of an
    /// organization, best first.
    #[must_use]
    pub fn levels(&self, org: OrgId, side: Side, depth: usize) -> Vec<Level> {
        self.companies.get(&org.0).map_or_else(Vec::new, |company| {
            company
                .book
                .levels(side, depth, |id| self.orders.get(&id).map_or(0, ShareOrder::remaining))
        })
    }

    /// Changes the number of shares that a resident owns in an
    /// organization, and keeps the totals and the portfolio index right.
    pub(crate) fn add_held(&mut self, org: OrgId, resident: ResidentId, delta: i64) {
        let company = self.companies.entry(org.0).or_default();
        let holding = company.holders.entry(resident.0).or_default();
        holding.held += delta;
        company.outstanding += delta;
        let empty = holding.held == 0 && holding.locked == 0;
        if empty {
            company.holders.remove(&resident.0);
        }
        let list = self.portfolios.entry(resident.0).or_default();
        match (list.binary_search(&org.0), empty) {
            (Err(at), false) => list.insert(at, org.0),
            (Ok(at), true) => {
                list.remove(at);
            }
            _ => {}
        }
        if list.is_empty() {
            self.portfolios.remove(&resident.0);
        }
    }

    /// Changes the number of shares that open sell orders of a resident
    /// lock.
    pub(crate) fn add_locked(&mut self, org: OrgId, resident: ResidentId, delta: i64) {
        if let Some(holding) =
            self.companies.get_mut(&org.0).and_then(|c| c.holders.get_mut(&resident.0))
        {
            holding.locked += delta;
        }
    }

    /// Puts an order in the book. Returns its time priority.
    pub(crate) fn rest(&mut self, mut order: ShareOrder) {
        self.next_seq += 1;
        order.seq = self.next_seq;
        let quote = Quote { price: order.price, seq: order.seq, order: order.id };
        self.companies.entry(order.org.0).or_default().book.add(order.side, quote);
        self.placed.entry(order.placed_by.0).or_default().push(order.id);
        self.orders.insert(order.id, order);
    }

    /// Takes an order out of the book and forgets it.
    pub(crate) fn remove(&mut self, id: u64) -> Option<ShareOrder> {
        let order = self.orders.remove(&id)?;
        if let Some(company) = self.companies.get_mut(&order.org.0) {
            company.book.drop_order(order.side, id);
        }
        if let Some(list) = self.placed.get_mut(&order.placed_by.0) {
            list.retain(|&other| other != id);
            if list.is_empty() {
                self.placed.remove(&order.placed_by.0);
            }
        }
        Some(order)
    }

    /// Returns true when a new order of a resident would match one of its
    /// own open orders. An order that sells new shares for an organization
    /// is not an order of the resident.
    pub(crate) fn crosses_own(
        &self,
        resident: ResidentId,
        org: OrgId,
        side: Side,
        price: i64,
    ) -> bool {
        self.orders_of(resident).any(|o| {
            !o.issue
                && o.org == org
                && o.side != side
                && match side {
                    Side::Buy => o.price <= price,
                    Side::Sell => o.price >= price,
                }
        })
    }
}
