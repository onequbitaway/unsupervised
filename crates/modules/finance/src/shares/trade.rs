//! `buy_shares`, `sell_shares`, `cancel_shares` and `shares_book`: the
//! verbs of the share market. They work from anywhere, with money from the
//! bank account.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Builder, OrgId, Realm, ResidentId};

use super::engine;
use super::state::{ShareOrder, Shares};
use crate::bank::Bank;
use crate::exchange::Side;
use crate::params::Params;
use crate::util::{count, no_account, remote, spec};

/// Arguments of `buy_shares` and `sell_shares`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ShareOrderArgs {
    /// The organization whose shares you trade.
    pub org: OrgId,
    /// Number of shares.
    pub shares: i64,
    /// Limit price of one share in cents: the most that you pay, or the
    /// least that you take.
    pub price: i64,
}

/// Arguments of `cancel_shares`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CancelSharesArgs {
    /// Id of the share order, from the result of the verb that placed it.
    pub order: u64,
}

/// Arguments of `shares_book`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SharesBookArgs {
    /// The organization.
    pub org: OrgId,
}

/// Checks the numbers of a share order.
pub(crate) fn check_numbers(
    realm: &Realm,
    p: &Params,
    shares: i64,
    price: i64,
) -> Result<(), ActionError> {
    let (most, dearest) = (realm.param(p.shares_max_per_org), realm.param(p.share_price_max));
    if shares < 1 || shares > most {
        return Err(ActionError::invalid_args(format!(
            "The number of shares is from 1 to {most}."
        )));
    }
    if price < 1 || price > dearest {
        return Err(ActionError::invalid_args(format!(
            "The price of a share is from 1 to {dearest} cents."
        )));
    }
    // The limits keep the value of an order far below the range of the
    // ledger. This guards a data file that sets them too high.
    if shares.checked_mul(price).is_none() {
        return Err(ActionError::invalid_args("The value of the order is too large."));
    }
    Ok(())
}

/// Checks that a resident can place one more share order.
pub(crate) fn check_order_count(
    realm: &Realm,
    p: &Params,
    shares: &Shares,
    resident: ResidentId,
) -> Result<(), ActionError> {
    let most = count(realm, p.share_orders_max);
    if shares.orders_of(resident).count() >= most {
        return Err(ActionError::conflict(format!(
            "You have {most} open share orders. Use `cancel_shares` first."
        )));
    }
    Ok(())
}

/// Registers the verbs of the share market.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.global_verb::<ShareOrderArgs>(
        spec(
            "buy_shares",
            "Buy shares of an organization with money from your bank account. The part that \
             finds no seller waits as an order.",
            false,
        ),
        move |realm, ctx, args| trade(realm, &p, ctx.resident, args, Side::Buy),
    );
    b.global_verb::<ShareOrderArgs>(
        spec(
            "sell_shares",
            "Sell shares that you own. The money goes to your bank account. The part that \
             finds no buyer waits as an order.",
            false,
        ),
        move |realm, ctx, args| trade(realm, &p, ctx.resident, args, Side::Sell),
    );
    b.global_verb::<CancelSharesArgs>(
        remote("cancel_shares", "Cancel one of your open share orders.", false),
        |realm, ctx, args| cancel(realm, ctx.resident, args),
    );
    b.global_verb::<SharesBookArgs>(
        remote(
            "shares_book",
            "Read the best buy and sell prices of the shares of an organization.",
            true,
        ),
        move |realm, _ctx, args| book(realm, &p, args),
    );
}

fn trade(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    args: &ShareOrderArgs,
    side: Side,
) -> Result<Value, ActionError> {
    check_numbers(realm, p, args.shares, args.price)?;
    let mut bank = realm.res_mut::<Bank>();
    if !bank.is_open(resident) {
        return Err(no_account(resident, true));
    }
    let mut shares = realm.res_mut::<Shares>();
    let Some(company) = shares.company(args.org) else {
        return Err(ActionError::not_found(format!("Organization {} has no shares.", args.org)));
    };
    let free = company.holding(resident);
    let free = free.held - free.locked;
    check_order_count(realm, p, &shares, resident)?;
    if shares.crosses_own(resident, args.org, side, args.price) {
        return Err(ActionError::conflict("This order would trade with your own order."));
    }
    // Check what the order locks before anything changes.
    let (needed, available) = match side {
        Side::Buy => (args.shares * args.price, Bank::balance(realm, resident)),
        Side::Sell => (args.shares, free),
    };
    if available < needed {
        let message = match side {
            Side::Buy => "Your balance is too low for this order.",
            Side::Sell => "You do not have these shares.",
        };
        return Err(ActionError::insufficient(message)
            .with_details(json!({ "needed": needed, "available": available })));
    }
    let id = realm.alloc_id();
    let order = ShareOrder {
        id,
        org: args.org,
        placed_by: resident,
        side,
        price: args.price,
        qty: args.shares,
        ..ShareOrder::default()
    };
    match side {
        Side::Buy => bank.pay_out(realm, resident, order.escrow(), needed, "shares.lock")?,
        Side::Sell => shares.add_locked(args.org, resident, args.shares),
    }
    let outcome = engine::place(realm, p, &mut shares, &mut bank, order);
    Ok(json!({
        "order": (outcome.resting > 0).then_some(id),
        "side": side.name(),
        "traded": outcome.traded,
        "value": outcome.value,
        "waiting": outcome.resting,
        "balance": Bank::balance(realm, resident),
    }))
}

fn cancel(
    realm: &Realm,
    resident: ResidentId,
    args: &CancelSharesArgs,
) -> Result<Value, ActionError> {
    let mut shares = realm.res_mut::<Shares>();
    if shares.order(args.order).is_none_or(|order| order.placed_by != resident) {
        return Err(ActionError::not_found(format!("You have no share order {}.", args.order)));
    }
    let mut bank = realm.res_mut::<Bank>();
    let unfilled = engine::cancel(realm, &mut shares, &mut bank, args.order)
        .map_or(0, |order| order.remaining());
    Ok(json!({ "cancelled": args.order, "unfilled": unfilled }))
}

fn book(realm: &Realm, p: &Params, args: &SharesBookArgs) -> Result<Value, ActionError> {
    let shares = realm.res::<Shares>();
    let Some(company) = shares.company(args.org) else {
        return Err(ActionError::not_found(format!("Organization {} has no shares.", args.org)));
    };
    let depth = count(realm, p.book_depth);
    Ok(json!({
        "org": args.org,
        "bids": shares.levels(args.org, Side::Buy, depth),
        "asks": shares.levels(args.org, Side::Sell, depth),
        "last_price": company.last_price(),
        "outstanding": company.outstanding(),
        "on_offer": company.on_offer(),
    }))
}
