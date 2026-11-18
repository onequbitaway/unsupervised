//! Measures the frame time of the kernel under load.
//!
//! Builds a world with N residents (first argument, default 100,000) in
//! zones of about 2,000 residents. Every resident has a running action
//! that a routine starts again when it ends, and an attribute that decays
//! every tick. Then it steps 60 frames and prints the mean and the worst
//! frame time and an estimate of the memory per resident.
//!
//! The second argument is the number of warm-up frames before the
//! measurement (default 30). A long warm-up shows the steady state of a
//! world that has run for a while.
//!
//! ```text
//! cargo run --release -p unsup-kernel --example bench_tick -- 100000
//! cargo run --release -p unsup-kernel --example bench_tick -- 100000 1500
//! ```

// A benchmark reads the wall clock. The simulation never does.
#![allow(clippy::disallowed_methods)]

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::protocol::{ActionRequest, Condition, Routine, SpawnSpec};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    AttrDef, Begin, Builder, CityConfig, CityId, Component, Holder, Money, OwnerId, Plugin, Pos,
    Scope, Stage, VerbSpec, World, WorldConfig, ZoneId,
};

/// Residents per zone that the benchmark aims for.
const PER_ZONE: usize = 2_000;

/// Frames that run before the measurement starts, unless the second
/// argument says otherwise.
const WARMUP_FRAMES: usize = 30;

/// Frames that are measured.
const FRAMES: usize = 60;

/// Per-resident state of the benchmark module.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Job {
    shifts: u32,
    earned: i64,
}

impl Component for Job {
    const NAME: &'static str = "bench.job";
}

/// Arguments of `work`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WorkArgs {
    /// Duration in ticks.
    ticks: u32,
}

struct Bench;

impl Plugin for Bench {
    fn name(&self) -> &'static str {
        "bench"
    }

    fn build(&self, b: &mut Builder) {
        let energy = b.attr(AttrDef::new("energy").range(0, 10_000).default(10_000));
        let wages = b.faucet("bench.wages");
        b.component::<Job>();
        if b.scope() == Scope::Zone {
            // The decaying attribute: one pass over a dense column per tick.
            b.system(Stage::Update, 0, "bench.decay", move |realm| {
                for value in realm.attrs_mut(energy).iter_mut() {
                    *value = (*value - 1).max(0);
                }
            });
            // A periodic system, as most module systems are.
            b.every(60, Stage::Post, 0, "bench.rest", move |realm| {
                for value in realm.attrs_mut(energy).iter_mut() {
                    if *value < 5_000 {
                        *value += 2_000;
                    }
                }
            });
            // Spread new residents over all zones.
            b.on_spawn(0, move |realm, idx| {
                let id = realm.id_of(idx);
                realm.ledger_mut().mint(wages, Holder::Resident(id), Money(1_000));
                let zones = realm.layout().zones();
                let zone = &zones[(id.0 % zones.len() as u64) as usize];
                let (x, y) = {
                    let mut rng = realm.rng();
                    (rng.range(0, 64) as i32, rng.range(0, 64) as i32)
                };
                let pos = Pos::new(zone.bounds.x0 + x, zone.bounds.y0 + y);
                realm.teleport(idx, zone.id, pos).expect("the position is inside the zone");
            });
        }
        b.verb::<WorkArgs>(
            VerbSpec {
                verb: "work",
                module: "bench",
                summary: "Work a shift.",
                free: false,
                allowed_in: &[],
            },
            |_realm, _ctx, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, args| {
                let pay = i64::from(args.ticks) * 10;
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(pay));
                let mut jobs = realm.col_mut::<Job>();
                let job = &mut jobs[ctx.idx];
                job.shifts += 1;
                job.earned += pay;
                Ok(json!({ "pay": pay }))
            },
        );
    }
}

/// Returns how many `work` actions have finished in the whole world.
fn shifts(world: &World) -> u64 {
    world
        .layout()
        .zones()
        .iter()
        .filter_map(|zone| world.realm(unsup_kernel::RealmId::Zone(zone.id)))
        .map(|realm| realm.col::<Job>().iter().map(|job| u64::from(job.shifts)).sum::<u64>())
        .sum()
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1_000.0
}

/// Reads a number from the command line, or stops with the usage text.
fn argument(position: usize, default: usize) -> usize {
    match std::env::args().nth(position) {
        Some(arg) => match arg.replace('_', "").parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!(
                    "usage: bench_tick [RESIDENTS] [WARMUP_FRAMES]   (defaults: 100000 and 30)"
                );
                std::process::exit(2);
            }
        },
        None => default,
    }
}

fn main() {
    let residents = argument(1, 100_000).max(1);
    let warmup = argument(2, WARMUP_FRAMES);
    let districts = (residents.div_ceil(PER_ZONE) as f64).sqrt().ceil().max(1.0) as u32;
    let config = WorldConfig {
        seed: 42,
        cities: vec![CityConfig::new("Bench", Pos::new(0, 0), districts)],
        ..WorldConfig::default()
    };

    let build = Instant::now();
    let mut world = World::new(&[&Bench], config);
    for i in 0..residents as u64 {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i % 1_000),
            name: format!("r{i}"),
            model: None,
            city: Some(CityId(0)),
        };
        let id = world.spawn(spec).expect("spawn");
        // A routine keeps the resident busy for ever. Shift lengths differ,
        // so the action completions spread over the ticks.
        let shift = 5 + i % 20;
        let routine = Routine {
            name: "work".into(),
            when: Condition::Always,
            then: ActionRequest::new("work", json!({ "ticks": shift })),
        };
        world
            .act(id, ActionRequest::new("set_routines", json!({ "routines": [routine] })))
            .expect("act");
        world.act(id, ActionRequest::new("work", json!({ "ticks": 1 + i % 24 }))).expect("act");
    }
    let build = build.elapsed();
    let zones = world.layout().zones().len();
    println!(
        "world: {residents} residents in {zones} zones ({} per zone), built in {:.1} s, {} threads",
        residents / zones,
        build.as_secs_f64(),
        rayon::current_num_threads()
    );

    for _ in 0..warmup {
        world.step();
    }
    let shifts_before = shifts(&world);
    let mut times = Vec::with_capacity(FRAMES);
    for _ in 0..FRAMES {
        let start = Instant::now();
        world.step();
        times.push(start.elapsed());
    }
    let ended = shifts(&world) - shifts_before;
    let total: Duration = times.iter().sum();
    let mean = total / FRAMES as u32;
    let worst = times.iter().max().copied().unwrap_or_default();
    let best = times.iter().min().copied().unwrap_or_default();

    let busy = (0..zones as u32)
        .filter_map(|z| world.realm(unsup_kernel::RealmId::Zone(ZoneId(z))))
        .map(|realm| realm.indices().filter(|&idx| realm.activity(idx).is_some()).count())
        .sum::<usize>();
    let memory = world.memory_estimate();
    world.audit().expect("the ledgers are balanced");

    println!("frames: {FRAMES} measured after {warmup} warm-up frames, frame {}", world.frame());
    println!(
        "frame time: mean {:.3} ms, worst {:.3} ms, best {:.3} ms",
        millis(mean),
        millis(worst),
        millis(best)
    );
    println!(
        "load: {busy} of {residents} residents have a running action, {} actions ended and started per frame",
        ended / FRAMES as u64
    );
    println!(
        "memory estimate: {} bytes per resident, {:.1} MiB for all residents",
        memory.per_resident(),
        memory.bytes as f64 / (1024.0 * 1024.0)
    );
    println!(
        "budget: one frame per second leaves {:.1}x headroom at the worst frame",
        1_000.0 / millis(worst)
    );
}
