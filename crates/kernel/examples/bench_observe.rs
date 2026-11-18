//! Measures the cost of observations.
//!
//! The first argument chooses what to measure.
//!
//! `nearby` measures one observation in two places:
//!
//! - a town: 9 zones with 2,000 residents each (second argument), at random
//!   tiles, and 40 sites per zone. The observers are the residents of the
//!   zone in the middle, so many of them see across a border.
//! - a crowd: 15,000 residents (third argument) on one tile.
//!
//! For each place it prints the mean time of one observation with all
//! kernel sections, with only the section `nearby`, and with only the
//! section `self`. It also prints a checksum of the `nearby` sections. Two
//! builds of the kernel that give the same checksum gave the same lists.
//!
//! `batch` measures observations per second in a large world: N residents
//! (second argument, default 1,000,000) in zones of 2,000. It observes
//! every resident once with `World::observe` on one thread, then with
//! `World::observe_batch` on one thread and on all processor cores. A
//! third argument `global` adds four observers in the global realm, which
//! only one thread at a time can run.
//!
//! ```text
//! cargo run --release -p unsup-kernel --example bench_observe -- nearby
//! cargo run --release -p unsup-kernel --example bench_observe -- nearby 2000 15000
//! cargo run --release -p unsup-kernel --example bench_observe -- batch 1000000
//! cargo run --release -p unsup-kernel --example bench_observe -- batch 1000000 global
//! ```

// A benchmark reads the wall clock. The simulation never does.
#![allow(clippy::disallowed_methods)]

use std::time::Instant;

use serde_json::json;
use unsup_kernel::protocol::{ObserveOpts, SpawnSpec};
use unsup_kernel::{
    AttrDef, Builder, CityConfig, CityId, Holder, Money, OrgId, OwnerId, Plugin, Pos, Rect,
    ResidentId, Scope, Site, SiteId, World, WorldConfig, ZoneId,
};

/// Residents per zone that the benchmark aims for.
const PER_ZONE: usize = 2_000;

/// Sites in each zone: 8 columns by 5 rows of lots.
const SITE_COLUMNS: u32 = 8;
const SITE_ROWS: u32 = 5;

/// Residents that join between two frames while the world is built.
const SPAWNS_PER_FRAME: usize = 1_000;

/// Observers in the crowd whose observation is measured.
const CROWD_OBSERVERS: usize = 200;

/// Requests in one call of `observe_batch`.
const BATCH: usize = 100_000;

/// A town with sites and residents at random tiles of all zones.
struct Town {
    /// Spread the residents over all zones. When false they stay where they
    /// appear: at the centre of the first zone.
    spread: bool,
    /// Add observers to the global realm.
    global_sections: bool,
}

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        // About as many attributes as the game modules give a resident.
        for name in ["food", "energy", "health", "mood", "age", "labor", "farming", "trading"] {
            b.attr(AttrDef::new(name).range(0, 1_000).default(500));
        }
        let grant = b.faucet("bench.start");
        match b.scope() {
            Scope::Zone => {
                let spread = self.spread;
                b.on_worldgen(0, |realm, _context| {
                    let Some(info) = realm.zone_info() else {
                        return;
                    };
                    let bounds = info.bounds;
                    let width = bounds.width() / SITE_COLUMNS;
                    let height = bounds.height() / SITE_ROWS;
                    let owner = Holder::Org(OrgId::city(info.city));
                    for row in 0..SITE_ROWS {
                        for column in 0..SITE_COLUMNS {
                            let rect = Rect::sized(
                                bounds.x0 + (column * width) as i32 + 1,
                                bounds.y0 + (row * height) as i32 + 1,
                                width - 2,
                                height - 2,
                            );
                            let id = SiteId(realm.alloc_id());
                            let name = format!("Lot {}", row * SITE_COLUMNS + column);
                            realm
                                .sites_mut()
                                .insert(Site::new(id, "home", name, rect).with_owner(owner));
                        }
                    }
                });
                b.on_spawn(0, move |realm, idx| {
                    let id = realm.id_of(idx);
                    realm.ledger_mut().mint(grant, Holder::Resident(id), Money(1_000));
                    if !spread {
                        return;
                    }
                    let zones = realm.layout().zones();
                    let zone = &zones[(id.0 % zones.len() as u64) as usize];
                    let size = zone.bounds.width() as i64;
                    let (x, y) = {
                        let mut rng = realm.rng();
                        (rng.range(0, size) as i32, rng.range(0, size) as i32)
                    };
                    let pos = Pos::new(zone.bounds.x0 + x, zone.bounds.y0 + y);
                    realm.teleport(idx, zone.id, pos).expect("the position is inside the zone");
                });
            }
            Scope::Global => {
                if !self.global_sections {
                    return;
                }
                // What the observers of the game modules do in the global
                // realm: look the resident up and build a small section.
                for section in ["bank", "orgs", "mail", "missions"] {
                    b.observe_global(section, 0, move |realm, resident, _opts| {
                        let directory = realm.directory();
                        let entry = directory.get(resident)?;
                        let cash = realm.ledger().money(Holder::Resident(resident));
                        Some(json!({ "name": entry.name, "zone": entry.zone, "balance": cash }))
                    });
                }
            }
        }
    }
}

/// Makes a world with one city of `districts` by `districts` zones and
/// spawns `residents` residents. A frame runs after every 1,000 spawns.
fn build(plugin: &'static Town, districts: u32, residents: usize) -> (World, Vec<ResidentId>) {
    let config = WorldConfig {
        seed: 42,
        cities: vec![CityConfig::new("Bench", Pos::new(0, 0), districts)],
        ..WorldConfig::default()
    };
    let mut world = World::new(&[plugin], config);
    let mut ids = Vec::with_capacity(residents);
    for i in 0..residents {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i as u64 % 1_000),
            name: format!("r{i}"),
            model: None,
            city: Some(CityId(0)),
        };
        ids.push(world.spawn(spec).expect("spawn"));
        if (i + 1) % SPAWNS_PER_FRAME == 0 {
            world.step();
        }
    }
    // Two frames: every zone tells the zones around it who is near the
    // border, and they take the news.
    world.step();
    world.step();
    (world, ids)
}

/// Options that ask for one section, or for all with `None`.
fn only(section: Option<&str>) -> ObserveOpts {
    ObserveOpts { sections: section.map(|s| vec![s.to_string()]), ..ObserveOpts::default() }
}

/// Observes each resident `rounds` times and returns the mean time of one
/// observation in microseconds.
fn time_observations(world: &World, who: &[ResidentId], opts: &ObserveOpts, rounds: usize) -> f64 {
    let start = Instant::now();
    let mut seen = 0usize;
    for _ in 0..rounds {
        for &id in who {
            let observation = world.observe(id, opts).expect("the resident lives");
            seen += observation.sections.len();
        }
    }
    assert!(seen > 0 || who.is_empty());
    start.elapsed().as_secs_f64() * 1e6 / (rounds * who.len()).max(1) as f64
}

/// Returns a checksum of the `nearby` sections of the observers, and the
/// mean number of residents in a list.
fn nearby_checksum(world: &World, who: &[ResidentId]) -> (String, f64) {
    let mut hasher = blake3::Hasher::new();
    let mut listed = 0usize;
    let opts = only(Some("nearby"));
    for &id in who {
        let observation = world.observe(id, &opts).expect("the resident lives");
        let nearby = &observation.sections["nearby"];
        listed += nearby["residents"].as_array().map_or(0, Vec::len);
        hasher.update(nearby.to_string().as_bytes());
    }
    let hex = hasher.finalize().to_hex();
    (hex[..16].to_string(), listed as f64 / who.len().max(1) as f64)
}

/// Prints the cost of one observation for a list of observers.
fn report_place(label: &str, world: &World, who: &[ResidentId], rounds: usize) {
    let all = time_observations(world, who, &only(None), rounds);
    let nearby = time_observations(world, who, &only(Some("nearby")), rounds);
    let own = time_observations(world, who, &only(Some("self")), rounds);
    let (checksum, listed) = nearby_checksum(world, who);
    println!(
        "{label}: one observation {all:.2} us (only nearby {nearby:.2} us, only self {own:.2} us), {} observers, {listed:.1} residents in a list, checksum {checksum}",
        who.len()
    );
}

fn nearby(per_zone: usize, crowd: usize) {
    static TOWN: Town = Town { spread: true, global_sections: false };
    let (world, ids) = build(&TOWN, 3, per_zone * 9);
    let middle: Vec<ResidentId> =
        ids.iter().copied().filter(|&id| world.locate(id) == Some(ZoneId(4))).collect();
    println!("town: {} residents in 9 zones, {} threads", ids.len(), rayon::current_num_threads());
    report_place("town, zone in the middle", &world, &middle, 3);
    report_place("town, all zones", &world, &ids, 1);
    drop(world);

    static SQUARE: Town = Town { spread: false, global_sections: false };
    let (world, ids) = build(&SQUARE, 1, crowd);
    let step = (ids.len() / CROWD_OBSERVERS).max(1);
    let observers: Vec<ResidentId> = ids.iter().copied().step_by(step).collect();
    println!(
        "crowd: {} residents on the tile {}",
        ids.len(),
        world.layout().zones()[0].bounds.centre()
    );
    report_place("crowd on one tile", &world, &observers, 1);
}

fn batch(residents: usize, global_sections: bool) {
    static PLAIN: Town = Town { spread: true, global_sections: false };
    static GLOBAL: Town = Town { spread: true, global_sections: true };
    let plugin = if global_sections { &GLOBAL } else { &PLAIN };
    let districts = (residents.div_ceil(PER_ZONE) as f64).sqrt().ceil().max(1.0) as u32;
    let start = Instant::now();
    let (mut world, ids) = build(plugin, districts, residents);
    println!(
        "world: {} residents in {} zones, built in {:.1} s, {} threads, {} observers in the global realm",
        ids.len(),
        world.layout().zones().len(),
        start.elapsed().as_secs_f64(),
        rayon::current_num_threads(),
        if global_sections { 4 } else { 0 },
    );
    let opts = ObserveOpts::default();

    // One thread, one call per resident. The clock runs while the
    // observations are made, and stops while they are freed: a runtime
    // hands them to the threads of the gateway, which free them.
    let mut single = 0.0;
    let mut sections = 0usize;
    for chunk in ids.chunks(BATCH) {
        let start = Instant::now();
        let observations: Vec<_> =
            chunk.iter().map(|&id| world.observe(id, &opts).expect("the resident lives")).collect();
        single += start.elapsed().as_secs_f64();
        sections += observations.iter().map(|o| o.sections.len()).sum::<usize>();
    }
    println!(
        "World::observe, one thread: {:.0} observations per second ({:.2} us each, {} sections each)",
        ids.len() as f64 / single,
        single * 1e6 / ids.len() as f64,
        sections / ids.len().max(1)
    );

    // The same requests in batches of 100,000, as a runtime sends them
    // between two frames.
    let requests: Vec<(ResidentId, ObserveOpts)> =
        ids.iter().map(|&id| (id, opts.clone())).collect();
    let batches = |world: &mut World| -> (f64, usize) {
        let mut seconds = 0.0;
        let mut sections = 0usize;
        for chunk in requests.chunks(BATCH) {
            let start = Instant::now();
            let answers = world.observe_batch(chunk);
            seconds += start.elapsed().as_secs_f64();
            sections += answers
                .iter()
                .map(|answer| answer.as_ref().expect("the resident lives").sections.len())
                .sum::<usize>();
        }
        (seconds, sections)
    };
    let one = rayon::ThreadPoolBuilder::new().num_threads(1).build().expect("a thread pool");
    let (alone, seen) = one.install(|| batches(&mut world));
    assert_eq!(seen, sections, "a batch gives the sections that `observe` gives");
    println!(
        "World::observe_batch, one thread: {:.0} observations per second ({:.2} us each)",
        ids.len() as f64 / alone,
        alone * 1e6 / ids.len() as f64
    );
    // Twice: the first round also warms the threads up.
    for round in ["first round", "second round"] {
        let (all, seen) = batches(&mut world);
        assert_eq!(seen, sections, "a batch gives the sections that `observe` gives");
        println!(
            "World::observe_batch, {} threads, {round}: {:.0} observations per second ({:.2} us of wall time each), {:.1} times `World::observe`",
            rayon::current_num_threads(),
            ids.len() as f64 / all,
            all * 1e6 / ids.len() as f64,
            single / all
        );
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: bench_observe nearby [PER_ZONE] [CROWD]   (defaults: 2000 and 15000)\n       bench_observe batch [RESIDENTS] [global]   (default: 1000000)"
    );
    std::process::exit(2);
}

/// Reads a number from the command line, or stops with the usage text.
fn number(position: usize, default: usize) -> usize {
    match std::env::args().nth(position) {
        Some(arg) => arg.replace('_', "").parse().unwrap_or_else(|_| usage()),
        None => default,
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("nearby") => nearby(number(2, PER_ZONE).max(1), number(3, 15_000).max(1)),
        Some("batch") => {
            let global = match std::env::args().nth(3).as_deref() {
                None => false,
                Some("global") => true,
                Some(_) => usage(),
            };
            batch(number(2, 1_000_000).max(1), global);
        }
        _ => usage(),
    }
}
