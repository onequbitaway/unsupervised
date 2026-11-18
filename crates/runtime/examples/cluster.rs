//! Runs one world in one process and then in a cluster of processes on
//! this machine, with the same inputs, and compares the two.
//!
//! The load is a crowd that walks: every resident walks to a random tile
//! of the world again and again, one tile per tick, and crosses the zone
//! borders on the way. Some put money in the bank of the global realm.
//! The example starts a coordinator in this process and each worker as a
//! child process, connected over TCP on this machine.
//!
//! It prints the frame time and the input rate of both runs, the time of
//! each phase of a cluster frame, and whether every realm ends in the same
//! state.
//!
//! ```text
//! cargo run --release -p unsup-runtime --example cluster
//! cargo run --release -p unsup-runtime --example cluster -- WORKERS RESIDENTS FRAMES DISTRICTS ZONE_SIZE
//! cargo run --release -p unsup-runtime --example cluster -- crash
//! ```
//!
//! The defaults are 3 workers, 2,000 residents, 300 frames and a city of
//! 3 by 3 zones of 32 by 32 tiles. `crash` shows what happens when a
//! worker process is killed.

#[path = "../tests/common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::process::{Child, Command, ExitCode};
use std::time::{Duration, Instant};

use serde_json::json;
use unsup_kernel::{Plugin, RealmId, World, WorldConfig};
use unsup_protocol::{ObserveOpts, ResidentId, SimError};
use unsup_runtime::cluster::{
    ClusterConfig, ClusterStats, ClusterTotals, Coordinator, WorkerConfig, run_worker,
};
use unsup_runtime::{RuntimeConfig, SimHandle, start};

use common::cluster::{cluster_digests, world_digests};
use common::roads::{plugins, world_config};
use common::script::Dice;
use common::{act, spec};

/// Seed of the world and of the load.
const SEED: u64 = 7;

/// One resident in this many puts money in the bank when it gets a new
/// walk.
const SAVERS: u64 = 5;

/// The size of the run.
#[derive(Clone, Copy, Debug)]
struct Size {
    workers: usize,
    residents: u64,
    frames: u64,
    districts: u32,
    zone_size: u32,
}

impl Size {
    fn world(&self) -> WorldConfig {
        world_config(SEED, self.districts, self.zone_size)
    }

    /// Tiles per side of the world.
    fn side(&self) -> u64 {
        u64::from(self.districts) * u64::from(self.zone_size)
    }

    /// Frames between two walks of one resident: about the time of one
    /// walk, so that a resident walks nearly all the time.
    fn period(&self) -> u64 {
        (self.side() / 2).max(1)
    }
}

/// What one run of the load measured.
struct Measured {
    /// Time of each frame in microseconds.
    frames: Vec<u64>,
    /// Actions that the world took.
    actions: u64,
    /// Time of all action calls.
    acting: Duration,
    /// Time to create the residents.
    spawning: Duration,
    /// Frames until every resident stood still, after the load.
    rest: u64,
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

/// Returns the value at a percentile of sorted numbers.
fn percentile(sorted: &[u64], percent: usize) -> u64 {
    let rank = (sorted.len() * percent).div_ceil(100);
    sorted.get(rank.saturating_sub(1)).copied().unwrap_or(0)
}

/// Sends one action. A request that the world refuses (a full queue) is
/// part of the load; a runtime that does not take the call is not.
async fn send(handle: &SimHandle, resident: ResidentId, verb: &str, args: serde_json::Value) {
    match handle.act(resident, act(verb, args)).await {
        Ok(_) => {}
        Err(error) => panic!("the runtime did not take an action: {error}"),
    }
}

/// Runs the load: the same calls for one process and for a cluster.
async fn load(handle: &SimHandle, size: Size) -> Measured {
    let start = Instant::now();
    for n in 0..size.residents {
        handle.spawn(spec(&format!("R{n}"))).await.expect("spawn");
    }
    let spawning = start.elapsed();

    let mut dice = Dice::new(SEED);
    let (side, period) = (size.side(), size.period());
    let mut frames = Vec::with_capacity(size.frames as usize);
    let (mut actions, mut acting) = (0, Duration::ZERO);
    for frame in 0..size.frames {
        // Each resident gets a new walk every `period` frames. The
        // residents take turns, so every frame has its share.
        let start = Instant::now();
        for n in (0..size.residents).filter(|n| (n + frame) % period == 0) {
            let resident = ResidentId(n + 1);
            let target = json!({ "x": dice.below(side), "y": dice.below(side) });
            send(handle, resident, "walk", target).await;
            actions += 1;
            if n % SAVERS == 0 {
                send(handle, resident, "deposit", json!({ "amount": 1 })).await;
                actions += 1;
            }
        }
        acting += start.elapsed();
        let start = Instant::now();
        handle.step(1).await.expect("step");
        frames.push(micros(start.elapsed()));
    }

    // The residents finish their walks. Then three more frames, so that
    // every message has arrived and no realm holds one.
    let mut rest = 0;
    loop {
        handle.step(8).await.expect("step");
        rest += 8;
        let mut busy = false;
        for n in 0..size.residents {
            let opts =
                ObserveOpts { sections: Some(vec!["self".into()]), ..ObserveOpts::default() };
            let seen = handle.observe(ResidentId(n + 1), opts).await.expect("observe");
            let me = &seen.sections["self"];
            if !me["activity"].is_null() || me["queue"] != json!(0) {
                busy = true;
                break;
            }
        }
        if !busy {
            break;
        }
    }
    handle.step(3).await.expect("step");
    Measured { frames, actions, acting, spawning, rest: rest + 3 }
}

/// The world in one process, with the runtime of one process.
async fn one_process(size: Size) -> (Measured, World) {
    let plugins = plugins();
    let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    let world = World::new(&refs, size.world());
    let (handle, join) = start(world, RuntimeConfig::manual());
    let measured = load(&handle, size).await;
    (measured, join.shutdown().expect("a clean stop"))
}

/// Starts the worker processes: this program again, as `worker`.
fn start_workers(size: Size, coordinator: SocketAddr) -> Vec<Child> {
    let program = std::env::current_exe().expect("the path of this program");
    (0..size.workers)
        .map(|slot| {
            Command::new(&program)
                .args(["worker", &coordinator.to_string(), &slot.to_string()])
                .args([size.districts.to_string(), size.zone_size.to_string()])
                .spawn()
                .expect("a worker process starts")
        })
        .collect()
}

/// What the cluster run gave, besides the times.
struct ClusterEnd {
    stats: ClusterStats,
    states: BTreeMap<RealmId, String>,
    totals: ClusterTotals,
}

/// The world in a cluster: the coordinator here, each worker a process.
async fn cluster(size: Size) -> (Measured, ClusterEnd) {
    let coordinator = Coordinator::bind("127.0.0.1:0").expect("a free port");
    let address = coordinator.local_addr().expect("the address");
    let mut workers = start_workers(size, address);
    let config = ClusterConfig::manual(size.world(), size.workers);
    let (handle, join) = coordinator.start(plugins(), config).expect("the cluster starts");
    let measured = load(&handle, size).await;
    let end = ClusterEnd {
        stats: handle.cluster_stats().await.expect("stats"),
        states: cluster_digests(&handle).await,
        totals: handle.audit().await.expect("the union of all processes balances"),
    };
    drop(handle);
    join.shutdown().expect("the cluster stops in order");
    for worker in &mut workers {
        let status = worker.wait().expect("the worker process ends");
        assert!(status.success(), "a worker ended with {status}");
    }
    (measured, end)
}

fn print_row(name: &str, size: Size, measured: &Measured) {
    let mut sorted = measured.frames.clone();
    sorted.sort_unstable();
    let total: u64 = sorted.iter().sum();
    let mean = total / sorted.len().max(1) as u64;
    let per_second = |count: u64, time: Duration| count as f64 / time.as_secs_f64().max(1e-9);
    println!(
        "{name:<14} {mean:>9} {:>9} {:>9} {:>9} {:>11.0} {:>11.0} {:>11.0}",
        percentile(&sorted, 50),
        percentile(&sorted, 99),
        sorted.last().copied().unwrap_or(0),
        per_second(sorted.len() as u64, Duration::from_micros(total)),
        per_second(measured.actions, measured.acting),
        per_second(size.residents, measured.spawning),
    );
}

async fn compare(size: Size) -> bool {
    let processors = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!(
        "cluster: {} residents walk for {} frames in {} zones of {}x{} tiles; a new walk every {} \
         frames; {} workers; {processors} processors",
        size.residents,
        size.frames,
        size.districts * size.districts,
        size.zone_size,
        size.zone_size,
        size.period(),
        size.workers,
    );
    let (single, world) = one_process(size).await;
    let (multi, end) = cluster(size).await;

    println!();
    println!(
        "{:<14} {:>9} {:>9} {:>9} {:>9} {:>11} {:>11} {:>11}",
        "", "mean us", "p50 us", "p99 us", "worst us", "frames/s", "actions/s", "spawns/s"
    );
    print_row("one process", size, &single);
    print_row(&format!("{} workers", size.workers), size, &multi);

    let stats = &end.stats;
    let frames = stats.frame.max(1);
    println!();
    println!(
        "a cluster frame, mean over {frames} frames: zones {} us, global realm {} us, exchange {} us",
        stats.zones_micros / frames,
        stats.global_micros / frames,
        stats.exchange_micros / frames,
    );
    let (sent, received): (u64, u64) = stats
        .workers
        .iter()
        .fold((0, 0), |(s, r), worker| (s + worker.bytes_sent, r + worker.bytes_received));
    println!(
        "routed: {} residents changed zone, {} messages; {:.1} MB to the workers, {:.1} MB from \
         them; the kernel dropped {} messages",
        stats.migrations,
        stats.messages,
        sent as f64 / 1e6,
        received as f64 / 1e6,
        stats.dropped_messages,
    );
    let residents: Vec<u64> = stats.workers.iter().map(|worker| worker.residents).collect();
    println!("residents per worker at the end: {residents:?}");

    // Both runs are at rest. The state of every realm must be the same.
    let expected = world_digests(&world);
    let differ: Vec<String> = expected
        .iter()
        .filter(|(realm, digest)| end.states.get(*realm) != Some(*digest))
        .map(|(realm, _)| realm.to_string())
        .collect();
    let totals = world.audit().expect("one process balances");
    let money = (totals.money.held, totals.money.minted, totals.money.burned);
    let cluster_money =
        (end.totals.money.held, end.totals.money.created, end.totals.money.destroyed);
    let same_rest = single.rest == multi.rest;
    let equal = differ.is_empty() && expected.len() == end.states.len() && money == cluster_money;
    println!();
    println!(
        "the world came to rest {} frames after the load in one process, {} in the cluster",
        single.rest, multi.rest
    );
    if equal && same_rest {
        println!(
            "the state of all {} realms is the same in both runs; {} cents held, {} made",
            expected.len(),
            money.0,
            money.1
        );
    } else {
        println!("THE RUNS DIFFER: realms {differ:?}; money {money:?} and {cluster_money:?}");
    }
    equal && same_rest
}

/// Shows that a worker process that is killed stops the cluster.
async fn crash() -> bool {
    let size = Size { workers: 3, residents: 30, frames: 0, districts: 3, zone_size: 16 };
    let coordinator = Coordinator::bind("127.0.0.1:0").expect("a free port");
    let address = coordinator.local_addr().expect("the address");
    let mut workers = start_workers(size, address);
    let config = ClusterConfig::manual(size.world(), size.workers);
    let (handle, join) = coordinator.start(plugins(), config).expect("the cluster starts");
    for n in 0..size.residents {
        let id = handle.spawn(spec(&format!("R{n}"))).await.expect("spawn");
        send(&handle, id, "walk", json!({ "x": (n * 5) % 48, "y": (n * 11) % 48 })).await;
    }
    println!("frame {}: three workers run", handle.step(20).await.expect("step"));
    println!("worker 1 (process {}) is killed", workers[1].id());
    workers[1].kill().expect("the worker process can be killed");
    let step = handle.step(1).await;
    println!("the next frame: {step:?}");
    let act = handle.act(ResidentId(1), common::act("chop", json!({}))).await.map(|ack| ack.status);
    println!("an action of a resident: {act:?}");
    let stopped = join.shutdown();
    println!(
        "the coordinator: {}",
        stopped.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.to_string())
    );
    let mut codes = Vec::new();
    for worker in &mut workers {
        codes.push(worker.wait().expect("the worker process ends").code());
    }
    println!("the exit codes of the workers: {codes:?}");
    step == Err(SimError::Shutdown) && act == Err(SimError::Shutdown) && stopped.is_err()
}

/// The body of a worker process.
fn worker(args: &[String]) -> ExitCode {
    let parsed = (|| {
        let coordinator: SocketAddr = args.first()?.parse().ok()?;
        let slot: u32 = args.get(1)?.parse().ok()?;
        let districts: u32 = args.get(2)?.parse().ok()?;
        let zone_size: u32 = args.get(3)?.parse().ok()?;
        Some((coordinator, slot, world_config(SEED, districts, zone_size)))
    })();
    let Some((coordinator, slot, world)) = parsed else {
        eprintln!("usage: cluster worker ADDRESS SLOT DISTRICTS ZONE_SIZE");
        return ExitCode::from(2);
    };
    match run_worker(plugins(), WorkerConfig::new(world, slot, coordinator)) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("worker {slot}: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Reads a number from the command line, or the default.
fn number(args: &[String], position: usize, default: u64) -> Option<u64> {
    match args.get(position) {
        Some(arg) => arg.replace('_', "").parse().ok(),
        None => Some(default),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("worker") {
        return worker(&args[1..]);
    }
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the tokio runtime starts");
    if args.first().map(String::as_str) == Some("crash") {
        return if tokio.block_on(crash()) { ExitCode::SUCCESS } else { ExitCode::FAILURE };
    }
    let size = (|| {
        Some(Size {
            workers: number(&args, 0, 3)?.max(1) as usize,
            residents: number(&args, 1, 2_000)?.max(1),
            frames: number(&args, 2, 300)?.max(1),
            districts: u32::try_from(number(&args, 3, 3)?).ok()?.max(1),
            zone_size: u32::try_from(number(&args, 4, 32)?).ok()?,
        })
    })();
    let Some(size) = size else {
        eprintln!(
            "usage: cluster [WORKERS] [RESIDENTS] [FRAMES] [DISTRICTS] [ZONE_SIZE]   \
             (defaults: 3, 2000, 300, 3 and 32)\n       cluster crash"
        );
        return ExitCode::from(2);
    };
    if tokio.block_on(compare(size)) { ExitCode::SUCCESS } else { ExitCode::FAILURE }
}
