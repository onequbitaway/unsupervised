//! Statistics: exact percentiles of a list of samples, and a latency
//! histogram for many samples from many tasks.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};

/// Returns the value at quantile `q` (0 to 1) of a sorted list, with the
/// nearest-rank method: the smallest value such that at least `q` of the
/// samples are at or below it. Returns `None` for an empty list.
#[must_use]
pub fn nearest_rank<T: Copy>(sorted: &[T], q: f64) -> Option<T> {
    if sorted.is_empty() {
        return None;
    }
    let rank = (q.clamp(0.0, 1.0) * sorted.len() as f64).ceil() as usize;
    sorted.get(rank.clamp(1, sorted.len()) - 1).copied()
}

/// The summary of a list of durations, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Millis {
    /// Number of samples.
    pub count: u64,
    /// Arithmetic mean.
    pub mean: f64,
    /// Smallest sample.
    pub min: f64,
    /// Median: half of the samples are at or below it.
    pub p50: f64,
    /// 95 % of the samples are at or below it.
    pub p95: f64,
    /// 99 % of the samples are at or below it.
    pub p99: f64,
    /// Largest sample.
    pub max: f64,
}

impl Millis {
    /// Summarizes samples in nanoseconds. The percentiles are exact
    /// (nearest rank). An empty list gives zeros.
    #[must_use]
    pub fn from_nanos(samples: &[u64]) -> Millis {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let ms = |nanos: u64| nanos as f64 / 1e6;
        let at = |q: f64| nearest_rank(&sorted, q).map_or(0.0, ms);
        let total: u128 = sorted.iter().map(|&n| u128::from(n)).sum();
        Millis {
            count: sorted.len() as u64,
            mean: if sorted.is_empty() { 0.0 } else { total as f64 / sorted.len() as f64 / 1e6 },
            min: sorted.first().copied().map_or(0.0, ms),
            p50: at(0.50),
            p95: at(0.95),
            p99: at(0.99),
            max: sorted.last().copied().map_or(0.0, ms),
        }
    }
}

/// A latency histogram that many tasks can write to.
///
/// It keeps counts in buckets with a fixed number of significant digits, so
/// its size does not grow with the number of samples. The buckets are split
/// over several locks; a task picks one by a number of its own, for example
/// the number of the agent.
#[derive(Debug)]
pub struct Latency {
    shards: Vec<Mutex<Histogram<u64>>>,
    max_micros: u64,
}

/// Number of locks of a [`Latency`].
const SHARDS: usize = 64;

impl Latency {
    /// Makes a histogram for latencies up to `max`, with `sigfig`
    /// significant digits (1 to 5). A longer latency counts as `max`.
    ///
    /// # Errors
    ///
    /// Fails when the bounds are not valid for a histogram.
    pub fn new(max: Duration, sigfig: u8) -> Result<Latency, String> {
        let max_micros = u64::try_from(max.as_micros()).unwrap_or(u64::MAX).max(2);
        let mut shards = Vec::with_capacity(SHARDS);
        for _ in 0..SHARDS {
            let histogram = Histogram::new_with_bounds(1, max_micros, sigfig)
                .map_err(|error| format!("cannot make a latency histogram: {error:?}"))?;
            shards.push(Mutex::new(histogram));
        }
        Ok(Latency { shards, max_micros })
    }

    /// Adds one latency. `shard` chooses the lock; any number works.
    pub fn record(&self, shard: usize, latency: Duration) {
        let micros = u64::try_from(latency.as_micros()).unwrap_or(u64::MAX);
        let value = micros.clamp(1, self.max_micros);
        // A poisoned lock means that a task panicked while it recorded. The
        // counts are still valid numbers, so keep recording.
        let mut histogram = match self.shards[shard % SHARDS].lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        histogram.saturating_record(value);
    }

    /// Returns the summary of everything that was recorded.
    #[must_use]
    pub fn summary(&self) -> LatencySummary {
        let mut all: Option<Histogram<u64>> = None;
        for shard in &self.shards {
            let histogram = match shard.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            match &mut all {
                None => all = Some(histogram.clone()),
                // All shards have the same bounds, so the sum cannot fail.
                Some(sum) => {
                    let _ = sum.add(&*histogram);
                }
            }
        }
        let Some(all) = all.filter(|h| !h.is_empty()) else {
            return LatencySummary::default();
        };
        let ms = |micros: u64| micros as f64 / 1e3;
        LatencySummary {
            count: all.len(),
            mean_ms: all.mean() / 1e3,
            min_ms: ms(all.min()),
            p50_ms: ms(all.value_at_quantile(0.50)),
            p95_ms: ms(all.value_at_quantile(0.95)),
            p99_ms: ms(all.value_at_quantile(0.99)),
            p999_ms: ms(all.value_at_quantile(0.999)),
            max_ms: ms(all.max()),
        }
    }
}

/// The summary of a [`Latency`] histogram, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LatencySummary {
    /// Number of samples.
    pub count: u64,
    /// Arithmetic mean.
    pub mean_ms: f64,
    /// Smallest sample.
    pub min_ms: f64,
    /// Median.
    pub p50_ms: f64,
    /// 95th percentile.
    pub p95_ms: f64,
    /// 99th percentile.
    pub p99_ms: f64,
    /// 99.9th percentile.
    pub p999_ms: f64,
    /// Largest sample.
    pub max_ms: f64,
}

/// Counts by name, for example errors by code. Ordered by name.
pub type Counts = BTreeMap<String, u64>;

/// Adds `n` to the count of `name`.
pub fn bump(counts: &mut Counts, name: &str, n: u64) {
    if n == 0 {
        return;
    }
    match counts.get_mut(name) {
        Some(count) => *count += n,
        None => {
            counts.insert(name.to_string(), n);
        }
    }
}

/// Adds every count of `from` to `into`.
pub fn merge(into: &mut Counts, from: &Counts) {
    for (name, &n) in from {
        bump(into, name, n);
    }
}
