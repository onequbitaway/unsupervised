//! The pseudo-random number generator of a realm.
//!
//! The generator is xoshiro256** seeded with splitmix64. It is written here,
//! with integers only, so that the output never depends on the version of a
//! third-party crate. Its state is part of the snapshot.

use serde::{Deserialize, Serialize};

/// One step of splitmix64. It advances `state` and returns the next output.
#[inline]
fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// A deterministic random number generator (xoshiro256**).
///
/// Randomness in the simulation comes only from `realm.rng()`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rng {
    s: [u64; 4],
}

impl Rng {
    /// Makes a generator from a seed.
    #[must_use]
    pub fn seeded(seed: u64) -> Rng {
        let mut state = seed;
        let mut s = [0u64; 4];
        for word in &mut s {
            *word = splitmix64(&mut state);
        }
        // xoshiro must not start from the all-zero state. splitmix64 cannot
        // produce four zeros in a row, but the check costs nothing.
        if s == [0; 4] {
            s[0] = 0x9e37_79b9_7f4a_7c15;
        }
        Rng { s }
    }

    /// Makes the generator of stream `stream` for a world seed. Different
    /// streams of one seed are independent; each realm has its own stream.
    #[must_use]
    pub fn stream(seed: u64, stream: u64) -> Rng {
        let mut state = seed;
        let a = splitmix64(&mut state);
        let mut state = stream ^ 0x6a09_e667_f3bc_c908;
        let b = splitmix64(&mut state);
        Rng::seeded(a ^ b.rotate_left(32))
    }

    /// Returns the next 64 random bits.
    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        let result = self.s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = self.s[1] << 17;
        self.s[2] ^= self.s[0];
        self.s[3] ^= self.s[1];
        self.s[1] ^= self.s[2];
        self.s[0] ^= self.s[3];
        self.s[2] ^= t;
        self.s[3] = self.s[3].rotate_left(45);
        result
    }

    /// Returns the next 32 random bits.
    #[inline]
    pub fn next_u32(&mut self) -> u32 {
        (self.next_u64() >> 32) as u32
    }

    /// Returns a uniform number in `0..n`. Returns 0 when `n` is 0.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        // Lemire's method: multiply, and reject the few values that would
        // make the result biased.
        let mut m = u128::from(self.next_u64()) * u128::from(n);
        if (m as u64) < n {
            let threshold = n.wrapping_neg() % n;
            while (m as u64) < threshold {
                m = u128::from(self.next_u64()) * u128::from(n);
            }
        }
        (m >> 64) as u64
    }

    /// Returns a uniform number in `lo..hi`. Returns `lo` when the range is
    /// empty.
    pub fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo {
            return lo;
        }
        let span = hi.abs_diff(lo);
        lo.wrapping_add_unsigned(self.below(span))
    }

    /// Returns true with probability `num / den`. Returns false when `den`
    /// is 0.
    pub fn chance(&mut self, num: u64, den: u64) -> bool {
        den != 0 && self.below(den) < num
    }

    /// Returns true with a probability given in basis points (1/10,000).
    pub fn chance_bp(&mut self, basis_points: u32) -> bool {
        self.chance(u64::from(basis_points), 10_000)
    }

    /// Returns a uniform element of the slice, or `None` when it is empty.
    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() { None } else { items.get(self.below(items.len() as u64) as usize) }
    }

    /// Shuffles the slice in place (Fisher-Yates).
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = self.below(i as u64 + 1) as usize;
            items.swap(i, j);
        }
    }

    /// Returns a new generator seeded from this one. Use it to give a
    /// sub-task its own stream.
    pub fn fork(&mut self) -> Rng {
        Rng::seeded(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_the_reference() {
        let mut state = 1_234_567u64;
        let got: Vec<u64> = (0..5).map(|_| splitmix64(&mut state)).collect();
        assert_eq!(
            got,
            [
                6_457_827_717_110_365_317,
                3_203_168_211_198_807_973,
                9_817_491_932_198_370_423,
                4_593_380_528_125_082_431,
                16_408_922_859_458_223_821,
            ]
        );
    }

    #[test]
    fn xoshiro_matches_the_reference() {
        let mut rng = Rng { s: [1, 2, 3, 4] };
        let got: Vec<u64> = (0..10).map(|_| rng.next_u64()).collect();
        assert_eq!(
            got,
            [
                11_520,
                0,
                1_509_978_240,
                1_215_971_899_390_074_240,
                1_216_172_134_540_287_360,
                607_988_272_756_665_600,
                16_172_922_978_634_559_625,
                8_476_171_486_693_032_832,
                10_595_114_339_597_558_777,
                2_904_607_092_377_533_576,
            ]
        );
    }

    #[test]
    fn same_seed_same_sequence_and_streams_differ() {
        let mut a = Rng::seeded(42);
        let mut b = Rng::seeded(42);
        let mut c = Rng::stream(42, 1);
        let mut d = Rng::stream(42, 2);
        let xs: Vec<u64> = (0..8).map(|_| a.next_u64()).collect();
        assert_eq!(xs, (0..8).map(|_| b.next_u64()).collect::<Vec<_>>());
        let cs: Vec<u64> = (0..8).map(|_| c.next_u64()).collect();
        let ds: Vec<u64> = (0..8).map(|_| d.next_u64()).collect();
        assert_ne!(cs, ds);
        assert_ne!(cs, xs);
    }

    #[test]
    fn bounded_draws_stay_in_range_and_cover_it() {
        let mut rng = Rng::seeded(7);
        let mut seen = [false; 6];
        for _ in 0..2_000 {
            let v = rng.below(6);
            assert!(v < 6);
            seen[v as usize] = true;
            let r = rng.range(-3, 4);
            assert!((-3..4).contains(&r));
        }
        assert_eq!(seen, [true; 6]);
        assert_eq!(rng.below(0), 0);
        assert_eq!(rng.range(5, 5), 5);
        assert_eq!(rng.range(5, 2), 5);
        assert_eq!(rng.range(i64::MIN, i64::MIN + 1), i64::MIN);
    }

    #[test]
    fn chance_respects_the_extremes() {
        let mut rng = Rng::seeded(9);
        for _ in 0..200 {
            assert!(!rng.chance(0, 10));
            assert!(rng.chance(10, 10));
            assert!(!rng.chance(1, 0));
            assert!(!rng.chance_bp(0));
            assert!(rng.chance_bp(10_000));
        }
        let hits = (0..10_000).filter(|_| rng.chance_bp(2_500)).count();
        assert!((2_200..2_800).contains(&hits), "{hits}");
    }

    #[test]
    fn pick_and_shuffle() {
        let mut rng = Rng::seeded(3);
        assert_eq!(rng.pick::<u8>(&[]), None);
        assert_eq!(rng.pick(&[5]), Some(&5));
        let mut items: Vec<u32> = (0..50).collect();
        rng.shuffle(&mut items);
        assert_ne!(items, (0..50).collect::<Vec<_>>());
        items.sort_unstable();
        assert_eq!(items, (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn state_survives_serialization() {
        let mut rng = Rng::seeded(11);
        rng.next_u64();
        let bytes = rmp_serde::to_vec_named(&rng).unwrap();
        let mut back: Rng = rmp_serde::from_slice(&bytes).unwrap();
        assert_eq!(back.next_u64(), rng.next_u64());
        assert_ne!(rng.fork().next_u64(), rng.next_u64());
    }
}
