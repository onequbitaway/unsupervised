//! Small helpers that every part of the module uses: integer money
//! arithmetic, the place check and the waiting time of a counter verb.

use serde_json::{Value, json};
use unsup_kernel::{
    ActionError, Begin, Idx, ParamId, Realm, ResidentId, SiteId, TICKS_PER_DAY, VerbSpec,
};

use crate::params::Params;

/// One whole in basis points.
pub(crate) const BP: i64 = 10_000;

/// The name of the module in verb descriptions.
pub(crate) const MODULE: &str = "finance";

/// Returns `floor(a * b / c)` without overflow in the product. `c` must be
/// greater than zero; a `c` of zero or less gives 0. A result that does not
/// fit is clamped.
pub(crate) fn mul_div(a: i64, b: i64, c: i64) -> i64 {
    if c <= 0 {
        return 0;
    }
    let value = (i128::from(a) * i128::from(b)).div_euclid(i128::from(c));
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// Returns the part of `amount` that `bp` basis points are, rounded down.
pub(crate) fn part(amount: i64, bp: i64) -> i64 {
    mul_div(amount, bp, BP)
}

/// Returns the number of whole game days since the world began. Day 0 is the
/// first day. The calendar follows the frame number.
pub(crate) fn day_of(realm: &Realm) -> u64 {
    realm.frame() / TICKS_PER_DAY
}

/// Returns the calendar form of a day number (see [`day_of`]): the year and
/// the day of the year, both from 1, as residents see them in the clock.
pub(crate) fn calendar_day(realm: &Realm, day: u64) -> Value {
    let clock = realm.layout().clock_at(day.saturating_mul(TICKS_PER_DAY));
    json!({ "year": clock.year, "day": clock.day })
}

/// Returns the value of a parameter as a count that is not negative.
pub(crate) fn count(realm: &Realm, param: ParamId) -> usize {
    usize::try_from(realm.param(param)).unwrap_or(0)
}

/// Returns the drama score of an event from its parameter.
pub(crate) fn drama(realm: &Realm, param: ParamId) -> u8 {
    u8::try_from(realm.param(param).clamp(0, 100)).unwrap_or(0)
}

/// Returns the site of the given kind at which the resident is: the site
/// with the lowest id whose rectangle contains the position of the resident.
pub(crate) fn site_of_kind(realm: &Realm, idx: Idx, kind: &str) -> Option<SiteId> {
    let pos = realm.pos(idx);
    realm.sites().of_kind(kind).find(|site| site.rect.contains(pos)).map(|site| site.id)
}

/// Checks that the resident is at a site of the given kind.
pub(crate) fn require_site(realm: &Realm, idx: Idx, kind: &str) -> Result<SiteId, ActionError> {
    site_of_kind(realm, idx, kind).ok_or_else(|| {
        ActionError::not_here(format!("You must be at a site of the kind `{kind}` to do this."))
    })
}

/// The answer of a zone verb whose effect is in the global realm: wait for
/// the answer, at most `kernel.global_timeout` ticks.
pub(crate) fn wait_for_global(realm: &Realm, p: &Params) -> Begin {
    Begin::Wait { timeout: u32::try_from(realm.param(p.global_timeout).max(1)).unwrap_or(1) }
}

/// The error of an amount that is zero or negative.
pub(crate) fn positive(amount: i64, what: &str) -> Result<i64, ActionError> {
    if amount > 0 {
        Ok(amount)
    } else {
        Err(ActionError::invalid_args(format!("The {what} must be greater than zero.")))
    }
}

/// The error of a resident without a bank account.
pub(crate) fn no_account(resident: ResidentId, own: bool) -> ActionError {
    if own {
        ActionError::not_found("You have no bank account. Use `open_account` at a bank.")
    } else {
        ActionError::not_found(format!("Resident {resident} has no bank account."))
    }
}

/// Makes the description of a verb of this module.
pub(crate) const fn spec(verb: &'static str, summary: &'static str, free: bool) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free, allowed_in: &[] }
}

/// Makes the description of a verb that works from a hospital bed: it only
/// uses the bank account.
pub(crate) const fn remote(verb: &'static str, summary: &'static str, free: bool) -> VerbSpec {
    VerbSpec { verb, module: MODULE, summary, free, allowed_in: &["hospitalized"] }
}
