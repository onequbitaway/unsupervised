//! `transfer`: money from the account of one resident to the account of
//! another, from anywhere.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Builder, Realm, ResidentId};

use super::counter::BANK_TREASURY;
use super::state::Bank;
use crate::params::Params;
use crate::util::{no_account, part, positive, remote};

/// Arguments of `transfer`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TransferArgs {
    /// The resident whose account gets the money.
    pub to: ResidentId,
    /// The amount in cents. It must be greater than zero.
    pub amount: i64,
}

/// Registers the verb `transfer`.
pub(crate) fn register(b: &mut Builder, p: Params) {
    b.global_verb::<TransferArgs>(
        remote(
            "transfer",
            "Send money from your bank account to the account of another resident.",
            true,
        ),
        move |realm, ctx, args| transfer(realm, &p, ctx.resident, args),
    );
}

fn transfer(
    realm: &Realm,
    p: &Params,
    from: ResidentId,
    args: &TransferArgs,
) -> Result<Value, ActionError> {
    let amount = positive(args.amount, "amount")?;
    if args.to == from {
        return Err(ActionError::invalid_args("You cannot transfer money to yourself."));
    }
    let mut bank = realm.res_mut::<Bank>();
    if !bank.is_open(from) {
        return Err(no_account(from, true));
    }
    if !bank.is_open(args.to) {
        return Err(no_account(args.to, false));
    }
    let fee = part(amount, realm.param(p.transfer_fee_bp));
    let balance = Bank::balance(realm, from);
    let needed = amount.saturating_add(fee);
    if balance < needed {
        return Err(ActionError::insufficient("Your balance is too low for this transfer.")
            .with_details(json!({ "needed": needed, "available": balance })));
    }
    bank.pay_out(realm, from, BANK_TREASURY, fee, "bank.fee")?;
    bank.move_between(realm, from, args.to, amount, "bank.transfer")?;
    realm.notify(
        args.to,
        "finance.transfer_received",
        Some(from),
        json!({ "from": from, "amount": amount }),
    );
    Ok(json!({ "sent": amount, "fee": fee, "balance": Bank::balance(realm, from) }))
}
