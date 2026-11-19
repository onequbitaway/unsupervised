//! The central bank: `central_bank_set_rate`, `central_bank_issue` and
//! `central_bank_retire`.
//!
//! A resident with the permission `bank.central` in the central bank
//! organization or in the nation sets the base rate, issues money to the
//! nation treasury and retires treasury money. The permission is checked
//! in the zone, where memberships live. The effect is in the global realm.

mod state;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionCtx, ActionError, Builder, Dest, EventMeta, Holder, Message, Money, OrgId, Realm,
    ResidentId, Scope,
};

pub use self::state::CentralBank;
use crate::params::Params;
use crate::util::{drama, positive, spec, wait_for_global};

/// The permission of the central banker.
pub(crate) const CENTRAL_PERM: &str = "bank.central";

/// The treasury that gets issued money and loses retired money.
pub(crate) const NATION_TREASURY: Holder = Holder::Org(OrgId::NATION);

/// Arguments of `central_bank_set_rate`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SetRateArgs {
    /// The new yearly base rate in basis points (100 is 1 percent).
    pub rate_bp: i64,
}

/// Arguments of `central_bank_issue` and `central_bank_retire`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct CentralAmountArgs {
    /// The amount in cents. It must be greater than zero.
    pub amount: i64,
}

/// What the central banker asks the global realm to do.
#[derive(Debug, Serialize, Deserialize)]
enum CentralOp {
    SetRate { rate_bp: i64 },
    Issue { amount: i64 },
    Retire { amount: i64 },
}

/// A request of the central banker in a zone to the global realm.
#[derive(Debug, Serialize, Deserialize)]
struct CentralMsg {
    resident: ResidentId,
    action_id: u64,
    op: CentralOp,
}

impl Message for CentralMsg {
    const KIND: &'static str = "finance.central";
}

/// Checks in the zone that the resident is a central banker. Only the two
/// reserved organizations count, so that a role in a private organization
/// gives nothing.
fn check_banker(realm: &Realm, ctx: &ActionCtx) -> Result<(), ActionError> {
    let allowed = [OrgId::CENTRAL_BANK, OrgId::NATION]
        .into_iter()
        .any(|org| realm.can(ctx.idx, org, CENTRAL_PERM));
    if allowed {
        Ok(())
    } else {
        Err(ActionError::not_allowed(format!(
            "You need the permission `{CENTRAL_PERM}` in the central bank or in the nation."
        )))
    }
}

/// Sends a request to the global realm and waits for the answer.
fn ask(realm: &Realm, p: &Params, ctx: &ActionCtx, op: CentralOp) -> unsup_kernel::Begin {
    realm.send(Dest::Global, &CentralMsg { resident: ctx.resident, action_id: ctx.action_id, op });
    wait_for_global(realm, p)
}

/// Registers the central bank.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.verb::<SetRateArgs>(
        spec("central_bank_set_rate", "Set the base interest rate of the central bank.", false),
        move |realm, ctx, args| {
            check_banker(realm, ctx)?;
            Ok(ask(realm, &p, ctx, CentralOp::SetRate { rate_bp: args.rate_bp }))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.verb::<CentralAmountArgs>(
        spec("central_bank_issue", "Create money and put it in the nation treasury.", false),
        move |realm, ctx, args| {
            check_banker(realm, ctx)?;
            let amount = positive(args.amount, "amount")?;
            Ok(ask(realm, &p, ctx, CentralOp::Issue { amount }))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.verb::<CentralAmountArgs>(
        spec("central_bank_retire", "Destroy money of the nation treasury.", false),
        move |realm, ctx, args| {
            check_banker(realm, ctx)?;
            let amount = positive(args.amount, "amount")?;
            Ok(ask(realm, &p, ctx, CentralOp::Retire { amount }))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    if b.scope() == Scope::Global {
        b.resource::<CentralBank>();
        b.on_message::<CentralMsg>(move |realm, _from, msg| {
            let result = match msg.op {
                CentralOp::SetRate { rate_bp } => set_rate(realm, &p, msg.resident, rate_bp),
                CentralOp::Issue { amount } => issue(realm, &p, msg.resident, amount),
                CentralOp::Retire { amount } => retire(realm, &p, msg.resident, amount),
            };
            realm.complete_action(msg.resident, msg.action_id, result);
        });
    }
}

fn set_rate(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    rate_bp: i64,
) -> Result<Value, ActionError> {
    let (floor, ceiling) = (realm.param(p.base_rate_floor_bp), realm.param(p.base_rate_ceiling_bp));
    if rate_bp < floor || rate_bp > ceiling {
        return Err(ActionError::not_allowed(format!(
            "The law allows a base rate from {floor} to {ceiling} basis points."
        ))
        .with_details(json!({ "min": floor, "max": ceiling })));
    }
    let old = realm.param(p.base_rate_bp);
    let new = realm.set_param(p.base_rate_bp, rate_bp, None);
    realm.emit(
        EventMeta::public("finance.rate_changed", drama(realm, p.drama_central_bank))
            .actor(resident),
        &json!({ "resident": resident, "old_bp": old, "rate_bp": new }),
    );
    Ok(json!({ "rate_bp": new, "old_bp": old }))
}

/// Returns the error of an amount above what is left of a yearly limit.
fn over_limit(what: &str, limit: i64, used: i64) -> ActionError {
    ActionError::not_allowed(format!(
        "The law allows the central bank to {what} {limit} cents in one year."
    ))
    .with_details(json!({ "limit": limit, "used": used, "left": (limit - used).max(0) }))
}

fn issue(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    amount: i64,
) -> Result<Value, ActionError> {
    let mut central = realm.res_mut::<CentralBank>();
    central.roll(realm.clock().year);
    let limit = realm.param(p.issue_limit_per_year);
    if central.issued.saturating_add(amount) > limit {
        return Err(over_limit("issue", limit, central.issued));
    }
    if realm.ledger_mut().try_mint(p.central_issue, NATION_TREASURY, Money(amount)).is_err() {
        // The ledger creates the whole amount or nothing. It refuses an
        // amount that would take the realm above the supply cap.
        return Err(ActionError::invalid_args("The amount is too large for the treasury."));
    }
    central.issued += amount;
    realm.emit(
        EventMeta::public("finance.money_issued", drama(realm, p.drama_central_bank))
            .actor(resident),
        &json!({ "resident": resident, "amount": amount }),
    );
    Ok(json!({
        "issued": amount,
        "issued_this_year": central.issued,
        "treasury": realm.ledger().money(NATION_TREASURY),
    }))
}

fn retire(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    amount: i64,
) -> Result<Value, ActionError> {
    let mut central = realm.res_mut::<CentralBank>();
    central.roll(realm.clock().year);
    let limit = realm.param(p.retire_limit_per_year);
    if central.retired.saturating_add(amount) > limit {
        return Err(over_limit("retire", limit, central.retired));
    }
    realm.ledger_mut().burn(NATION_TREASURY, p.central_retire, Money(amount))?;
    central.retired += amount;
    realm.emit(
        EventMeta::public("finance.money_retired", drama(realm, p.drama_central_bank))
            .actor(resident),
        &json!({ "resident": resident, "amount": amount }),
    );
    Ok(json!({
        "retired": amount,
        "retired_this_year": central.retired,
        "treasury": realm.ledger().money(NATION_TREASURY),
    }))
}
