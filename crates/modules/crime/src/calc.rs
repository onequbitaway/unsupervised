//! The formulas of the module. They use integers only. A chance is in
//! basis points (1/10,000). See `docs/design/modules/crime.md`.

use unsup_kernel::{AttrId, Idx, Realm};
use unsup_life::skill_level;

use crate::ids::Handles;

/// Basis points in one whole.
pub(crate) const WHOLE_BP: i64 = 10_000;

/// Returns `amount * bp / 10,000`, rounded down. A negative input gives 0.
pub(crate) fn share(amount: i64, bp: i64) -> i64 {
    let amount = i128::from(amount.max(0));
    let bp = i128::from(bp.max(0));
    i64::try_from(amount * bp / i128::from(WHOLE_BP)).unwrap_or(i64::MAX)
}

/// Returns `amount * percent / 100`, rounded down. A negative input gives
/// 0.
pub(crate) fn percent(amount: i64, percent: i64) -> i64 {
    share(amount, percent.saturating_mul(100))
}

/// Returns `amount * percent / 100`, rounded up. A negative input gives 0.
pub(crate) fn percent_up(amount: i64, percent: i64) -> i64 {
    let amount = i128::from(amount.max(0));
    let percent = i128::from(percent.max(0));
    i64::try_from((amount * percent + 99) / 100).unwrap_or(i64::MAX)
}

/// Returns `a / b`, rounded up, for numbers that are not negative. Returns
/// 0 when `b` is 0 or less.
pub(crate) fn div_up(a: i64, b: i64) -> i64 {
    if b <= 0 || a <= 0 {
        return 0;
    }
    (a - 1) / b + 1
}

/// Returns the level of a skill of a resident.
pub(crate) fn level(realm: &Realm, idx: Idx, skill: AttrId) -> i64 {
    skill_level(realm.attr(idx, skill))
}

/// Returns true when it is night: the hour is from `crime.night_from_hour`
/// up to, and not including, `crime.night_to_hour`.
pub(crate) fn is_night(realm: &Realm, h: &Handles) -> bool {
    let hour = i64::from(realm.clock().hour);
    let from = realm.param(h.p.night_from);
    let to = realm.param(h.p.night_to);
    if from <= to { (from..to).contains(&hour) } else { hour >= from || hour < to }
}

/// Puts a chance between `crime.chance_min_bp` and `crime.chance_max_bp`.
pub(crate) fn clamp_chance(realm: &Realm, h: &Handles, raw: i64) -> u32 {
    let least = realm.param(h.p.chance_min).clamp(0, WHOLE_BP);
    let most = realm.param(h.p.chance_max).clamp(least, WHOLE_BP);
    u32::try_from(raw.clamp(least, most)).unwrap_or(0)
}

/// Rolls a chance in basis points with the random stream of the realm.
pub(crate) fn roll(realm: &Realm, chance_bp: u32) -> bool {
    realm.rng().chance_bp(chance_bp)
}

/// Returns a chance in basis points between 0 and 10,000, without the
/// limits of [`clamp_chance`]. The chances of witnesses and of the city
/// watch use it: they can be 0.
pub(crate) fn plain_chance(raw: i64) -> u32 {
    u32::try_from(raw.clamp(0, WHOLE_BP)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shares_round_down_and_never_go_negative() {
        assert_eq!(share(1_000, 2_500), 250);
        assert_eq!(share(999, 6_000), 599);
        assert_eq!(share(-5, 6_000), 0);
        assert_eq!(share(5, -1), 0);
        assert_eq!(percent(1_001, 25), 250);
        assert_eq!(percent_up(1_001, 25), 251);
        assert_eq!(percent_up(4, 25), 1);
        assert_eq!(percent_up(0, 25), 0);
        assert_eq!(share(i64::MAX, 10_000), i64::MAX);
    }

    #[test]
    fn division_rounds_up() {
        assert_eq!(div_up(10, 10), 1);
        assert_eq!(div_up(11, 10), 2);
        assert_eq!(div_up(0, 10), 0);
        assert_eq!(div_up(5, 0), 0);
        assert_eq!(plain_chance(-3), 0);
        assert_eq!(plain_chance(12_000), 10_000);
    }
}
