//! The verbs of trade: `stock`, `unstock`, `set_price`, `buy` and `sell`.
//!
//! A shop has a stock (the goods of `Holder::Site`) and a till (its
//! money). Sales go into the till. The shop pays the sales tax out of the
//! price. At a depot, `buy` and `sell` trade with the system instead.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Dest, Holder, ItemId, Money, Realm, Site};

use super::Phase;
use crate::calc;
use crate::events::{self, Sale};
use crate::ids::Econ;
use crate::labour;
use crate::site;
use crate::state::jobs::JobBook;
use crate::state::prices::{Price, PriceBook};

/// Arguments of `stock` and `unstock`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StockArgs {
    /// Name of the good.
    pub item: String,
    /// How many units to move.
    pub qty: i64,
}

/// Arguments of `set_price`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetPriceArgs {
    /// Name of the good.
    pub item: String,
    /// Cents that a customer pays for one unit, sales tax included. Absent:
    /// the shop does not sell the good.
    #[serde(default)]
    pub price: Option<i64>,
    /// Cents that the shop pays to a resident for one unit. Absent: the
    /// shop does not buy the good.
    #[serde(default)]
    pub buy_price: Option<i64>,
}

/// Arguments of `buy`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct BuyArgs {
    /// Name of the good.
    pub item: String,
    /// How many units to buy.
    pub qty: i64,
    /// Greatest price in cents for one unit that you accept. The action
    /// fails with `conflict` when the price is higher.
    #[serde(default)]
    pub max_price: Option<i64>,
}

/// Arguments of `sell`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SellArgs {
    /// Name of the good.
    pub item: String,
    /// How many units to sell.
    pub qty: i64,
    /// Least price in cents for one unit that you accept. The action fails
    /// with `conflict` when the price is lower.
    #[serde(default)]
    pub min_price: Option<i64>,
}

fn check_qty(econ: &Econ, realm: &Realm, qty: i64) -> Result<(), ActionError> {
    let most = realm.param(econ.ids.trade_qty_max);
    if qty < 1 || qty > most {
        return Err(ActionError::invalid_args(format!("`qty` must be from 1 to {most}.")));
    }
    Ok(())
}

/// The error for too few goods or too little money.
fn short(what: &str, needed: i64, available: i64) -> ActionError {
    ActionError::insufficient(what)
        .with_details(json!({ "needed": needed, "available": available }))
}

/// The verb `stock`: moves goods from the resident into the stock of the
/// site.
pub(crate) fn stock(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &StockArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let employed = realm.res::<JobBook>().job_of(here.id, ctx.resident).is_some();
    if !employed && !site::controls(realm, ctx.idx, &here) {
        return Err(ActionError::not_allowed(
            "Only the owner of this site, a member of its owner with `site.manage`, or a worker of the site can stock it.",
        ));
    }
    let item = labour::good(realm, &args.item)?;
    check_qty(econ, realm, args.qty)?;
    let me = Holder::Resident(ctx.resident);
    let have = realm.ledger().qty(me, item);
    if have < args.qty {
        return Err(short("You do not carry that many units.", args.qty, have));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let place = Holder::Site(here.id);
    realm.ledger_mut().transfer_item(me, place, item, args.qty, "stock")?;
    let in_stock = realm.ledger().qty(place, item);
    Ok(json!({ "site": here.id, "item": args.item, "qty": args.qty, "in_stock": in_stock }))
}

/// The verb `unstock`: moves goods from the stock of the site to the
/// resident.
pub(crate) fn unstock(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &StockArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    let item = labour::good(realm, &args.item)?;
    check_qty(econ, realm, args.qty)?;
    let place = Holder::Site(here.id);
    let have = realm.ledger().qty(place, item);
    if have < args.qty {
        return Err(short("The stock of this site has fewer units.", args.qty, have));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    realm.ledger_mut().transfer_item(
        place,
        Holder::Resident(ctx.resident),
        item,
        args.qty,
        "unstock",
    )?;
    let in_stock = realm.ledger().qty(place, item);
    Ok(json!({ "site": here.id, "item": args.item, "qty": args.qty, "in_stock": in_stock }))
}

/// Checks that the site is a shop with a permit.
fn check_retail(econ: &Econ, here: &Site) -> Result<(), ActionError> {
    if !econ.rules.kind(&here.kind).retail {
        return Err(ActionError::not_here(format!(
            "A {} does not trade with residents. Go to a shop, a restaurant or a depot.",
            here.kind
        )));
    }
    if !site::is_open(here) {
        return Err(ActionError::not_allowed("This business has no permit. It does not trade."));
    }
    Ok(())
}

/// The verb `set_price`: sets what the shop takes and pays for a good.
pub(crate) fn set_price(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &SetPriceArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::controls(realm, ctx.idx, &here) {
        return Err(site::not_controller());
    }
    check_retail(econ, &here)?;
    let item = labour::good(realm, &args.item)?;
    let most = realm.param(econ.ids.price_max);
    for price in [args.price, args.buy_price].into_iter().flatten() {
        if price < 1 || price > most {
            return Err(ActionError::invalid_args(format!(
                "A price must be from 1 to {most} cents. Leave it out to stop the offer."
            )));
        }
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let price = Price { sell: args.price, buy: args.buy_price };
    realm.res_mut::<PriceBook>().set(here.id, item, price);
    Ok(json!({
        "site": here.id,
        "item": args.item,
        "price": args.price,
        "buy_price": args.buy_price,
    }))
}

/// The verb `buy`: buys goods from the shop or the depot where the
/// resident is.
pub(crate) fn buy(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &BuyArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let ids = &econ.ids;
    let here = site::here(realm, ctx.idx)?;
    let item = labour::good(realm, &args.item)?;
    check_qty(econ, realm, args.qty)?;
    let depot = econ.rules.kind(&here.kind).depot;
    let unit_price = if depot {
        depot_price(econ, realm, item, &args.item, |p| p.ceiling)?
    } else {
        check_retail(econ, &here)?;
        realm.res::<PriceBook>().get(here.id, item).sell.ok_or_else(|| {
            ActionError::not_found(format!("This shop does not sell `{}`.", args.item))
        })?
    };
    if args.max_price.is_some_and(|max| unit_price > max) {
        return Err(ActionError::conflict(format!(
            "One unit costs {unit_price} cents. That is above your limit."
        ))
        .with_details(json!({ "unit_price": unit_price })));
    }
    let place = Holder::Site(here.id);
    if !depot {
        let in_stock = realm.ledger().qty(place, item);
        if in_stock < args.qty {
            return Err(short("The shop has fewer units in stock.", args.qty, in_stock));
        }
    }
    let total = calc::total(unit_price, args.qty)
        .ok_or_else(|| ActionError::invalid_args("The amount is too large."))?;
    let me = Holder::Resident(ctx.resident);
    let cash = realm.ledger().money(me).0;
    if cash < total {
        return Err(short("You do not have enough cash.", total, cash));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let tax = if depot {
        // The depot is the system: the money leaves the world and the
        // goods enter it.
        let mut ledger = realm.ledger_mut();
        ledger.burn(me, ids.import_money, Money(total))?;
        ledger.produce(ids.import_goods, me, item, args.qty);
        0
    } else {
        let tax = calc::share(total, realm.param(ids.sales_tax_bp));
        {
            let mut ledger = realm.ledger_mut();
            ledger.transfer_money(me, place, Money(total), "sale")?;
            ledger.transfer_item(place, me, item, args.qty, "sale")?;
        }
        realm.wire_money(place, Dest::Global, site::city_holder(realm), Money(tax), "sales_tax")?;
        tax
    };
    let sale = Sale {
        seller: here.owner.unwrap_or(place),
        buyer: Some(ctx.resident),
        site: here.id,
        city: site::city(realm),
        item: &args.item,
        qty: args.qty,
        unit_price,
        tax,
    };
    events::goods_sold(realm, ctx.resident, realm.pos(ctx.idx), &sale);
    Ok(json!({
        "site": here.id,
        "item": args.item,
        "qty": args.qty,
        "unit_price": unit_price,
        "total": total,
        "tax": tax,
    }))
}

/// The verb `sell`: sells goods to the shop or the depot where the
/// resident is.
pub(crate) fn sell(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &SellArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let ids = &econ.ids;
    let here = site::here(realm, ctx.idx)?;
    let item = labour::good(realm, &args.item)?;
    check_qty(econ, realm, args.qty)?;
    let depot = econ.rules.kind(&here.kind).depot;
    let unit_price = if depot {
        depot_price(econ, realm, item, &args.item, |p| p.floor)?
    } else {
        check_retail(econ, &here)?;
        realm.res::<PriceBook>().get(here.id, item).buy.ok_or_else(|| {
            ActionError::not_found(format!("This shop does not buy `{}`.", args.item))
        })?
    };
    if args.min_price.is_some_and(|min| unit_price < min) {
        return Err(ActionError::conflict(format!(
            "One unit gives {unit_price} cents. That is below your limit."
        ))
        .with_details(json!({ "unit_price": unit_price })));
    }
    let me = Holder::Resident(ctx.resident);
    let have = realm.ledger().qty(me, item);
    if have < args.qty {
        return Err(short("You do not carry that many units.", args.qty, have));
    }
    let total = calc::total(unit_price, args.qty)
        .ok_or_else(|| ActionError::invalid_args("The amount is too large."))?;
    let place = Holder::Site(here.id);
    if !depot {
        let till = realm.ledger().money(place).0;
        if till < total {
            return Err(short("The till of the shop cannot pay that much.", total, till));
        }
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    {
        let mut ledger = realm.ledger_mut();
        if depot {
            // The depot is the system: the goods leave the world and the
            // money enters it.
            ledger.consume(me, ids.export_goods, item, args.qty)?;
            ledger.mint(ids.export_money, me, Money(total));
        } else {
            ledger.transfer_item(me, place, item, args.qty, "purchase")?;
            ledger.transfer_money(place, me, Money(total), "purchase")?;
        }
    }
    realm.attr_add(ctx.idx, ids.trading, realm.param(ids.xp_per_sale));
    let buyer = match here.owner {
        Some(Holder::Resident(owner)) if !depot => Some(owner),
        _ => None,
    };
    let sale = Sale {
        seller: me,
        buyer,
        site: here.id,
        city: site::city(realm),
        item: &args.item,
        qty: args.qty,
        unit_price,
        tax: 0,
    };
    events::goods_sold(realm, ctx.resident, realm.pos(ctx.idx), &sale);
    Ok(json!({
        "site": here.id,
        "item": args.item,
        "qty": args.qty,
        "unit_price": unit_price,
        "total": total,
    }))
}

/// Returns the floor or the ceiling price of a good at the depot.
fn depot_price(
    econ: &Econ,
    realm: &Realm,
    item: ItemId,
    name: &str,
    pick: impl Fn(&crate::rules::DepotPrice) -> unsup_kernel::ParamId,
) -> Result<i64, ActionError> {
    let prices = econ
        .rules
        .depot
        .get(&item)
        .ok_or_else(|| ActionError::not_found(format!("The depot does not trade `{name}`.")))?;
    Ok(realm.param(pick(prices)))
}
