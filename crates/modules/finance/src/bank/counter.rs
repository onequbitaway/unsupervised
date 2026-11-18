//! The bank counter: `open_account`, `deposit` and `withdraw`.
//!
//! The resident stands at a `bank` site in a zone. Deposits live in the
//! global realm. So each verb checks the place in the zone, moves cash by
//! wire, sends a message to the global realm and waits for the answer.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, Builder, Dest, EventMeta, Holder, Message, Money, NoArgs, OrgId, Realm,
    ResidentId, Scope,
};

use super::loans;
use super::state::Bank;
use crate::params::Params;
use crate::util::{day_of, drama, no_account, part, positive, require_site, spec, wait_for_global};

/// The kind of site where the counter verbs work.
const BANK_SITE: &str = "bank";

/// The treasury of the bank: fees and loan interest go in, deposit interest
/// comes out.
pub(crate) const BANK_TREASURY: Holder = Holder::Org(OrgId::CENTRAL_BANK);

/// Arguments of `deposit` and `withdraw`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct AmountArgs {
    /// The amount in cents. It must be greater than zero.
    pub amount: i64,
}

/// What the counter asks the global realm to do.
#[derive(Debug, Serialize, Deserialize)]
enum CounterOp {
    /// Open an account. The fee is already in the bank treasury.
    Open { fee: i64 },
    /// Count a deposit. The cash is already in the account.
    Deposit { amount: i64 },
    /// Pay cash out of the account.
    Withdraw { amount: i64 },
}

/// A request of the bank counter of a zone to the global realm.
#[derive(Debug, Serialize, Deserialize)]
struct CounterMsg {
    resident: ResidentId,
    action_id: u64,
    op: CounterOp,
}

impl Message for CounterMsg {
    const KIND: &'static str = "finance.counter";
}

/// Registers the counter verbs and the handler of the global realm.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.verb::<NoArgs>(
        spec("open_account", "Open a bank account. You must be at a bank.", false),
        move |realm, ctx, _args| {
            require_site(realm, ctx.idx, BANK_SITE)?;
            let fee = realm.param(p.account_open_fee);
            realm.wire_money(
                Holder::Resident(ctx.resident),
                Dest::Global,
                BANK_TREASURY,
                Money(fee),
                "bank.fee",
            )?;
            let op = CounterOp::Open { fee };
            realm.send(
                Dest::Global,
                &CounterMsg { resident: ctx.resident, action_id: ctx.action_id, op },
            );
            Ok(wait_for_global(realm, &p))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.verb::<AmountArgs>(
        spec("deposit", "Put cash into your bank account. You must be at a bank.", false),
        move |realm, ctx, args| {
            require_site(realm, ctx.idx, BANK_SITE)?;
            let amount = positive(args.amount, "amount")?;
            realm.wire_money(
                Holder::Resident(ctx.resident),
                Dest::Global,
                Bank::holder(ctx.resident),
                Money(amount),
                "bank.deposit",
            )?;
            let op = CounterOp::Deposit { amount };
            realm.send(
                Dest::Global,
                &CounterMsg { resident: ctx.resident, action_id: ctx.action_id, op },
            );
            Ok(wait_for_global(realm, &p))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    b.verb::<AmountArgs>(
        spec("withdraw", "Take cash out of your bank account. You must be at a bank.", false),
        move |realm, ctx, args| {
            require_site(realm, ctx.idx, BANK_SITE)?;
            let amount = positive(args.amount, "amount")?;
            let op = CounterOp::Withdraw { amount };
            realm.send(
                Dest::Global,
                &CounterMsg { resident: ctx.resident, action_id: ctx.action_id, op },
            );
            Ok(wait_for_global(realm, &p))
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
    if b.scope() == Scope::Global {
        b.on_message::<CounterMsg>(move |realm, _from, msg| {
            let result = match msg.op {
                CounterOp::Open { fee } => open(realm, &p, msg.resident, fee),
                CounterOp::Deposit { amount } => deposit(realm, msg.resident, amount),
                CounterOp::Withdraw { amount } => withdraw(realm, &p, msg.resident, amount),
            };
            realm.complete_action(msg.resident, msg.action_id, result);
        });
    }
}

/// Sends cash back to a resident whose request the bank refuses.
fn refund(realm: &Realm, from: Holder, resident: ResidentId, amount: i64) {
    // The wire cannot fail: the money arrived from this resident in the
    // message before this one, and nothing ran in between.
    let _ = realm.wire_money(
        from,
        Dest::Resident(resident),
        Holder::Resident(resident),
        Money(amount),
        "bank.refund",
    );
}

fn open(realm: &Realm, p: &Params, resident: ResidentId, fee: i64) -> Result<Value, ActionError> {
    let mut bank = realm.res_mut::<Bank>();
    if !bank.open(realm, resident, day_of(realm)) {
        refund(realm, BANK_TREASURY, resident, fee);
        return Err(ActionError::conflict("You already have a bank account."));
    }
    realm.emit(
        EventMeta::public("finance.account_opened", drama(realm, p.drama_routine)).actor(resident),
        &json!({ "resident": resident }),
    );
    Ok(json!({ "account": resident, "fee": fee, "balance": Bank::balance(realm, resident) }))
}

fn deposit(realm: &Realm, resident: ResidentId, amount: i64) -> Result<Value, ActionError> {
    let mut bank = realm.res_mut::<Bank>();
    if !bank.is_open(resident) {
        refund(realm, Bank::holder(resident), resident, amount);
        return Err(no_account(resident, true));
    }
    bank.wired_in(amount);
    Ok(json!({ "deposited": amount, "balance": Bank::balance(realm, resident) }))
}

fn withdraw(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    amount: i64,
) -> Result<Value, ActionError> {
    let mut bank = realm.res_mut::<Bank>();
    if !bank.is_open(resident) {
        return Err(no_account(resident, true));
    }
    let fee = part(amount, realm.param(p.withdraw_fee_bp));
    let needed = amount.saturating_add(fee);
    loans::check_free(realm, p, &bank, resident, needed, "withdrawal")?;
    bank.pay_out(realm, resident, BANK_TREASURY, fee, "bank.fee")?;
    bank.wire_out(realm, resident, resident, amount, "bank.withdraw")?;
    Ok(json!({ "withdrawn": amount, "fee": fee, "balance": Bank::balance(realm, resident) }))
}
