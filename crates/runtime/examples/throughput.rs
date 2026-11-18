//! Measures what the runtime can take: actions per second and the time from
//! a call to its acknowledgement, with the journal off and on.
//!
//! Many callers send `chop` actions through one `SimHandle` while the world
//! runs frames in real time. Each caller owns one resident. The example
//! prints, for each setting:
//!
//! - the rate with every caller sending as fast as it gets answers
//!   (saturated), and the latency percentiles at that rate;
//! - the latency percentiles at half of that rate, which is closer to a
//!   server that is not overloaded;
//! - where the simulation thread spent its time.
//!
//! It also measures the kernel alone (no runtime), the effect of the batch
//! size on a journal that syncs, what happens when more callers arrive than
//! the mailbox holds, a snapshot, and the cost of a frame while very many
//! `wait` calls wait.
//!
//! ```text
//! cargo run --release -p unsup-runtime --example throughput
//! cargo run --release -p unsup-runtime --example throughput -- CALLERS SECONDS WAITERS
//! ```
//!
//! The defaults are 4,096 callers, 3 seconds per measurement and 200,000
//! waiting calls.

#[path = "../tests/common/mod.rs"]
mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;
use unsup_kernel::{CityConfig, Input, Plugin, Pos, ResidentId, World, WorldConfig};
use unsup_protocol::{ActionRequest, SimError, WaitFilter};
use unsup_runtime::{
    DurableConfig, DurableJournal, FsStorage, RuntimeConfig, SimHandle, SimStats, SyncPolicy, start,
};

use common::{Town, act, plugins, spec};

/// Real time between two frames while the callers run.
const TICK: Duration = Duration::from_millis(50);

/// Frames between two snapshots in the runs with a journal.
const SNAPSHOT_EVERY: u64 = 20;

/// How long a caller waits before it tries again after `overloaded`.
const BACK_OFF: Duration = Duration::from_millis(1);

/// What one measurement gave.
struct Outcome {
    /// Acknowledged actions.
    acks: u64,
    /// Calls that were refused with `overloaded`.
    overloaded: u64,
    /// Time from each call to its acknowledgement, in microseconds, sorted.
    latencies: Vec<u32>,
    /// Length of the measurement.
    elapsed: Duration,
    /// The statistics of the runtime before and after.
    before: SimStats,
    after: SimStats,
}

impl Outcome {
    fn rate(&self) -> f64 {
        self.acks as f64 / self.elapsed.as_secs_f64()
    }

    /// Returns a latency percentile in milliseconds.
    fn percentile(&self, percent: f64) -> f64 {
        let rank = ((self.latencies.len() as f64) * percent / 100.0).ceil() as usize;
        let micros = self.latencies.get(rank.saturating_sub(1)).copied().unwrap_or(0);
        f64::from(micros) / 1_000.0
    }
}

/// Makes a world of `districts * districts` zones with the town module.
fn world(districts: u32) -> World {
    let config = WorldConfig {
        cities: vec![CityConfig::new("Load", Pos::new(0, 0), districts)],
        ..WorldConfig::with_seed(7)
    };
    World::new(&[&Town::default()], config)
}

fn chop() -> ActionRequest {
    act("chop", json!({}))
}

/// Creates `count` residents and spreads them over the zones.
async fn populate(handle: &SimHandle, count: usize, zones: usize) -> Vec<ResidentId> {
    let mut ids = Vec::with_capacity(count);
    for n in 0..count {
        let id = handle.spawn(spec(&format!("r{n}"))).await.expect("spawn");
        let travel = act("travel", json!({ "zone": n % zones }));
        handle.act(id, travel).await.expect("travel");
        ids.push(id);
    }
    ids
}

/// One caller: sends `chop` until the deadline. With a `period` it sends
/// one action per period; without, the next action follows the answer.
async fn caller(
    handle: SimHandle,
    resident: ResidentId,
    deadline: Instant,
    period: Option<Duration>,
    offset: Duration,
) -> (Vec<u32>, u64) {
    let mut latencies = Vec::new();
    let mut overloaded = 0;
    let mut pace = period.map(|period| {
        let mut pace = tokio::time::interval_at((Instant::now() + offset).into(), period);
        pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        pace
    });
    let end = tokio::time::sleep_until(deadline.into());
    tokio::pin!(end);
    loop {
        if let Some(pace) = &mut pace {
            tokio::select! {
                _ = pace.tick() => {}
                () = &mut end => break,
            }
        }
        let sent = Instant::now();
        if sent >= deadline {
            break;
        }
        let answer = tokio::select! {
            answer = handle.act(resident, chop()) => answer,
            // A call that is still on its way at the end is not counted.
            () = &mut end => break,
        };
        match answer {
            Ok(_) => latencies.push(u32::try_from(sent.elapsed().as_micros()).unwrap_or(u32::MAX)),
            Err(SimError::Overloaded) => {
                overloaded += 1;
                tokio::time::sleep(BACK_OFF).await;
            }
            Err(error) => panic!("the runtime stopped during the measurement: {error}"),
        }
    }
    (latencies, overloaded)
}

/// Runs all callers for `seconds` and collects what they measured.
async fn measure(
    handle: &SimHandle,
    residents: &[ResidentId],
    seconds: f64,
    rate: Option<f64>,
) -> Outcome {
    let before = handle.stats().await.expect("stats");
    let period = rate.map(|rate| Duration::from_secs_f64(residents.len() as f64 / rate));
    let started = Instant::now();
    let deadline = started + Duration::from_secs_f64(seconds);
    let tasks: Vec<_> = residents
        .iter()
        .enumerate()
        .map(|(n, &resident)| {
            // Paced callers do not all send at the same moment.
            let offset = period
                .map_or(Duration::ZERO, |period| period.mul_f64(n as f64 / residents.len() as f64));
            tokio::spawn(caller(handle.clone(), resident, deadline, period, offset))
        })
        .collect();
    let mut latencies = Vec::new();
    let mut overloaded = 0;
    for task in tasks {
        let (mut samples, refused) = task.await.expect("a caller ran");
        latencies.append(&mut samples);
        overloaded += refused;
    }
    let elapsed = started.elapsed();
    let after = handle.stats().await.expect("stats");
    latencies.sort_unstable();
    Outcome { acks: latencies.len() as u64, overloaded, latencies, elapsed, before, after }
}

fn print_header() {
    println!(
        "{:<40} {:>11} {:>8} {:>8} {:>8} {:>9} {:>8} {:>10}   simulation thread",
        "setting", "actions/s", "p50 ms", "p90 ms", "p99 ms", "p99.9 ms", "max ms", "overloaded"
    );
}

fn print_row(label: &str, outcome: &Outcome) {
    let (a, b) = (&outcome.after, &outcome.before);
    let total = outcome.elapsed.as_micros().max(1) as f64;
    let share = |after: u64, before: u64| 100.0 * (after - before) as f64 / total;
    let frames = a.frames_run - b.frames_run;
    let frame_time = (a.mean_frame_micros * a.frames_run)
        .saturating_sub(b.mean_frame_micros * b.frames_run) as f64;
    println!(
        "{label:<40} {:>11.0} {:>8.2} {:>8.2} {:>8.2} {:>9.2} {:>8.2} {:>10}   intake {:.0}%, commit {:.0}%, apply {:.0}%, reply {:.0}%, {frames} frames {:.0}%",
        outcome.rate(),
        outcome.percentile(50.0),
        outcome.percentile(90.0),
        outcome.percentile(99.0),
        outcome.percentile(99.9),
        outcome.percentile(100.0),
        outcome.overloaded,
        share(a.intake_micros, b.intake_micros),
        share(a.commit_micros, b.commit_micros),
        share(a.apply_micros, b.apply_micros),
        share(a.reply_micros, b.reply_micros),
        100.0 * frame_time / total,
    );
}

/// The kernel without a runtime: `threads` worlds, each on its own thread,
/// apply `chop` actions as fast as they can. Returns actions per second.
fn kernel_alone(threads: usize, residents: usize, seconds: f64) -> f64 {
    let deadline = Instant::now() + Duration::from_secs_f64(seconds);
    let workers: Vec<_> = (0..threads)
        .map(|_| {
            std::thread::spawn(move || {
                let mut world = world(4);
                let ids: Vec<ResidentId> = (0..residents)
                    .map(|n| world.spawn(spec(&format!("r{n}"))).expect("spawn"))
                    .collect();
                let started = Instant::now();
                let mut applied = 0u64;
                while Instant::now() < deadline {
                    for &resident in &ids {
                        world.apply(Input::Act { resident, request: chop() });
                    }
                    applied += ids.len() as u64;
                }
                applied as f64 / started.elapsed().as_secs_f64()
            })
        })
        .collect();
    workers.into_iter().map(|worker| worker.join().expect("a kernel thread ran")).sum()
}

/// A directory for the log and the snapshots of one run.
fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("unsup-throughput-{}-{name}", std::process::id()))
}

/// Calls that the simulation thread takes in one batch in the run with
/// large batches. One sync of the log then serves this many inputs.
const LARGE_BATCH: usize = 4_096;

/// Calls per batch in the run with small batches and one sync per input.
const SMALL_BATCH: usize = 4;

/// One setting: saturated, then at half of the saturated rate. `batch` is
/// `RuntimeConfig::batch`; `None` is the default.
async fn run_setting(
    label: &str,
    sync: Option<SyncPolicy>,
    batch: Option<usize>,
    callers: usize,
    seconds: f64,
) {
    let dir = scratch(&label.replace([' ', ','], "-"));
    let (world, journal) = match sync {
        None => (world(4), None),
        Some(sync) => {
            let storage = Arc::new(FsStorage::open(&dir).expect("the scratch directory opens"));
            let durable =
                DurableConfig { sync, snapshot_every: SNAPSHOT_EVERY, ..DurableConfig::default() };
            let opened = DurableJournal::open(storage, plugins(), durable, |_| world(4))
                .expect("the journal opens");
            (opened.world, Some(opened.journal))
        }
    };
    let zones = world.layout().zones().len();
    let mut config = RuntimeConfig::real_time(TICK);
    if let Some(batch) = batch {
        config.batch = batch;
    }
    let durable = journal.is_some();
    if let Some(journal) = journal {
        config.journal = Box::new(journal);
    }
    let (handle, join) = start(world, config);
    let residents = populate(&handle, callers, zones).await;

    let saturated = measure(&handle, &residents, seconds, None).await;
    print_row(&format!("{label}, saturated"), &saturated);
    let half = measure(&handle, &residents, seconds, Some(saturated.rate() / 2.0)).await;
    print_row(&format!("{label}, half load"), &half);
    if durable {
        let stats = handle.stats().await.expect("stats");
        println!(
            "{:<40} {} snapshots; the last one: {:.1} KiB, the simulation thread stood still for {:.2} ms, the write took {:.2} ms; log lag now {} inputs",
            "",
            stats.snapshots,
            stats.last_snapshot_bytes as f64 / 1024.0,
            stats.last_snapshot_micros as f64 / 1_000.0,
            stats.last_snapshot_write_micros as f64 / 1_000.0,
            stats.journal_lag,
        );
    }
    let world = join.shutdown().expect("a clean stop");
    world.audit().expect("the ledgers balance");
    if durable {
        std::fs::remove_dir_all(&dir).expect("the scratch directory can be removed");
    }
}

/// More callers than the mailbox holds: the runtime refuses the extra
/// calls, the callers back off, and the rate stays up.
async fn run_overload(seconds: f64) {
    let config = RuntimeConfig { mailbox: 1_024, ..RuntimeConfig::real_time(TICK) };
    let callers = 4 * config.mailbox;
    let (handle, join) = start(world(4), config);
    let residents = populate(&handle, callers, 16).await;
    let outcome = measure(&handle, &residents, seconds, None).await;
    print_row(&format!("{callers} callers, mailbox 1024"), &outcome);
    join.shutdown().expect("a clean stop");
}

/// Sends a call again until the runtime takes it.
async fn insist<T, F: Future<Output = Result<T, SimError>>>(mut call: impl FnMut() -> F) -> T {
    loop {
        match call().await {
            Ok(value) => return value,
            Err(SimError::Overloaded) => tokio::time::sleep(BACK_OFF).await,
            Err(error) => panic!("the runtime stopped: {error}"),
        }
    }
}

/// Frames in one measurement of the frame cost.
const FRAMES: u32 = 200;

/// What a frame costs in milliseconds: the median time of the world in a
/// frame, and the mean time of the simulation thread per frame. The second
/// number holds everything that the runtime does around the world too: the
/// journal, the pacer, the waiting calls and the broadcast.
async fn frame_cost(handle: &SimHandle) -> (f64, f64) {
    let started = Instant::now();
    handle.step(FRAMES).await.expect("step");
    let whole = started.elapsed().as_secs_f64() * 1_000.0 / f64::from(FRAMES);
    let world = handle.stats().await.expect("stats").p50_frame_micros as f64 / 1_000.0;
    (world, whole)
}

/// The cost of a frame while very many calls wait, and of waking some.
async fn run_waiters(waiters: usize) {
    let config = RuntimeConfig { stats_window: FRAMES as usize, ..RuntimeConfig::manual() };
    let (handle, join) = start(world(16), config);
    let started = Instant::now();
    let mut residents = Vec::with_capacity(waiters);
    for n in 0..waiters {
        let id = insist(|| handle.spawn(spec(&format!("w{n}")))).await;
        insist(|| handle.act(id, act("travel", json!({ "zone": n % 256 })))).await;
        residents.push(id);
    }
    println!(
        "wait: {waiters} residents in 256 zones, made in {:.1} s",
        started.elapsed().as_secs_f64()
    );
    let (idle_world, idle_whole) = frame_cost(&handle).await;

    let started = Instant::now();
    let calls: Vec<_> = residents
        .iter()
        .map(|&resident| {
            let handle = handle.clone();
            tokio::spawn(async move {
                insist(|| handle.wait(resident, WaitFilter::Any, Duration::from_secs(600))).await
            })
        })
        .collect();
    // The statistics call competes with the flood for the mailbox.
    while (insist(|| handle.stats()).await.waiting as usize) < waiters {
        tokio::time::sleep(BACK_OFF).await;
    }
    let registered = started.elapsed();
    let (loaded_world, loaded_whole) = frame_cost(&handle).await;
    println!("wait: {waiters} calls wait (registered in {:.2} s)", registered.as_secs_f64());
    println!(
        "wait: a frame without them: {idle_whole:.3} ms in all, {idle_world:.3} ms of it in the world; with them: {loaded_whole:.3} ms in all, {loaded_world:.3} ms in the world",
    );

    // One resident in a hundred gets a notice. The frame wakes its call.
    let chosen: Vec<ResidentId> = residents.iter().copied().step_by(100).collect();
    for pair in chosen.windows(2) {
        let tell = act("tell", json!({ "to": pair[1], "kind": "town.greeting" }));
        handle.act(pair[0], tell).await.expect("tell");
    }
    let started = Instant::now();
    handle.step(1).await.expect("step");
    let woke = started.elapsed();
    let stats = handle.stats().await.expect("stats");
    println!(
        "wait: a frame that wakes {} calls takes {:.3} ms in all; {} calls still wait",
        waiters as u64 - stats.waiting,
        woke.as_secs_f64() * 1_000.0,
        stats.waiting,
    );
    for call in calls {
        call.abort();
    }
    join.shutdown().expect("a clean stop");
}

/// Reads a number from the command line, or stops with the usage text.
fn argument(position: usize, default: f64) -> f64 {
    match std::env::args().nth(position) {
        Some(arg) => arg.replace('_', "").parse().unwrap_or_else(|_| {
            eprintln!(
                "usage: throughput [CALLERS] [SECONDS] [WAITERS]   (defaults: 4096, 3 and 200000)"
            );
            std::process::exit(2);
        }),
        None => default,
    }
}

fn main() {
    let callers = (argument(1, 4_096.0) as usize).max(1);
    let seconds = argument(2, 3.0).max(0.1);
    let waiters = argument(3, 200_000.0) as usize;
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let plugin: &dyn Plugin = &Town::default();
    println!(
        "throughput: {callers} callers, each with one resident, in 16 zones; {seconds} s per measurement; a frame every {} ms; {threads} processors; module `{}`",
        TICK.as_millis(),
        plugin.name(),
    );

    let one = kernel_alone(1, callers, seconds.min(2.0));
    let many = kernel_alone(threads.min(8), callers, seconds.min(2.0));
    println!(
        "kernel alone: {one:.0} actions/s on one thread ({:.2} us per action); {many:.0} actions/s with {} worlds on {} threads",
        1_000_000.0 / one,
        threads.min(8),
        threads.min(8),
    );

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime starts");
    runtime.block_on(async {
        print_header();
        run_setting("journal off", None, None, callers, seconds).await;
        let interval = SyncPolicy::Interval(Duration::from_millis(50));
        run_setting("journal, sync every 50 ms", Some(interval), None, callers, seconds).await;
        let every_batch = Some(SyncPolicy::EveryBatch);
        run_setting("journal, sync every batch", every_batch, None, callers, seconds).await;
        // One sync serves a whole batch, so a larger batch takes more
        // inputs per sync.
        let label = format!("sync every batch of {LARGE_BATCH}");
        run_setting(&label, every_batch, Some(LARGE_BATCH), callers, seconds).await;
        // One sync per input is slow. A few callers are enough to show it.
        // The thread looks for a due frame between two batches, so a small
        // batch keeps the frames on time.
        let few = callers.min(64);
        let every_record = Some(SyncPolicy::EveryRecord);
        let label = format!("sync every record, {few} callers");
        run_setting(&label, every_record, None, few, seconds).await;
        let label = format!("every record, batch of {SMALL_BATCH}");
        run_setting(&label, every_record, Some(SMALL_BATCH), few, seconds).await;
        run_overload(seconds).await;
        if waiters > 0 {
            run_waiters(waiters).await;
        }
    });
}
