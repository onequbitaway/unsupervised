//! The map of the showcase world: streets, lots, buildings and sites.
//!
//! Every district has a street along each side and an avenue cross in the
//! middle. That leaves four blocks. A block holds four lots, a park or a
//! piece of countryside, by the type of the district.

use unsup_kernel::{Pos, Realm, Rect, Rng, Site, SiteId, TileMap};

/// Tile codes of `docs/design/game-design.md`, section 2.
pub mod tile {
    /// Grass.
    pub const GRASS: u8 = 0;
    /// Road.
    pub const ROAD: u8 = 1;
    /// Pavement.
    pub const PAVEMENT: u8 = 2;
    /// Floor inside a site.
    pub const FLOOR: u8 = 3;
    /// Wall.
    pub const WALL: u8 = 4;
    /// Water.
    pub const WATER: u8 = 5;
    /// Trees.
    pub const TREES: u8 = 6;
    /// Field.
    pub const FIELD: u8 = 7;
    /// Rock.
    pub const ROCK: u8 = 8;
}

/// Returns true when a resident can stand on a tile of this kind.
pub fn walkable(kind: u8) -> bool {
    !matches!(kind, tile::WALL | tile::WATER | tile::ROCK)
}

/// The types of a district, in the order in which a city uses them.
const DISTRICTS: [&str; 5] = ["downtown", "residential", "commercial", "industrial", "rural"];

/// Site kinds of each built district type.
fn kinds(district: &str) -> &'static [&'static str] {
    match district {
        "downtown" => &["city_hall", "bank", "office", "courthouse", "newsroom", "exchange"],
        "residential" => &["home", "apartment", "home", "school", "shelter", "apartment"],
        "commercial" => &["shop", "restaurant", "shop", "office", "hospital", "restaurant"],
        _ => &["factory", "warehouse", "workshop", "depot", "factory", "police_station"],
    }
}

const FAMILIES: [&str; 12] = [
    "Abara",
    "Bianchi",
    "Chen",
    "Dufour",
    "Eze",
    "Fischer",
    "Garcia",
    "Haddad",
    "Ito",
    "Jensen",
    "Kowalski",
    "Lindqvist",
];

fn label(kind: &str) -> String {
    let mut words = kind.replace('_', " ");
    if let Some(first) = words.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    words
}

fn add_site(realm: &Realm, kind: &str, name: String, rect: Rect) -> SiteId {
    let id = SiteId(realm.alloc_id());
    realm.sites_mut().insert(Site::new(id, kind, name, rect))
}

/// Builds one house on a lot: walls, a floor and a door towards `door`.
fn building(realm: &Realm, rng: &mut Rng, district: &str, lot: Rect, door: Pos) {
    let house = Rect::new(lot.x0 + 1, lot.y0 + 1, lot.x1 - 1, lot.y1 - 1);
    if house.width() < 4 || house.height() < 4 {
        return;
    }
    let kind = *rng.pick(kinds(district)).unwrap_or(&"office");
    let family = *rng.pick(&FAMILIES).unwrap_or(&"Abara");
    {
        let mut tiles = realm.tiles_mut();
        tiles.fill(lot, tile::PAVEMENT);
        tiles.fill(house, tile::WALL);
        tiles.fill(Rect::new(house.x0 + 1, house.y0 + 1, house.x1 - 1, house.y1 - 1), tile::FLOOR);
        // The door is in the wall that looks at the avenue cross.
        let middle = house.centre();
        let at = if door.x.abs_diff(middle.x) > door.y.abs_diff(middle.y) {
            Pos::new(if door.x > middle.x { house.x1 - 1 } else { house.x0 }, middle.y)
        } else {
            Pos::new(middle.x, if door.y > middle.y { house.y1 - 1 } else { house.y0 })
        };
        tiles.set(at, tile::FLOOR);
    }
    add_site(realm, kind, format!("{family} {}", label(kind)), house);
}

/// Scatters a tile kind over a rectangle: `per_mille` of the tiles get it.
fn scatter(tiles: &mut TileMap, rng: &mut Rng, rect: Rect, kind: u8, per_mille: u64) {
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            if rng.chance(per_mille, 1_000) {
                tiles.set(Pos::new(x, y), kind);
            }
        }
    }
}

/// Makes a block a park with trees and a pond. Returns its site.
fn park(realm: &Realm, rng: &mut Rng, block: Rect, name: String) -> SiteId {
    {
        let mut tiles = realm.tiles_mut();
        scatter(&mut tiles, rng, block, tile::TREES, 140);
        let pond = Rect::new(block.x0 + 3, block.y0 + 3, block.x0 + 9, block.y0 + 7);
        tiles.fill(pond, tile::WATER);
        // A paved square in the middle: new residents appear here.
        let c = block.centre();
        tiles.fill(Rect::new(c.x - 2, c.y - 2, c.x + 3, c.y + 3), tile::PAVEMENT);
    }
    add_site(realm, "park", name, block)
}

/// Makes a block a piece of countryside: a farm, a forest or a mine.
fn countryside(realm: &Realm, rng: &mut Rng, block: Rect, which: u64) {
    let family = *rng.pick(&FAMILIES).unwrap_or(&"Abara");
    let mut tiles = realm.tiles_mut();
    let (kind, name) = match which % 3 {
        0 => {
            tiles.fill(block, tile::FIELD);
            ("farm", format!("{family} Farm"))
        }
        1 => {
            scatter(&mut tiles, rng, block, tile::TREES, 600);
            ("forest", format!("{family} Wood"))
        }
        _ => {
            scatter(&mut tiles, rng, block, tile::ROCK, 350);
            let pit = Rect::new(block.x0 + 6, block.y0 + 6, block.x1 - 6, block.y1 - 6);
            tiles.fill(pit, tile::GRASS);
            ("mine", format!("{family} Mine"))
        }
    };
    drop(tiles);
    add_site(realm, kind, name, block);
}

/// Writes the tiles and the sites of the zone of `realm`. `number` is the
/// place of the zone in its city, from 0.
pub fn generate(realm: &Realm, number: u32) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let b = info.bounds;
    let name = info.name.clone();
    let mut rng = realm.rng().fork();
    let district = DISTRICTS[number as usize % DISTRICTS.len()];
    let c = b.centre();
    {
        let mut tiles = realm.tiles_mut();
        // Streets along the four sides, an avenue cross, pavements beside.
        for (rect, kind) in [
            (Rect::new(b.x0, b.y0, b.x1, b.y0 + 3), tile::PAVEMENT),
            (Rect::new(b.x0, b.y1 - 3, b.x1, b.y1), tile::PAVEMENT),
            (Rect::new(b.x0, b.y0, b.x0 + 3, b.y1), tile::PAVEMENT),
            (Rect::new(b.x1 - 3, b.y0, b.x1, b.y1), tile::PAVEMENT),
            (Rect::new(c.x - 3, b.y0, c.x + 3, b.y1), tile::PAVEMENT),
            (Rect::new(b.x0, c.y - 3, b.x1, c.y + 3), tile::PAVEMENT),
            (Rect::new(b.x0, b.y0, b.x1, b.y0 + 2), tile::ROAD),
            (Rect::new(b.x0, b.y1 - 2, b.x1, b.y1), tile::ROAD),
            (Rect::new(b.x0, b.y0, b.x0 + 2, b.y1), tile::ROAD),
            (Rect::new(b.x1 - 2, b.y0, b.x1, b.y1), tile::ROAD),
            (Rect::new(c.x - 2, b.y0, c.x + 2, b.y1), tile::ROAD),
            (Rect::new(b.x0, c.y - 2, b.x1, c.y + 2), tile::ROAD),
        ] {
            tiles.fill(rect, kind);
        }
    }
    // The four blocks between the streets.
    let blocks = [
        Rect::new(b.x0 + 4, b.y0 + 4, c.x - 4, c.y - 4),
        Rect::new(c.x + 4, b.y0 + 4, b.x1 - 4, c.y - 4),
        Rect::new(b.x0 + 4, c.y + 4, c.x - 4, b.y1 - 4),
        Rect::new(c.x + 4, c.y + 4, b.x1 - 4, b.y1 - 4),
    ];
    for (at, block) in blocks.into_iter().enumerate() {
        if block.width() < 12 || block.height() < 12 {
            continue;
        }
        if at == 0 {
            // Every district has a park where new residents appear.
            let site = park(realm, &mut rng, block, format!("{name} Park"));
            if let Some(site) = realm.sites_mut().get_mut(site) {
                site.add_tag(Site::SPAWN_TAG);
            }
        } else if district == "rural" {
            countryside(realm, &mut rng, block, at as u64 + u64::from(number));
        } else {
            let m = block.centre();
            for lot in [
                Rect::new(block.x0, block.y0, m.x, m.y),
                Rect::new(m.x, block.y0, block.x1, m.y),
                Rect::new(block.x0, m.y, m.x, block.y1),
                Rect::new(m.x, m.y, block.x1, block.y1),
            ] {
                building(realm, &mut rng, district, lot, c);
            }
        }
    }
}
