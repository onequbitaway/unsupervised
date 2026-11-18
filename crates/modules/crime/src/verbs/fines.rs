//! The verb `pay_fine`: a convict pays what it owes to the court.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Holder, VerbSpec};

use super::{COURT_ALLOWED_IN, MODULE};
use crate::debts;
use crate::ids::Handles;
use crate::names::event;
use crate::record::Record;

/// Arguments of `pay_fine`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct PayFineArgs {
    /// Cents to pay. Without it, you pay all that you owe, or all the cash
    /// that you have.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<i64>,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.instant_verb::<PayFineArgs>(
        VerbSpec {
            verb: "pay_fine",
            module: MODULE,
            summary: "Pay fines and restitutions that a court gave you, from your cash. What you do not pay in time becomes jail time.",
            free: false,
            allowed_in: COURT_ALLOWED_IN,
        },
        move |realm, ctx, args| {
            if args.amount.is_some_and(|amount| amount < 1) {
                return Err(ActionError::invalid_args("`amount` must be at least 1."));
            }
            let owed = realm.col::<Record>()[ctx.idx].owed();
            if owed == 0 {
                return Err(ActionError::conflict("You owe nothing to a court."));
            }
            let cash = realm.ledger().money(Holder::Resident(ctx.resident)).cents();
            if cash < 1 {
                return Err(ActionError::insufficient("You have no cash.")
                    .with_details(json!({ "needed": owed, "available": cash })));
            }
            let paid = debts::pay(realm, &h, ctx.idx, args.amount.unwrap_or(owed), None);
            let left = owed - paid;
            realm.emit(
                EventMeta::public(event::FINE_PAID, h.drama(realm, h.p.drama_fine_paid))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({ "resident": ctx.resident, "amount": paid, "owed": left }),
            );
            Ok(json!({ "paid": paid, "owed": left }))
        },
    );
}
