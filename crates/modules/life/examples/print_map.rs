//! Prints the map of a generated city as text, one zone after another.
//!
//! ```text
//! cargo run -p unsup-life --example print_map -- [SEED] [DISTRICTS]
//! ```
//!
//! The defaults are seed 1 and a city of 2 by 2 districts. Each tile is one
//! character; a legend follows each zone.

use unsup_kernel::{CityConfig, Pos, RealmId, World, WorldConfig};
use unsup_life::ZoneFacts;
use unsup_life::names::tile;

/// Returns the character of a tile code.
fn glyph(code: u8) -> char {
    match code {
        tile::GRASS => '.',
        tile::ROAD => '=',
        tile::PAVEMENT => ':',
        tile::FLOOR => ' ',
        tile::WALL => '#',
        tile::WATER => '~',
        tile::TREES => 'T',
        tile::FIELD => '"',
        tile::ROCK => '^',
        _ => '?',
    }
}

/// Reads a number from the command line, or stops with the usage text.
fn argument(position: usize, default: u64) -> u64 {
    match std::env::args().nth(position) {
        Some(text) => match text.parse() {
            Ok(number) => number,
            Err(_) => {
                eprintln!("usage: print_map [SEED] [DISTRICTS]   (defaults: 1 and 2)");
                std::process::exit(2);
            }
        },
        None => default,
    }
}

fn main() {
    let seed = argument(1, 1);
    let districts = u32::try_from(argument(2, 2)).unwrap_or(2).max(1);
    let config = WorldConfig {
        seed,
        cities: vec![CityConfig::new("Sample", Pos::new(0, 0), districts)],
        ..WorldConfig::default()
    };
    let life = unsup_life::plugin();
    let world = World::new(&[&life], config);
    for zone in world.layout().zones() {
        let Some(realm) = world.realm(RealmId::Zone(zone.id)) else {
            continue;
        };
        let district = realm.res::<ZoneFacts>().district.clone();
        println!("zone {} \"{}\": {district}", zone.id, zone.name);
        let tiles = realm.tiles();
        for row in tiles.as_slice().chunks(tiles.width().max(1) as usize) {
            println!("{}", row.iter().map(|&code| glyph(code)).collect::<String>());
        }
        for site in realm.sites().iter() {
            println!(
                "  {:>16} {:<18} at ({}, {}) {}x{} value {}",
                site.kind,
                site.name,
                site.rect.x0,
                site.rect.y0,
                site.rect.width(),
                site.rect.height(),
                site.attr("value"),
            );
        }
        println!();
    }
}
