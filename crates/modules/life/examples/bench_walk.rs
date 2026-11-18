//! Measures the frame time of the life module while many residents walk.
//!
//! Builds one city with N residents (first argument, default 100,000) in
//! zones of about 2,000 residents. Every resident gets a plan of walks to
//! random tiles: most in the zone where it starts, the others in a zone
//! next to it, so that residents cross zone borders all the time. Then it
//! steps 60 frames and prints the mean, the median and the worst frame time.
//!
//! The second argument is the number of warm-up frames before the
//! measurement (default 30).
//!
//! ```text
//! cargo run --release -p unsup-life --example bench_walk -- 100000
//! cargo run --release -p unsup-life --example bench_walk -- 1000000 60
//! ```

// A benchmark reads the wall clock. The simulation never does.
#![allow(clippy::disallowed_methods)]

use std::time::{Duration, Instant};

use unsup_kernel::protocol::{ActionRequest, ActionStatus, SpawnSpec};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    Builder, CityConfig, CityId, OwnerId, Plugin, Pos, RealmId, ResidentId, Rng, World,
    WorldConfig, ZoneId,
};
use unsup_life::Walkers;
use unsup_life::names::tile;

/// Residents per zone that the benchmark aims for.
const PER_ZONE: usize = 2_000;

/// Frames that run before the measurement starts, unless the second
/// argument says otherwise.
const WARMUP_FRAMES: usize = 30;

/// Frames that are measured.
const FRAMES: usize = 60;

/// Walks in the plan of each resident: the one that runs and the ones in
/// the queue. The queue of the kernel holds 8 actions.
const WALKS: usize = 9;

/// One walk in this many goes to a zone next to the start zone.
const CROSS_ONE_IN: u64 = 4;

/// Seed of the world and of the choice of targets.
const SEED: u64 = 42;

/// Spreads new residents over all zones. A resident starts on the road at
/// the north edge of its zone; the life module makes every edge a road.
struct Spread;

impl Plugin for Spread {
    fn name(&self) -> &'static str {
        "bench"
    }

    fn build(&self, b: &mut Builder) {
        b.on_spawn(100, |realm, idx| {
            let id = realm.id_of(idx);
            let zones = realm.layout().zones();
            let zone = &zones[(id.0 % zones.len() as u64) as usize];
            let column = realm.rng().range(0, i64::from(zone.bounds.width())) as i32;
            let pos = Pos::new(zone.bounds.x0 + column, zone.bounds.y0);
            realm.teleport(idx, zone.id, pos).expect("the position is inside the zone");
        });
    }
}

/// Returns the tiles of every zone on which a resident can stand.
fn walkable_tiles(world: &World) -> Vec<Vec<Pos>> {
    world
        .layout()
        .zones()
        .iter()
        .map(|zone| {
            let realm = world.realm(RealmId::Zone(zone.id)).expect("the world hosts every zone");
            let tiles = realm.tiles();
            let b = zone.bounds;
            (b.y0..b.y1)
                .flat_map(|y| (b.x0..b.x1).map(move |x| Pos::new(x, y)))
                .filter(|pos| tiles.get(*pos).is_some_and(tile::is_walkable))
                .collect()
        })
        .collect()
}

/// Returns the zones that share an edge with a zone of a square city with
/// `side` zones on each side.
fn neighbours(zone: usize, side: usize) -> Vec<usize> {
    let (col, row) = (zone % side, zone / side);
    let mut out = Vec::with_capacity(4);
    if row > 0 {
        out.push(zone - side);
    }
    if col + 1 < side {
        out.push(zone + 1);
    }
    if row + 1 < side {
        out.push(zone + side);
    }
    if col > 0 {
        out.push(zone - 1);
    }
    out
}

/// Returns the number of residents that walk, in all zones.
fn walking(world: &World) -> usize {
    (0..world.layout().zones().len() as u32)
        .filter_map(|zone| world.realm(RealmId::Zone(ZoneId(zone))))
        .map(|realm| realm.res::<Walkers>().len())
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
                    "usage: bench_walk [RESIDENTS] [WARMUP_FRAMES]   (defaults: 100000 and 30)"
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
    let side = (residents.div_ceil(PER_ZONE) as f64).sqrt().ceil().max(2.0) as usize;
    let config = WorldConfig {
        seed: SEED,
        cities: vec![CityConfig::new("Bench", Pos::new(0, 0), side as u32)],
        ..WorldConfig::default()
    };

    let build = Instant::now();
    let life = unsup_life::plugin();
    let mut world = World::new(&[&life, &Spread], config);
    let zones = world.layout().zones().len();
    let tiles = walkable_tiles(&world);
    let mut rng = Rng::stream(SEED, 1);
    let mut refused = 0usize;
    for i in 0..residents as u64 {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i % 1_000),
            name: format!("w{i}"),
            model: None,
            city: Some(CityId(0)),
        };
        let id: ResidentId = world.spawn(spec).expect("spawn");
        let home = (id.0 % zones as u64) as usize;
        let near = neighbours(home, side);
        for _ in 0..WALKS {
            let zone = if rng.chance(1, CROSS_ONE_IN) {
                near[rng.below(near.len() as u64) as usize]
            } else {
                home
            };
            let target = tiles[zone][rng.below(tiles[zone].len() as u64) as usize];
            let request = ActionRequest::new("move_to", json!({ "x": target.x, "y": target.y }));
            let ack = world.act(id, request).expect("act");
            if matches!(ack.status, ActionStatus::Rejected { .. }) {
                refused += 1;
            }
        }
    }
    let build = build.elapsed();
    let threads = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    println!(
        "world: {residents} walkers in {zones} zones ({} per zone), built in {:.1} s, {threads} threads",
        residents / zones,
        build.as_secs_f64(),
    );
    println!("plans: {WALKS} walks for each walker, {refused} walks refused at once");

    for _ in 0..warmup {
        world.step();
    }
    let walking_before = walking(&world);
    let mut times = Vec::with_capacity(FRAMES);
    let mut steps = 0usize;
    let mut crossings = 0usize;
    for _ in 0..FRAMES {
        let start = Instant::now();
        let frame = world.step();
        times.push(start.elapsed());
        steps += frame.zones.iter().map(|zone| zone.moved.len()).sum::<usize>();
        crossings += frame.zones.iter().map(|zone| zone.left.len()).sum::<usize>();
    }
    let total: Duration = times.iter().sum();
    let mean = total / FRAMES as u32;
    let worst = times.iter().max().copied().unwrap_or_default();
    let best = times.iter().min().copied().unwrap_or_default();
    let mut sorted = times.clone();
    sorted.sort_unstable();
    let median = sorted[FRAMES / 2];
    let walking_after = walking(&world);
    let memory = world.memory_estimate();
    world.audit().expect("the ledgers are balanced");

    println!("frames: {FRAMES} measured after {warmup} warm-up frames, frame {}", world.frame());
    println!(
        "frame time: mean {:.3} ms, median {:.3} ms, worst {:.3} ms, best {:.3} ms",
        millis(mean),
        millis(median),
        millis(worst),
        millis(best)
    );
    println!(
        "load: {walking_before} walkers at the start and {walking_after} at the end, {} steps and {} zone crossings per frame",
        steps / FRAMES,
        crossings / FRAMES
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
