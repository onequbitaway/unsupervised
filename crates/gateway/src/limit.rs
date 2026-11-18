//! Rate limits: one token bucket for each key.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Time between two sweeps that remove the buckets of quiet keys.
const SWEEP_EVERY: Duration = Duration::from_secs(60);

/// A set of token buckets, one for each key.
///
/// A bucket holds at most `burst` tokens and gets `per_period` tokens in
/// each period, spread evenly. A request takes one token. A key that the
/// limiter has not seen has a full bucket.
#[derive(Debug)]
pub struct RateLimiter<K> {
    /// Tokens that a bucket gets in one second. Zero: no limit.
    per_second: f64,
    burst: f64,
    state: Mutex<State<K>>,
}

#[derive(Debug)]
struct State<K> {
    buckets: HashMap<K, Bucket>,
    swept: Option<Instant>,
}

#[derive(Clone, Copy, Debug)]
struct Bucket {
    tokens: f64,
    updated: Instant,
}

impl<K: Eq + Hash> RateLimiter<K> {
    /// Makes a limiter that lets `per_period` requests of one key through
    /// in each `period`, and `burst` requests at once after a quiet time.
    /// A `per_period` of 0 turns the limit off. A `burst` of 0 counts as 1.
    #[must_use]
    pub fn new(per_period: u32, period: Duration, burst: u32) -> RateLimiter<K> {
        let seconds = period.as_secs_f64();
        RateLimiter {
            per_second: if seconds > 0.0 { f64::from(per_period) / seconds } else { 0.0 },
            burst: f64::from(burst.max(1)),
            state: Mutex::new(State { buckets: HashMap::new(), swept: None }),
        }
    }

    /// Takes one token of `key` at the time `now`.
    ///
    /// # Errors
    ///
    /// When the bucket is empty: the time until it has one token again.
    pub fn check(&self, key: K, now: Instant) -> Result<(), Duration> {
        if self.per_second <= 0.0 {
            return Ok(());
        }
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.swept.is_none_or(|at| now.saturating_duration_since(at) >= SWEEP_EVERY) {
            // A bucket that is full again says the same as no bucket.
            let (rate, burst) = (self.per_second, self.burst);
            state.buckets.retain(|_, bucket| bucket.level(now, rate, burst) < burst);
            state.swept = Some(now);
        }
        let bucket =
            state.buckets.entry(key).or_insert(Bucket { tokens: self.burst, updated: now });
        bucket.tokens = bucket.level(now, self.per_second, self.burst);
        bucket.updated = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - bucket.tokens) / self.per_second))
        }
    }

    /// Returns the number of keys that have a bucket now.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.state.lock().unwrap_or_else(PoisonError::into_inner).buckets.len()
    }
}

impl Bucket {
    /// Returns the tokens in the bucket at the time `now`.
    fn level(&self, now: Instant, per_second: f64, burst: f64) -> f64 {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        (self.tokens + elapsed * per_second).min(burst)
    }
}

/// Returns the address that rate limits count for a client. An IPv6
/// client controls a whole /64 network, so the limit counts the network.
/// An IPv4 address inside an IPv6 address counts as the IPv4 address.
#[must_use]
pub fn limit_address(address: IpAddr) -> IpAddr {
    match address {
        IpAddr::V4(_) => address,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let [a, b, c, d, ..] = v6.segments();
                IpAddr::V6(Ipv6Addr::new(a, b, c, d, 0, 0, 0, 0))
            }
        },
    }
}

/// Returns the seconds to put in `Retry-After` for a wait: the wait
/// rounded up, and at least 1.
#[must_use]
pub fn retry_after_secs(wait: Duration) -> u64 {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    seconds.max(1)
}
