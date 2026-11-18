//! The verb `release`: an officer lets a resident out of custody.
//!
//! - A suspect that is under arrest and not charged: the officer needs
//!   `law.arrest`. The warrants of the suspect are open again.
//! - An inmate that serves a sentence: the officer needs `law.jail`. The
//!   rest of the sentence is gone.
//!
//! A suspect that waits for a verdict stays: only the court ends a trial.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, Dest, EventMeta, ResidentId};

use super::need_power;
use crate::custody;
use crate::ids::Handles;
use crate::msg::Report;
use crate::names::{event, notice, perm};
use crate::powers;
use crate::record::Record;
use crate::verbs::{in_range, spec};

/// Arguments of `release`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReleaseArgs {
    /// The resident to let go. It must stand on your tile or on a tile
    /// next to it.
    pub target: ResidentId,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<ReleaseArgs>(
        spec(
            "release",
            "Police: let a resident next to you out of custody. `law.arrest` frees a suspect that is not charged. `law.jail` ends the sentence of an inmate.",
        ),
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::ARREST, perm::JAIL])?;
            let target = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.arrest_range))?;
            let city = powers::city_of(realm);
            let detained = realm.has_condition(target, h.detained);
            let jailed = realm.has_condition(target, h.jailed);
            if !detained && !jailed {
                return Err(ActionError::conflict("This resident is not in custody."));
            }
            if detained {
                if !realm.col::<Record>()[target].trials.is_empty() {
                    return Err(ActionError::conflict(
                        "This resident waits for a verdict. Only the court ends a trial.",
                    ));
                }
                if !powers::has_power(realm, ctx.idx, perm::ARREST, city) {
                    return Err(ActionError::not_allowed(
                        "You need `law.arrest` to free a suspect that is under arrest.",
                    ));
                }
                custody::end(realm, &h, target, "released", Some(ctx.resident));
                realm.send(Dest::Global, &Report::Released { target: args.target });
            } else {
                if !powers::has_power(realm, ctx.idx, perm::JAIL, city) {
                    return Err(ActionError::not_allowed(
                        "You need `law.jail` to end the sentence of an inmate.",
                    ));
                }
                realm.clear_condition(target, h.jailed);
                realm.notify(
                    args.target,
                    notice::RELEASED,
                    Some(ctx.resident),
                    json!({ "reason": "paroled", "free": true }),
                );
                realm.emit(
                    EventMeta::public(event::RELEASED, h.drama(realm, h.p.drama_released))
                        .actor(args.target)
                        .at(realm.pos(target)),
                    &json!({ "target": args.target, "reason": "paroled", "by": ctx.resident }),
                );
            }
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.arrest_ticks) })
        },
        |_realm, _ctx, args| Ok(json!({ "target": args.target, "released": true })),
    );
}
