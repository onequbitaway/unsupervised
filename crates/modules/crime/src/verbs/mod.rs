//! The verbs of the module that every resident can use.
//!
//! A crime checks its preconditions when it starts and does its work when
//! it ends, after its time cost. The outcome uses a skill of the resident
//! and one roll of the random stream of the zone. A crime that fails is
//! still a crime: the result has `success: false` and a case.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{ActionError, Idx, Realm, ResidentId, VerbSpec};

use crate::commit::Filed;
use crate::ids::Handles;

pub(crate) mod assault;
pub(crate) mod bribe;
pub(crate) mod burgle;
pub(crate) mod fence;
pub(crate) mod fines;
pub(crate) mod pickpocket;
pub(crate) mod report;
pub(crate) mod rob;
pub(crate) mod vandalize;

/// Name of the module in the verb catalogue.
pub(crate) const MODULE: &str = "crime";

/// The blocking conditions in which a resident can still deal with the
/// court: report a crime, pay a fine and read the docket.
pub(crate) const COURT_ALLOWED_IN: &[&str] = &["jailed", "detained", "hospitalized"];

/// Makes the description of a verb that occupies the resident.
pub(crate) const fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free: false, allowed_in: &[] }
}

/// Makes the description of a free verb that also works in custody.
pub(crate) const fn free_spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free: true, allowed_in: COURT_ALLOWED_IN }
}

/// Returns the index of another resident that is at most `range` tiles
/// from the actor, in the same zone.
pub(crate) fn in_range(
    realm: &Realm,
    actor: Idx,
    target: ResidentId,
    range: u32,
) -> Result<Idx, ActionError> {
    if realm.id_of(actor) == target {
        return Err(ActionError::invalid_args("The target must be another resident."));
    }
    let here = realm.pos(actor);
    match realm.idx_of(target) {
        Some(idx) if realm.pos(idx).dist(here) <= range => Ok(idx),
        _ => Err(ActionError::not_here(format!("Resident {target} is not next to you."))),
    }
}

/// Returns true when a resident is in custody: under arrest or in jail.
pub(crate) fn in_custody(realm: &Realm, h: &Handles, idx: Idx) -> bool {
    realm.has_condition(idx, h.detained) || realm.has_condition(idx, h.jailed)
}

/// Refuses a crime against a resident in custody.
pub(crate) fn not_guarded(realm: &Realm, h: &Handles, idx: Idx) -> Result<(), ActionError> {
    if in_custody(realm, h, idx) {
        return Err(ActionError::not_allowed("The guards protect a resident in custody."));
    }
    Ok(())
}

/// Takes health from a resident, down to `floor` at most. Returns the
/// health that it lost and the health that is left.
pub(crate) fn hurt(realm: &Realm, h: &Handles, idx: Idx, damage: i64, floor: i64) -> (i64, i64) {
    let before = realm.attr(idx, h.health);
    let after = realm.attr_set(idx, h.health, (before - damage.max(0)).max(floor).min(before));
    (before - after, after)
}

/// Gives a resident the experience of a crime that worked or failed.
pub(crate) fn learn(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    skill: unsup_kernel::AttrId,
    success: bool,
) {
    let xp = realm.param(if success { h.p.xp_success } else { h.p.xp_fail });
    realm.attr_add(idx, skill, xp.max(0));
}

/// Starts the result of a crime: `success`, `case` and `witnesses`.
pub(crate) fn outcome(success: bool, filed: Filed) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("success".into(), json!(success));
    out.insert("case".into(), json!(filed.case));
    out.insert("witnesses".into(), json!(filed.witnesses));
    out
}
