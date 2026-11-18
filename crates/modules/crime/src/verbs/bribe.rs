//! The verb `bribe`: a secret payment to a resident next to you.
//!
//! A gift with `give` is public. A bribe is not: no viewer and no
//! bystander sees it. It is a crime of the giver. The case is hidden
//! until the receiver reports it or an investigation at the scene finds
//! it. The engine does not enforce what the bribe was for.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{ActionError, Begin, Builder, Holder, Money, ResidentId, Untrusted};

use super::{in_range, spec};
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::names::{label, notice};
use crate::record::{Known, Record, Role};

/// Arguments of `bribe`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct BribeArgs {
    /// The resident that gets the money. It must stand on your tile or on
    /// a tile next to it.
    pub to: ResidentId,
    /// Cents of cash to pay.
    pub amount: i64,
    /// A short text for the receiver, for example what you ask for. The
    /// engine does not enforce it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Checks the text of a note: not blank, not too long, and no control
/// character except a line break.
fn check_note(note: &str, max: usize) -> Result<(), ActionError> {
    if note.trim().is_empty() {
        return Err(ActionError::invalid_args("`note` must not be blank."));
    }
    if note.len() > max {
        return Err(ActionError::invalid_args(format!("`note` has more than {max} bytes.")));
    }
    if note.chars().any(|c| c.is_control() && c != '\n') {
        return Err(ActionError::invalid_args("`note` has a control character."));
    }
    Ok(())
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<BribeArgs>(
        spec(
            "bribe",
            "Pay cash in secret to a resident next to you. Nobody else sees it. It is a crime that the receiver can report and an investigation can find.",
        ),
        move |realm, ctx, args| {
            if args.amount < 1 {
                return Err(ActionError::invalid_args("`amount` must be at least 1."));
            }
            if let Some(note) = &args.note {
                check_note(note, h.count(realm, h.p.bribe_note_max))?;
            }
            let receiver = in_range(realm, ctx.idx, args.to, h.tiles(realm, h.p.bribe_range))?;
            let (from, to) = (Holder::Resident(ctx.resident), Holder::Resident(args.to));
            realm.ledger_mut().transfer_money(from, to, Money(args.amount), label::BRIBE)?;

            let case = commit::new_case(realm);
            let pos = realm.pos(ctx.idx);
            let deed = Deed {
                kind: CrimeKind::Bribery,
                perp: ctx.idx,
                pos,
                site: realm.sites().at(pos).map(|site| site.id),
                victim: None,
                target: None,
                owner: None,
                value: args.amount,
                failed: false,
                force: false,
            };
            commit::file(realm, &h, case, &deed, Finder::Nobody);
            // The receiver knows who paid. It can report the case.
            let known = Known {
                case,
                kind: CrimeKind::Bribery,
                role: Role::Receiver,
                suspect: Some(ctx.resident),
                reported: false,
            };
            realm.col_mut::<Record>()[receiver].add_known(known, h.count(realm, h.p.known_max));
            let mut data = Map::new();
            data.insert("case".into(), json!(case));
            data.insert("from".into(), json!(ctx.resident));
            data.insert("amount".into(), json!(args.amount));
            if let Some(note) = &args.note {
                data.insert("note".into(), json!(Untrusted::new(ctx.resident, note.clone())));
            }
            realm.notify(args.to, notice::BRIBE, Some(ctx.resident), Value::Object(data));
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.bribe_ticks) })
        },
        |_realm, _ctx, args| Ok(json!({ "to": args.to, "amount": args.amount })),
    );
}
