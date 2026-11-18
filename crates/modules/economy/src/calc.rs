//! The formulas of the module. All of them use integers and round down.
//!
//! `docs/design/modules/economy.md` gives the same formulas in words.

/// One whole in basis points.
pub(crate) const BP: i64 = 10_000;
/// Game minutes in one hour.
const HOUR: i64 = 60;
/// The greatest value of a need, for example `need.mood`.
const NEED_MAX: i64 = 1_000;
/// Experience points per level, squared: level = floor(sqrt(xp / 100)).
const XP_PER_LEVEL_SQUARED: i64 = 100;

fn to_i64(value: i128) -> i64 {
    i64::try_from(value).unwrap_or(if value < 0 { i64::MIN } else { i64::MAX })
}

/// Returns the skill level of an amount of experience:
/// `floor(sqrt(xp / 100))`.
pub(crate) fn level(xp: i64) -> i64 {
    let whole = u64::try_from(xp / XP_PER_LEVEL_SQUARED).unwrap_or(0);
    i64::try_from(whole.isqrt()).unwrap_or(i64::MAX)
}

/// Returns `amount * bp / 10,000`, rounded down. Taxes use it.
pub(crate) fn share(amount: i64, bp: i64) -> i64 {
    to_i64(i128::from(amount) * i128::from(bp) / i128::from(BP))
}

/// Returns `price * qty`, or `None` when it does not fit.
pub(crate) fn total(price: i64, qty: i64) -> Option<i64> {
    price.checked_mul(qty)
}

/// Returns the share of the normal output, in basis points, that a mood
/// gives: `floor_bp` at mood 0, 10,000 at mood 1,000, linear between.
pub(crate) fn mood_bp(mood: i64, floor_bp: i64) -> i64 {
    let floor_bp = floor_bp.clamp(0, BP);
    floor_bp + (BP - floor_bp) * mood.clamp(0, NEED_MAX) / NEED_MAX
}

/// Returns the experience for `minutes` of work.
pub(crate) fn xp_gain(per_hour: i64, minutes: i64, mood_bp: i64) -> i64 {
    to_i64(i128::from(per_hour) * i128::from(minutes) * i128::from(mood_bp) / i128::from(HOUR * BP))
        .max(0)
}

/// Returns the energy that `minutes` of work cost.
pub(crate) fn energy_cost(per_hour: i64, minutes: i64) -> i64 {
    to_i64(i128::from(per_hour) * i128::from(minutes) / i128::from(HOUR)).max(0)
}

/// Returns the units that `minutes` of gathering give, before the limit of
/// the node: `per_hour * minutes / 60 * (1 + level * bonus) * mood`.
pub(crate) fn harvest(per_hour: i64, minutes: i64, level: i64, bonus_bp: i64, mood_bp: i64) -> i64 {
    let skill_bp = i128::from(BP) + i128::from(level) * i128::from(bonus_bp);
    to_i64(
        i128::from(per_hour) * i128::from(minutes) * skill_bp * i128::from(mood_bp)
            / (i128::from(HOUR) * i128::from(BP) * i128::from(BP)),
    )
    .max(0)
}

/// Returns the speed of a producer in basis points:
/// `10,000 + extra_levels * speed_bonus_bp`.
fn speed_bp(extra_levels: i64, speed_bonus_bp: i64) -> i128 {
    i128::from(BP) + i128::from(extra_levels.max(0)) * i128::from(speed_bonus_bp.max(0))
}

/// Returns the ticks that `batches` of a recipe take, rounded up, at least
/// one.
pub(crate) fn produce_ticks(minutes: i64, batches: i64, extra_levels: i64, bonus_bp: i64) -> i64 {
    let speed = speed_bp(extra_levels, bonus_bp);
    let work = i128::from(minutes) * i128::from(batches) * i128::from(BP);
    to_i64((work + speed - 1) / speed).max(1)
}

/// Returns the batches of a recipe that fit in a shift, rounded down.
pub(crate) fn shift_batches(shift: i64, minutes: i64, extra_levels: i64, bonus_bp: i64) -> i64 {
    if minutes <= 0 {
        return 0;
    }
    let speed = speed_bp(extra_levels, bonus_bp);
    to_i64(i128::from(shift) * speed / (i128::from(minutes) * i128::from(BP))).max(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_the_root_of_a_hundredth_of_the_experience() {
        assert_eq!(level(-5), 0);
        assert_eq!(level(0), 0);
        assert_eq!(level(99), 0);
        assert_eq!(level(100), 1);
        assert_eq!(level(399), 1);
        assert_eq!(level(400), 2);
        assert_eq!(level(10_000), 10);
    }

    #[test]
    fn shares_round_down() {
        assert_eq!(share(2_000, 1_000), 200);
        assert_eq!(share(1_999, 1_000), 199);
        assert_eq!(share(9, 500), 0);
        assert_eq!(share(i64::MAX, BP), i64::MAX);
        assert_eq!(total(3, 4), Some(12));
        assert_eq!(total(i64::MAX, 2), None);
    }

    #[test]
    fn mood_scales_between_the_floor_and_one() {
        assert_eq!(mood_bp(1_000, 5_000), 10_000);
        assert_eq!(mood_bp(0, 5_000), 5_000);
        assert_eq!(mood_bp(500, 5_000), 7_500);
        assert_eq!(mood_bp(2_000, 5_000), 10_000);
        assert_eq!(mood_bp(-1, 5_000), 5_000);
    }

    #[test]
    fn experience_and_energy_follow_the_minutes() {
        assert_eq!(xp_gain(10, 240, 10_000), 40);
        assert_eq!(xp_gain(10, 240, 5_000), 20);
        assert_eq!(xp_gain(10, 5, 10_000), 0);
        assert_eq!(energy_cost(20, 240), 80);
        assert_eq!(energy_cost(20, 30), 10);
    }

    #[test]
    fn harvest_grows_with_level_and_mood() {
        assert_eq!(harvest(6, 60, 0, 1_000, 10_000), 6);
        assert_eq!(harvest(6, 240, 0, 1_000, 10_000), 24);
        // Level 5 adds 50 percent.
        assert_eq!(harvest(6, 60, 5, 1_000, 10_000), 9);
        // Mood 0 with a floor of one half.
        assert_eq!(harvest(6, 60, 0, 1_000, 5_000), 3);
        assert_eq!(harvest(6, 5, 0, 1_000, 10_000), 0);
    }

    #[test]
    fn production_is_faster_with_extra_levels() {
        assert_eq!(produce_ticks(20, 3, 0, 500), 60);
        // Two extra levels: 60 / 1.1 = 54.5, rounded up.
        assert_eq!(produce_ticks(20, 3, 2, 500), 55);
        assert_eq!(produce_ticks(1, 1, 100, 10_000), 1);
        assert_eq!(shift_batches(240, 20, 0, 500), 12);
        assert_eq!(shift_batches(240, 20, 2, 500), 13);
        assert_eq!(shift_batches(240, 300, 0, 500), 0);
        assert_eq!(shift_batches(240, 0, 0, 500), 0);
    }
}
