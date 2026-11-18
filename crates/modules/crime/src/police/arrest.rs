//! The verb `arrest`: an officer takes a suspect into custody.
//!
//! The engine refuses an arrest without grounds. The grounds are:
//!
//! - a warrant names the suspect, or the suspect has the condition
//!   `wanted` for another reason, or
//! - this officer caught the suspect in the act: it saw the crime, or it
//!   found the loot or contraband with a search, at most
//!   `crime.caught_ticks` ticks ago.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, ResidentId};

use super::{keep_result, kept_result, need_power};
use crate::custody;
use crate::ids::Handles;
use crate::names::perm;
use crate::record::Record;
use crate::verbs::{in_custody, in_range, spec};

/// Arguments of `arrest`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ArrestArgs {
    /// The resident to arrest. It must stand on your tile or on a tile
    /// next to it.
    pub target: ResidentId,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<ArrestArgs>(
        spec(
            "arrest",
            "Police: arrest a resident next to you. You need grounds: a warrant names it, or you caught it in the act. The suspect goes to the jail of the city.",
        ),
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::ARREST])?;
            let target = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.arrest_range))?;
            if in_custody(realm, &h, target) {
                return Err(ActionError::conflict("This resident is in custody already."));
            }
            let cases = {
                let records = realm.col::<Record>();
                let record = &records[target];
                let mut cases: Vec<u64> = record.warrants.iter().map(|w| w.case).collect();
                for case in record.caught_by(ctx.resident, realm.frame()) {
                    if !cases.contains(&case) {
                        cases.push(case);
                    }
                }
                cases
            };
            // The condition `wanted` alone is grounds too: another module
            // can give it to a resident without a case of this module.
            if cases.is_empty() && !realm.has_condition(target, h.wanted) {
                return Err(ActionError::not_allowed(
                    "You have no grounds for this arrest: no warrant names this resident and you did not catch it in the act.",
                ));
            }
            let jail = custody::take(realm, &h, target, Some(ctx.resident), &cases);
            realm.attr_add(ctx.idx, h.law, realm.param(h.p.arrest_xp).max(0));
            keep_result(realm, ctx, &json!({ "target": args.target, "cases": cases, "jail": jail }));
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.arrest_ticks) })
        },
        |realm, ctx, _args| Ok(kept_result(realm, ctx)),
    );
}
