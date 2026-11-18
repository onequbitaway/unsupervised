//! The matching engine of the share market. A new order trades at once
//! against the book, best price first and oldest order first. What is left
//! of it then waits in the book.

use serde_json::json;
use unsup_kernel::{EventMeta, Holder, Money, Realm, ResidentId};

use super::state::{ShareOrder, Shares};
use crate::bank::Bank;
use crate::exchange::Side;
use crate::params::Params;
use crate::util::drama;

/// What a new order did when it was placed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Outcome {
    /// Shares that traded at once.
    pub(crate) traded: i64,
    /// Cents that the trades moved.
    pub(crate) value: i64,
    /// Shares that wait in the book.
    pub(crate) resting: i64,
}

/// Returns the seller of an order as an event shows it: the resident, or
/// nothing for new shares that the organization sells.
fn seller_of(order: &ShareOrder) -> Option<ResidentId> {
    (!order.issue).then_some(order.placed_by)
}

/// Moves the money and the shares of one trade. `buy` pays `price` per
/// share from its escrow. A buyer that locked more per share gets the
/// difference back in its account.
fn settle(
    realm: &Realm,
    shares: &mut Shares,
    bank: &mut Bank,
    buy: &ShareOrder,
    sell: &ShareOrder,
    qty: i64,
    price: i64,
) {
    let value = price * qty;
    // The transfers cannot fail: the escrow of a buy order holds the limit
    // price of every share that is not yet bought.
    if sell.issue {
        let treasury = Holder::Org(sell.org);
        let _ =
            realm.ledger_mut().transfer_money(buy.escrow(), treasury, Money(value), "shares.issue");
        if let Some(company) = shares.companies.get_mut(&sell.org.0) {
            company.on_offer -= qty;
        }
    } else {
        let _ = bank.pay_in(realm, buy.escrow(), sell.placed_by, value, "shares.trade");
        shares.add_locked(sell.org, sell.placed_by, -qty);
        shares.add_held(sell.org, sell.placed_by, -qty);
    }
    let saved = (buy.price - price) * qty;
    let _ = bank.pay_in(realm, buy.escrow(), buy.placed_by, saved, "shares.unlock");
    shares.add_held(buy.org, buy.placed_by, qty);
    if let Some(company) = shares.companies.get_mut(&buy.org.0) {
        company.book.last_price = price;
    }
}

/// Places an order: trades it against the book, then lets the rest wait.
/// The caller has locked the cash of a buy order in its escrow, or the
/// shares of a sell order.
pub(crate) fn place(
    realm: &Realm,
    p: &Params,
    shares: &mut Shares,
    bank: &mut Bank,
    mut taker: ShareOrder,
) -> Outcome {
    let mut outcome = Outcome::default();
    while taker.remaining() > 0 {
        let Some(company) = shares.companies.get(&taker.org.0) else {
            break;
        };
        let (bid, ask) = company.book.best();
        let quote = match taker.side {
            Side::Buy => ask.filter(|ask| ask.price <= taker.price),
            Side::Sell => bid.filter(|bid| bid.price >= taker.price),
        };
        let Some(quote) = quote else {
            break;
        };
        let Some(maker) = shares.orders.get(&quote.order).cloned() else {
            // A quote without an order: drop it and go on.
            if let Some(company) = shares.companies.get_mut(&taker.org.0) {
                company.book.drop_order(Side::Buy, quote.order);
                company.book.drop_order(Side::Sell, quote.order);
            }
            continue;
        };
        // The order that waited in the book sets the price.
        let price = maker.price;
        let qty = taker.remaining().min(maker.remaining());
        let (buy, sell) = match taker.side {
            Side::Buy => (&taker, &maker),
            Side::Sell => (&maker, &taker),
        };
        settle(realm, shares, bank, buy, sell, qty, price);
        realm.emit(
            EventMeta::public("finance.shares_traded", drama(realm, p.drama_routine)),
            &json!({
                "org": taker.org,
                "shares": qty,
                "price": price,
                "buyer": buy.placed_by,
                "seller": seller_of(sell),
            }),
        );
        realm.notify(
            maker.placed_by,
            "finance.share_order_filled",
            None,
            json!({
                "order": maker.id,
                "org": maker.org,
                "side": maker.side.name(),
                "shares": qty,
                "price": price,
                "unfilled": maker.remaining() - qty,
            }),
        );
        taker.filled += qty;
        outcome.traded += qty;
        outcome.value += price * qty;
        if maker.remaining() == qty {
            shares.remove(maker.id);
        } else if let Some(order) = shares.orders.get_mut(&maker.id) {
            order.filled += qty;
        }
    }
    outcome.resting = taker.remaining();
    if outcome.resting > 0 {
        shares.rest(taker);
    }
    outcome
}

/// Cancels an open order and gives back what it locks: the cash of a buy
/// order goes to the account of the buyer, the shares of a sell order are
/// free again, and new shares on offer are withdrawn. Returns the order.
pub(crate) fn cancel(
    realm: &Realm,
    shares: &mut Shares,
    bank: &mut Bank,
    id: u64,
) -> Option<ShareOrder> {
    let order = shares.remove(id)?;
    match order.side {
        Side::Buy => {
            let locked = realm.ledger().money(order.escrow()).0;
            // It cannot fail: the amount is the balance of the escrow.
            let _ = bank.pay_in(realm, order.escrow(), order.placed_by, locked, "shares.unlock");
        }
        Side::Sell if order.issue => {
            if let Some(company) = shares.companies.get_mut(&order.org.0) {
                company.on_offer -= order.remaining();
            }
        }
        Side::Sell => shares.add_locked(order.org, order.placed_by, -order.remaining()),
    }
    Some(order)
}
