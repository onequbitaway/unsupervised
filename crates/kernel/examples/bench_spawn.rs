//! Measures how long it takes to put residents into a world that has spawn
//! sites.
//!
//! Builds one city with zones of about 2,000 residents. Each zone has 40
//! sites, and each second zone has one site with the tag `spawn`, about as
//! many as a generated city has. Then it spawns N residents (first
//! argument, default 100,000) and prints the time of one spawn.
//!
//! It also prints how many residents share the fullest tile: the kernel
//! spreads the newcomers of a spawn site over its walkable tiles.
//!
//! The last line is a checksum of the place of every resident. The same
//! world gives the same checksum, so two builds of the kernel can be
//! compared: they must put each resident on the same tile.
//!
//! ```text
//! cargo run --release -p unsup-kernel --example bench_spawn -- 100000
//! ```

// A benchmark reads the wall clock. The simulation never does.
#![allow(clippy::disallowed_methods)]

use std::collections::BTreeMap;
use std::time::Instant;

use unsup_kernel::protocol::SpawnSpec;
use unsup_kernel::{
    Builder, CityConfig, CityId, Holder, OrgId, OwnerId, Plugin, Pos, RealmId, Rect, Scope, Site,
    SiteId, World, WorldConfig,
};

/// Residents per zone that the benchmark aims for.
const PER_ZONE: usize = 2_000;

/// Sites in each zone: 8 columns by 5 rows of lots.
const SITE_COLUMNS: u32 = 8;
const SITE_ROWS: u32 = 5;

/// One zone in this many has a site with the tag `spawn`.
const SPAWN_EVERY: u32 = 2;

struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        if b.scope() != Scope::Zone {
            return;
        }
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
                    let number = row * SITE_COLUMNS + column;
                    let rect = Rect::sized(
                        bounds.x0 + (column * width) as i32 + 1,
                        bounds.y0 + (row * height) as i32 + 1,
                        width.saturating_sub(2).max(1),
                        height.saturating_sub(2).max(1),
                    );
                    let id = SiteId(realm.alloc_id());
                    let shelter = number == 0 && info.id.0.is_multiple_of(SPAWN_EVERY);
                    let site = if shelter {
                        Site::new(id, "shelter", "Shelter", rect)
                            .with_tag("public")
                            .with_tag(Site::SPAWN_TAG)
                            .with_attr("beds", 24)
                    } else {
                        Site::new(id, "home", format!("Home {number}"), rect).with_attr("beds", 2)
                    };
                    realm.sites_mut().insert(site.with_owner(owner).with_attr("value", 50_000));
                }
            }
        });
    }
}

/// Reads the number of residents from the command line, or stops with the
/// usage text.
fn residents_argument() -> usize {
    match std::env::args().nth(1) {
        Some(arg) => match arg.replace('_', "").parse() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("usage: bench_spawn [RESIDENTS]   (default: 100000)");
                std::process::exit(2);
            }
        },
        None => 100_000,
    }
}

/// Returns FNV-1a over the zone and the position of every resident, in id
/// order, the number of tiles with a resident, and the number of residents
/// on the fullest tile.
fn places(world: &World, residents: u64) -> (u64, usize, usize) {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut mix = |value: u64| {
        for byte in value.to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    let mut on_tile: BTreeMap<Pos, usize> = BTreeMap::new();
    for id in 1..=residents {
        let id = unsup_kernel::ResidentId(id);
        let zone = world.locate(id).expect("every resident is in a zone");
        let realm = world.realm(RealmId::Zone(zone)).expect("the world hosts every zone");
        let pos = realm.pos(realm.idx_of(id).expect("the resident is in its zone"));
        mix(u64::from(zone.0));
        mix(pos.x as u64);
        mix(pos.y as u64);
        *on_tile.entry(pos).or_default() += 1;
    }
    (hash, on_tile.len(), on_tile.values().copied().max().unwrap_or(0))
}

fn main() {
    let residents = residents_argument().max(1);
    let districts = (residents.div_ceil(PER_ZONE) as f64).sqrt().ceil().max(1.0) as u32;
    let config = WorldConfig {
        seed: 42,
        cities: vec![CityConfig::new("Bench", Pos::new(0, 0), districts)],
        ..WorldConfig::default()
    };
    let mut world = World::new(&[&Town], config);
    let zones = world.layout().zones().len();
    let sites: usize = (0..zones as u32)
        .filter_map(|zone| world.realm(RealmId::Zone(unsup_kernel::ZoneId(zone))))
        .map(|realm| realm.sites().len())
        .sum();
    let spawn_sites: usize = (0..zones as u32)
        .filter_map(|zone| world.realm(RealmId::Zone(unsup_kernel::ZoneId(zone))))
        .map(|realm| realm.sites().with_tag(Site::SPAWN_TAG).count())
        .sum();
    println!("world: {zones} zones, {sites} sites, {spawn_sites} of them with the tag `spawn`");

    let start = Instant::now();
    for i in 0..residents as u64 {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i % 1_000),
            name: format!("r{i}"),
            model: None,
            city: Some(CityId(0)),
        };
        world.spawn(spec).expect("spawn");
    }
    let elapsed = start.elapsed();
    println!(
        "spawn: {residents} residents in {:.2} s, {:.2} microseconds for each spawn",
        elapsed.as_secs_f64(),
        elapsed.as_secs_f64() * 1_000_000.0 / residents as f64
    );
    world.audit().expect("the ledgers are balanced");
    let (checksum, tiles, fullest) = places(&world, residents as u64);
    println!("crowding: the residents stand on {tiles} tiles, {fullest} on the fullest tile");
    println!("places: checksum {checksum:016x}");
}
