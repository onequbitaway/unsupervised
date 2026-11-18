//! Statistics on known data: percentiles, the latency histogram, and the
//! think time distributions.

use std::sync::Arc;
use std::time::Duration;

use unsup_kernel::Rng;
use unsup_loadtest::stats::{Latency, Millis, nearest_rank};
use unsup_loadtest::think::{ThinkKind, ThinkTime, TickThink};

#[test]
fn nearest_rank_percentiles_of_one_to_one_hundred() {
    let data: Vec<u32> = (1..=100).collect();
    assert_eq!(nearest_rank(&data, 0.50), Some(50));
    assert_eq!(nearest_rank(&data, 0.95), Some(95));
    assert_eq!(nearest_rank(&data, 0.99), Some(99));
    assert_eq!(nearest_rank(&data, 1.0), Some(100));
    assert_eq!(nearest_rank(&data, 0.0), Some(1));
    assert_eq!(nearest_rank(&data, 0.001), Some(1));
    assert_eq!(nearest_rank(&data, 0.991), Some(100));
    // Out of range quantiles are cut.
    assert_eq!(nearest_rank(&data, 7.0), Some(100));
    assert_eq!(nearest_rank(&data, -1.0), Some(1));
}

#[test]
fn nearest_rank_on_short_lists() {
    let four = [10, 20, 30, 40];
    assert_eq!(nearest_rank(&four, 0.50), Some(20));
    assert_eq!(nearest_rank(&four, 0.75), Some(30));
    assert_eq!(nearest_rank(&four, 0.76), Some(40));
    assert_eq!(nearest_rank(&four, 0.99), Some(40));
    assert_eq!(nearest_rank(&[7], 0.5), Some(7));
    assert_eq!(nearest_rank::<u64>(&[], 0.5), None);
}

#[test]
fn the_summary_of_frame_times_is_exact() {
    // 1 ms to 100 ms, not in order.
    let mut nanos: Vec<u64> = (1..=100u64).map(|ms| ms * 1_000_000).collect();
    Rng::seeded(3).shuffle(&mut nanos);
    let summary = Millis::from_nanos(&nanos);
    assert_eq!(summary.count, 100);
    assert!((summary.mean - 50.5).abs() < 1e-9, "mean {}", summary.mean);
    assert!((summary.min - 1.0).abs() < 1e-9);
    assert!((summary.p50 - 50.0).abs() < 1e-9);
    assert!((summary.p95 - 95.0).abs() < 1e-9);
    assert!((summary.p99 - 99.0).abs() < 1e-9);
    assert!((summary.max - 100.0).abs() < 1e-9);

    // One slow frame among 199 fast ones moves the worst value and not the
    // 99th percentile.
    let mut spiky = vec![2_000_000u64; 199];
    spiky.push(900_000_000);
    let summary = Millis::from_nanos(&spiky);
    assert!((summary.p99 - 2.0).abs() < 1e-9, "p99 {}", summary.p99);
    assert!((summary.max - 900.0).abs() < 1e-9);
    assert!((summary.mean - (199.0 * 2.0 + 900.0) / 200.0).abs() < 1e-9);

    assert_eq!(Millis::from_nanos(&[]), Millis::default());
}

#[test]
fn the_latency_histogram_gives_percentiles_within_its_precision() {
    let latency = Latency::new(Duration::from_secs(60), 3).expect("valid bounds");
    // 1 ms to 1,000 ms, written by 8 threads to different locks.
    let latency = Arc::new(latency);
    let threads: Vec<_> = (0..8u64)
        .map(|thread| {
            let latency = Arc::clone(&latency);
            std::thread::spawn(move || {
                for ms in (1..=1_000u64).filter(|ms| ms % 8 == thread) {
                    latency.record(thread as usize * 13, Duration::from_millis(ms));
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().expect("the thread ends");
    }
    let summary = latency.summary();
    assert_eq!(summary.count, 1_000);
    // Three significant digits: each value is within 0.1 % of the truth.
    let near = |got: f64, want: f64| (got - want).abs() <= want * 0.001 + 1e-9;
    assert!(near(summary.p50_ms, 500.0), "p50 {}", summary.p50_ms);
    assert!(near(summary.p95_ms, 950.0), "p95 {}", summary.p95_ms);
    assert!(near(summary.p99_ms, 990.0), "p99 {}", summary.p99_ms);
    assert!(near(summary.p999_ms, 999.0), "p99.9 {}", summary.p999_ms);
    assert!(near(summary.max_ms, 1_000.0), "max {}", summary.max_ms);
    assert!(near(summary.min_ms, 1.0), "min {}", summary.min_ms);
    assert!(near(summary.mean_ms, 500.5), "mean {}", summary.mean_ms);
}

#[test]
fn the_latency_histogram_handles_the_ends() {
    let latency = Latency::new(Duration::from_secs(1), 3).expect("valid bounds");
    assert_eq!(latency.summary().count, 0, "an empty histogram has no samples");
    assert!(latency.summary().p99_ms.abs() < f64::EPSILON);
    // A latency above the greatest value counts as the greatest value, and
    // a latency of zero counts as the smallest (one microsecond).
    latency.record(0, Duration::from_secs(50));
    latency.record(1, Duration::ZERO);
    let summary = latency.summary();
    assert_eq!(summary.count, 2);
    assert!((summary.max_ms - 1_000.0).abs() <= 1.0, "max {}", summary.max_ms);
    assert!((summary.min_ms - 0.001).abs() < 1e-9, "min {}", summary.min_ms);
}

fn mean_and_median(think: &ThinkTime, samples: usize) -> (f64, f64) {
    let mut rng = Rng::seeded(21);
    let mut values: Vec<f64> = (0..samples).map(|_| think.sample_secs(&mut rng)).collect();
    assert!(values.iter().all(|&v| v >= 0.0 && v.is_finite()), "think times are not negative");
    values.sort_by(f64::total_cmp);
    (values.iter().sum::<f64>() / samples as f64, values[samples / 2])
}

#[test]
fn think_time_in_seconds_has_the_asked_mean() {
    let constant = ThinkTime::new(ThinkKind::Constant, 2.5, 9.0).expect("valid");
    let mut rng = Rng::seeded(1);
    assert!((0..100).all(|_| (constant.sample_secs(&mut rng) - 2.5).abs() < f64::EPSILON));
    assert_eq!(constant.sample(&mut rng), Duration::from_millis(2_500));

    // Exponential: the mean is 4, the median is 4 * ln 2.
    let exponential = ThinkTime::new(ThinkKind::Exponential, 4.0, 0.0).expect("valid");
    let (mean, median) = mean_and_median(&exponential, 200_000);
    assert!((mean - 4.0).abs() < 0.08, "exponential mean {mean}");
    assert!((median - 4.0 * std::f64::consts::LN_2).abs() < 0.08, "exponential median {median}");

    // Log-normal with sigma 1: the mean is 5, the median is 5 / sqrt(e).
    let lognormal = ThinkTime::new(ThinkKind::Lognormal, 5.0, 1.0).expect("valid");
    let (mean, median) = mean_and_median(&lognormal, 200_000);
    assert!((mean - 5.0).abs() < 0.15, "log-normal mean {mean}");
    assert!((median - 5.0 / 1f64.exp().sqrt()).abs() < 0.1, "log-normal median {median}");

    // A mean of zero means no think time.
    let none = ThinkTime::new(ThinkKind::Lognormal, 0.0, 1.0).expect("valid");
    assert_eq!(none.sample(&mut rng), Duration::ZERO);

    assert!(ThinkTime::new(ThinkKind::Constant, -1.0, 0.0).is_err());
    assert!(ThinkTime::new(ThinkKind::Lognormal, 1.0, f64::NAN).is_err());
    assert_eq!("log-normal".parse::<ThinkKind>(), Ok(ThinkKind::Lognormal));
    assert_eq!("Exponential".parse::<ThinkKind>(), Ok(ThinkKind::Exponential));
    assert!("gaussian".parse::<ThinkKind>().is_err());
}

#[test]
fn think_time_in_ticks_stays_in_its_range() {
    let mut rng = Rng::seeded(2);
    let constant = TickThink::Constant { ticks: 7 };
    assert!((0..50).all(|_| constant.sample(&mut rng) == 7));

    let uniform = TickThink::Uniform { min: 2, max: 5 };
    let mut seen = [0u32; 8];
    for _ in 0..4_000 {
        seen[uniform.sample(&mut rng) as usize] += 1;
    }
    assert_eq!(seen[0] + seen[1] + seen[6] + seen[7], 0, "nothing outside 2..=5: {seen:?}");
    assert!(seen[2..=5].iter().all(|&count| count > 800), "all of 2..=5 are used: {seen:?}");

    let exponential = TickThink::Exponential { mean: 6.0, max: 20 };
    let samples: Vec<u32> = (0..50_000).map(|_| exponential.sample(&mut rng)).collect();
    assert!(samples.iter().all(|&ticks| ticks <= 20), "the cut at 20 holds");
    assert_eq!(samples.iter().copied().max(), Some(20), "the cut is reached");
    let mean = f64::from(samples.iter().sum::<u32>()) / samples.len() as f64;
    // The cut at 20 takes a little from the mean of 6.
    assert!((5.5..6.1).contains(&mean), "mean {mean}");
}
