//! Random numbers with a fraction, on top of the generator of the kernel.
//!
//! The load tools use the generator of the kernel (`unsup_kernel::Rng`), so
//! that one seed gives the same behaviour on every run. The kernel has only
//! integers; think times in seconds need fractions.

use unsup_kernel::Rng;

/// Bits of precision of an `f64` fraction.
const FRACTION_BITS: u32 = 53;

/// Returns a uniform number in `[0, 1)`.
pub fn unit(rng: &mut Rng) -> f64 {
    // The top 53 bits fill the fraction of an `f64` exactly.
    (rng.next_u64() >> (u64::BITS - FRACTION_BITS)) as f64 / (1u64 << FRACTION_BITS) as f64
}

/// Returns true with probability `p`. A `p` at or below 0 is never true and
/// a `p` at or above 1 is always true.
pub fn chance(rng: &mut Rng, p: f64) -> bool {
    if p <= 0.0 {
        false
    } else if p >= 1.0 {
        true
    } else {
        unit(rng) < p
    }
}

/// Returns a number from the exponential distribution with the given mean.
pub fn exponential(rng: &mut Rng, mean: f64) -> f64 {
    // `1 - unit` is in `(0, 1]`, so the logarithm is finite.
    -mean * (1.0 - unit(rng)).ln()
}

/// Returns a number from the standard normal distribution (Box-Muller).
pub fn standard_normal(rng: &mut Rng) -> f64 {
    let radius = (-2.0 * (1.0 - unit(rng)).ln()).sqrt();
    let angle = std::f64::consts::TAU * unit(rng);
    radius * angle.cos()
}

/// Returns a uniform integer from `min` to `max`, both included. Returns
/// `min` when `max` is not greater.
pub fn int_between(rng: &mut Rng, min: i64, max: i64) -> i64 {
    if max <= min {
        return min;
    }
    // The span of two `i64` fits in a `u64`; a span of 2^64 cannot happen
    // because `max - min + 1` would need `min == i64::MIN` and
    // `max == i64::MAX`, which the wrapping add maps to 0: then every `i64`
    // is allowed.
    let span = max.abs_diff(min).wrapping_add(1);
    if span == 0 {
        return rng.next_u64() as i64;
    }
    min.wrapping_add_unsigned(rng.below(span))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_stays_in_the_half_open_interval() {
        let mut rng = Rng::seeded(1);
        let mut sum = 0.0;
        for _ in 0..100_000 {
            let u = unit(&mut rng);
            assert!((0.0..1.0).contains(&u), "{u} is outside [0, 1)");
            sum += u;
        }
        let mean = sum / 100_000.0;
        assert!((mean - 0.5).abs() < 0.01, "the mean of uniform numbers is {mean}");
    }

    #[test]
    fn chance_handles_the_ends() {
        let mut rng = Rng::seeded(2);
        assert!((0..1_000).all(|_| !chance(&mut rng, 0.0)));
        assert!((0..1_000).all(|_| chance(&mut rng, 1.0)));
        assert!((0..1_000).all(|_| !chance(&mut rng, -3.0)));
        let hits = (0..100_000).filter(|_| chance(&mut rng, 0.25)).count();
        assert!((24_000..26_000).contains(&hits), "{hits} hits of 100000 at 25 %");
    }

    #[test]
    fn int_between_includes_both_ends() {
        let mut rng = Rng::seeded(3);
        let mut seen = [0u32; 5];
        for _ in 0..5_000 {
            let n = int_between(&mut rng, -2, 2);
            seen[usize::try_from(n + 2).expect("the number is from -2 to 2")] += 1;
        }
        assert!(seen.iter().all(|&count| count > 800), "counts: {seen:?}");
        assert_eq!(int_between(&mut rng, 7, 7), 7);
        assert_eq!(int_between(&mut rng, 9, 3), 9);
        // The full range must not panic or loop.
        let _ = int_between(&mut rng, i64::MIN, i64::MAX);
    }

    #[test]
    fn normal_numbers_have_mean_zero_and_variance_one() {
        let mut rng = Rng::seeded(4);
        let n = 200_000;
        let (mut sum, mut squares) = (0.0, 0.0);
        for _ in 0..n {
            let z = standard_normal(&mut rng);
            sum += z;
            squares += z * z;
        }
        let mean = sum / f64::from(n);
        let variance = squares / f64::from(n) - mean * mean;
        assert!(mean.abs() < 0.01, "mean {mean}");
        assert!((variance - 1.0).abs() < 0.02, "variance {variance}");
    }
}
