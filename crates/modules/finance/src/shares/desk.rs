//! `issue_shares` and `pay_dividend`: the verbs of the treasurer of an
//! organization.
//!
//! Memberships live in the zones. So each verb checks the permission
//! `treasury.spend` in the zone, sends a message to the global realm and
//! waits for the answer.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionCtx, ActionError, Builder, Dest, EventMeta, Holder, Message, OrgId, Realm, ResidentId,
    Scope,
};

use super::engine;
use super::state::{ShareOrder, Shares};
use super::trade::{check_numbers, check_order_count};
use crate::bank::Bank;
use crate::exchange::Side;
use crate::params::Params;
use crate::util::{day_of, drama, positive, spec, wait_for_global};

/// The permission that the treasurer verbs need in the organization.
pub(crate) const TREASURY_PERM: &str = "treasury.spend";

/// Arguments of `issue_shares`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct IssueSharesArgs {
    /// The organization that issues the shares. You need the permission
    /// `treasury.spend` there.
    pub org: OrgId,
    /// Number of new shares to offer.
    pub shares: i64,
    /// Least price of one share in cents. The money goes to the treasury
    /// of the organization.
    pub price: i64,
}

/// Arguments of `pay_dividend`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct PayDividendArgs {
    /// The organization that pays. You need the permission
    /// `treasury.spend` there.
    pub org: OrgId,
    /// The whole amount in cents. The holders share it by their shares.
    pub amount: i64,
}

/// What the treasurer asks the global realm to do.
#[derive(Debug, Serialize, Deserialize)]
enum DeskOp {
    Issue { shares: i64, price: i64 },
    Dividend { amount: i64 },
}

/// A request of a treasurer in a zone to the global realm.
#[derive(Debug, Serialize, Deserialize)]
struct DeskMsg {
    resident: ResidentId,
    action_id: u64,
    org: OrgId,
    op: DeskOp,
}

impl Message for DeskMsg {
    const KIND: &'static str = "finance.shares_desk";
}

/// Checks the permission of the treasurer in the zone.
fn check_treasurer(realm: &Realm, ctx: &ActionCtx, org: OrgId) -> Result<(), ActionError> {
    if realm.can(ctx.idx, org, TREASURY_PERM) {
        Ok(())
    } else {
        Err(ActionError::not_allowed(format!(
            "You need the permission `{TREASURY_PERM}` in organization {org}."
        )))
    }
}

/// Registers the treasurer verbs and the handler of the global realm.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.verb::<IssueSharesArgs>(
        spec(
            "issue_shares",
            "Offer new shares of an organization for sale. The money goes to its treasury.",
            false,
        ),
        move |realm, ctx, args| {
            check_treasurer(realm, ctx, args.org)?;
            check_numbers(realm, &p, args.shares, args.price)?;
            let op = DeskOp::Issue { shares: args.shares, price: args.price };
            let msg =
                DeskMsg { resident: ctx.resident, action_id: ctx.action_id, org: args.org, op };
            realm.send(Dest::Global, &msg);
            Ok(wait_for_global(realm, &p))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.verb::<PayDividendArgs>(
        spec(
            "pay_dividend",
            "Pay money from the treasury of an organization to its shareholders, by their \
             shares.",
            false,
        ),
        move |realm, ctx, args| {
            check_treasurer(realm, ctx, args.org)?;
            let amount = positive(args.amount, "amount")?;
            let op = DeskOp::Dividend { amount };
            let msg =
                DeskMsg { resident: ctx.resident, action_id: ctx.action_id, org: args.org, op };
            realm.send(Dest::Global, &msg);
            Ok(wait_for_global(realm, &p))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    if b.scope() == Scope::Global {
        b.on_message::<DeskMsg>(move |realm, _from, msg| {
            let result = match msg.op {
                DeskOp::Issue { shares, price } => {
                    issue(realm, &p, msg.resident, msg.org, shares, price)
                }
                DeskOp::Dividend { amount } => dividend(realm, &p, msg.resident, msg.org, amount),
            };
            realm.complete_action(msg.resident, msg.action_id, result);
        });
    }
}

fn issue(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    org: OrgId,
    qty: i64,
    price: i64,
) -> Result<Value, ActionError> {
    let mut shares = realm.res_mut::<Shares>();
    let most = realm.param(p.shares_max_per_org);
    let existing = shares.company(org).map_or(0, |c| c.outstanding() + c.on_offer());
    if existing + qty > most {
        return Err(ActionError::not_allowed(format!(
            "An organization has at most {most} shares."
        ))
        .with_details(json!({ "limit": most, "existing": existing })));
    }
    check_order_count(realm, p, &shares, resident)?;
    let id = realm.alloc_id();
    let order = ShareOrder {
        id,
        org,
        placed_by: resident,
        issue: true,
        side: Side::Sell,
        price,
        qty,
        ..ShareOrder::default()
    };
    shares.companies.entry(org.0).or_default().on_offer += qty;
    realm.emit(
        EventMeta::public("finance.shares_issued", drama(realm, p.drama_shares)).actor(resident),
        &json!({ "org": org, "shares": qty, "price": price }),
    );
    let mut bank = realm.res_mut::<Bank>();
    let outcome = engine::place(realm, p, &mut shares, &mut bank, order);
    Ok(json!({
        "order": (outcome.resting > 0).then_some(id),
        "sold": outcome.traded,
        "value": outcome.value,
        "on_offer": outcome.resting,
    }))
}

/// Splits `amount` cents over the holders by their shares. Each holder
/// gets the rounded-down part. The cents that are left go, one each, to the
/// holders with the largest remainders, and to the lowest resident id when
/// remainders are equal. The parts always add up to `amount`.
pub(crate) fn split(amount: i64, holders: &[(u64, i64)]) -> Vec<(u64, i64)> {
    let total: i128 = holders.iter().map(|&(_, held)| i128::from(held)).sum();
    if total <= 0 {
        return Vec::new();
    }
    let mut parts: Vec<(u64, i64, i128)> = holders
        .iter()
        .map(|&(id, held)| {
            let product = i128::from(amount) * i128::from(held);
            // The part is at most `amount`, so it fits.
            (id, i64::try_from(product / total).unwrap_or(0), product % total)
        })
        .collect();
    let paid: i64 = parts.iter().map(|part| part.1).sum();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|&a, &b| parts[b].2.cmp(&parts[a].2).then(parts[a].0.cmp(&parts[b].0)));
    let left = usize::try_from(amount - paid).unwrap_or(0);
    for &at in order.iter().take(left) {
        parts[at].1 += 1;
    }
    parts.into_iter().map(|(id, cents, _)| (id, cents)).collect()
}

fn dividend(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    org: OrgId,
    amount: i64,
) -> Result<Value, ActionError> {
    let mut shares = realm.res_mut::<Shares>();
    let holders: Vec<(u64, i64)> = shares
        .company(org)
        .map(|company| company.holders().map(|(id, holding)| (id.0, holding.held)).collect())
        .unwrap_or_default();
    let outstanding: i64 = holders.iter().map(|&(_, held)| held).sum();
    if outstanding <= 0 {
        return Err(ActionError::not_found(format!("Organization {org} has no shareholders.")));
    }
    let treasury = Holder::Org(org);
    let available = realm.ledger().money(treasury).0;
    if available < amount {
        return Err(ActionError::insufficient("The treasury does not have this amount.")
            .with_details(json!({ "needed": amount, "available": available })));
    }
    let mut bank = realm.res_mut::<Bank>();
    let today = day_of(realm);
    let parts = split(amount, &holders);
    for &(holder, cents) in &parts {
        let holder = ResidentId(holder);
        // A buyer has an account. An heir of shares can be without one:
        // it gets an account here, without a fee, so that the bank counts
        // the dividend as a deposit.
        bank.open(realm, holder, today);
        // It cannot fail: the parts add up to `amount`, which the treasury has.
        let _ = bank.pay_in(realm, treasury, holder, cents, "shares.dividend");
        if cents > 0 {
            realm.notify(
                holder,
                "finance.dividend_received",
                None,
                json!({ "org": org, "amount": cents }),
            );
        }
    }
    if let Some(company) = shares.companies.get_mut(&org.0) {
        company.dividends_paid += amount;
    }
    realm.emit(
        EventMeta::public("finance.dividend_paid", drama(realm, p.drama_shares)).actor(resident),
        &json!({ "org": org, "amount": amount, "shares": outstanding, "holders": parts.len() }),
    );
    Ok(json!({ "paid": amount, "shares": outstanding, "holders": parts.len() }))
}
