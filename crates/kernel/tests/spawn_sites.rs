//! Where new residents appear: the kernel keeps the list of spawn sites of
//! each city, and the list must always be what a scan of the sites gives.
//! The newcomers of a site take its walkable tiles in turn
//! (architecture.md, section 5.6).

mod common;

use std::collections::BTreeMap;

use common::{Demo, PLAZA, world};
use unsup_kernel::protocol::SpawnSpec;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityConfig, CityId, OwnerId, Pos, RealmId, Rect, ResidentId, Site, SiteId, World, WorldConfig,
    ZoneId,
};

/// The tile codes on which a resident can stand (game-design.md, section
/// 2): the default of `kernel.spawn_tiles`.
const WALKABLE: [u8; 6] = [0, 1, 2, 3, 6, 7];
const WALL: u8 = 4;
const WATER: u8 = 5;
const ROCK: u8 = 8;

/// The rule of the kernel, done the slow way: every site with the tag
/// `spawn` of the city, in zone order and then in site id order.
fn scan(world: &World, city: CityId) -> Vec<(ZoneId, Rect)> {
    let mut sites = Vec::new();
    for &zone in &world.layout().city(city).expect("the city exists").zones {
        if let Some(realm) = world.realm(RealmId::Zone(zone)) {
            sites.extend(realm.sites().with_tag(Site::SPAWN_TAG).map(|site| (zone, site.rect)));
        }
    }
    sites
}

/// The rule for the tile, done the slow way: the tiles of the site with a
/// code in `codes`, row by row, take turns. Turn 0 has the centre or the
/// next such tile after it. A site without such a tile gives its centre.
fn tile_of(world: &World, zone: ZoneId, site: Rect, turn: u64, codes: &[u8]) -> Pos {
    let realm = world.realm(RealmId::Zone(zone)).expect("the zone is hosted");
    let tiles = realm.tiles();
    let centre = site.centre();
    let mut free = Vec::new();
    for y in site.y0..site.y1 {
        for x in site.x0..site.x1 {
            let pos = Pos::new(x, y);
            if tiles.get(pos).is_some_and(|code| codes.contains(&code)) {
                free.push(pos);
            }
        }
    }
    if free.is_empty() {
        return centre;
    }
    let before = free.iter().filter(|pos| (pos.y, pos.x) < (centre.y, centre.x)).count();
    free[(before + (turn % free.len() as u64) as usize) % free.len()]
}

/// Where a newcomer appeared: the zone, the spawn site and the tile.
type Place = (ZoneId, Rect, Pos);

/// Spawns `count` residents in a city and checks that each is where the
/// rule puts it. Returns the places.
fn spawn_and_check(w: &mut TestWorld<'_>, city: CityId, count: usize) -> Vec<Place> {
    let mut places = Vec::new();
    for _ in 0..count {
        let sites = scan(&w.world, city);
        assert!(!sites.is_empty(), "the scenario has a spawn site");
        let spec = SpawnSpec { owner: OwnerId(1), name: "r".into(), model: None, city: Some(city) };
        let id = w.world.spawn(spec).expect("spawn");
        let number = sites.len() as u64;
        let (zone, site) = sites[(id.0 % number) as usize];
        let tile = tile_of(&w.world, zone, site, (id.0 - 1) / number, &WALKABLE);
        assert_eq!((w.zone_of(id), w.pos(id)), (zone, tile), "resident {id}");
        places.push((zone, site, tile));
    }
    places
}

fn shelter(w: &TestWorld<'_>, zone: u32, rect: Rect) -> SiteId {
    let realm = w.zone(ZoneId(zone));
    let id = SiteId(realm.alloc_id());
    realm.sites_mut().insert(Site::new(id, "shelter", "Shelter", rect).with_tag(Site::SPAWN_TAG))
}

/// Returns the number of different spawn sites among the places.
fn distinct(places: &[Place]) -> usize {
    let mut seen: Vec<(ZoneId, Rect)> =
        places.iter().map(|&(zone, site, _)| (zone, site)).collect();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

/// Returns the tiles of the places, in order.
fn tiles(places: &[Place]) -> Vec<Pos> {
    places.iter().map(|&(_, _, tile)| tile).collect()
}

/// Returns true when a place is at the site.
fn at_site(places: &[Place], zone: u32, site: Rect) -> bool {
    places.iter().any(|&(z, s, _)| (z, s) == (ZoneId(zone), site))
}

/// Spawns one resident in the first city and returns its position.
fn newcomer(w: &mut TestWorld<'_>) -> Pos {
    let id = w.spawn("newcomer");
    w.pos(id)
}

/// Makes the plaza of the demo world an ordinary site, so that a test has
/// only the spawn sites that it makes.
fn close_the_plaza(w: &TestWorld<'_>) {
    w.zone(ZoneId(0)).sites_mut().iter_mut().for_each(|site| site.tags.clear());
}

#[test]
fn residents_are_spread_over_the_spawn_sites_also_when_the_sites_change() {
    let mut w = world();
    let city = CityId(0);
    // One spawn site: the plaza. The first newcomer has its centre, the
    // next ones the tiles after it.
    let first = spawn_and_check(&mut w, city, 3);
    assert_eq!(distinct(&first), 1);
    assert!(at_site(&first, 0, PLAZA));
    assert_eq!(tiles(&first), [Pos::new(32, 32), Pos::new(33, 32), Pos::new(34, 32)]);

    // New spawn sites in other zones are used from the next spawn on.
    let east_rect = Rect::sized(70, 10, 6, 6);
    let south_a_rect = Rect::sized(100, 100, 4, 4);
    let south_b_rect = Rect::sized(110, 70, 8, 2);
    let east = shelter(&w, 1, east_rect);
    let south_a = shelter(&w, 3, south_a_rect);
    let south_b = shelter(&w, 3, south_b_rect);
    let places = spawn_and_check(&mut w, city, 8);
    assert_eq!(distinct(&places), 4, "{places:?}");
    assert_eq!(
        scan(&w.world, city),
        [
            (ZoneId(0), PLAZA),
            (ZoneId(1), east_rect),
            (ZoneId(3), south_a_rect),
            (ZoneId(3), south_b_rect)
        ]
    );

    // A change of a site that makes or moves no spawn site changes nothing.
    w.zone(ZoneId(1)).sites_mut().get_mut(east).expect("the site").add_tenant(ResidentId(1));
    w.zone(ZoneId(2)).sites_mut().insert(Site::new(
        SiteId(9),
        "shop",
        "Shop",
        Rect::sized(5, 70, 3, 3),
    ));
    assert_eq!(distinct(&spawn_and_check(&mut w, city, 8)), 4);

    // A site that loses the tag is not a spawn site any more.
    close_the_plaza(&w);
    let places = spawn_and_check(&mut w, city, 9);
    assert_eq!(distinct(&places), 3);
    assert!(!at_site(&places, 0, PLAZA));

    // A site that moves takes the residents with it.
    let moved = Rect::sized(80, 80, 2, 2);
    w.zone(ZoneId(3)).sites_mut().get_mut(south_a).expect("the site").rect = moved;
    let places = spawn_and_check(&mut w, city, 9);
    assert!(at_site(&places, 3, moved), "{places:?}");
    assert!(!at_site(&places, 3, south_a_rect));
    assert!(places.iter().all(|&(_, site, tile)| site.contains(tile)), "{places:?}");

    // A site that is removed is gone.
    w.zone(ZoneId(3)).sites_mut().remove(south_b);
    let places = spawn_and_check(&mut w, city, 6);
    assert_eq!(distinct(&places), 2);

    // Without a spawn site: the centre of the first zone of the city.
    w.zone(ZoneId(3)).sites_mut().remove(south_a);
    w.zone(ZoneId(1)).sites_mut().remove(east);
    assert_eq!(scan(&w.world, city), []);
    for _ in 0..2 {
        assert_eq!(newcomer(&mut w), Pos::new(32, 32));
    }
    w.assert_conserved();
}

#[test]
fn newcomers_take_the_walkable_tiles_of_a_site_in_turn() {
    let mut w = world();
    close_the_plaza(&w);
    // A yard of 5 x 3 tiles with a wall, water and rock in it. `o` is a
    // tile on which a resident can stand, `C` is the centre:
    //
    //   x  20 21 22 23 24
    //   y 70  o  #  o  o  o        # wall
    //     71  o  o  C  ~  o        ~ water
    //     72  ^  o  o  o  o        ^ rock
    let yard = Rect::sized(20, 70, 5, 3);
    shelter(&w, 2, yard);
    {
        let zone = w.zone(ZoneId(2));
        let mut map = zone.tiles_mut();
        // Six walkable codes in the yard, so that each is used.
        for (i, pos) in
            [(20, 70), (22, 70), (23, 70), (24, 70), (20, 71), (21, 71)].iter().enumerate()
        {
            assert!(map.set(Pos::new(pos.0, pos.1), WALKABLE[i]));
        }
        assert!(map.set(Pos::new(21, 70), WALL));
        assert!(map.set(Pos::new(23, 71), WATER));
        assert!(map.set(Pos::new(20, 72), ROCK));
    }
    // The centre first, then row by row, and from the last tile to the
    // first. No one appears on the wall, the water or the rock.
    let order = [
        (22, 71),
        (24, 71),
        (21, 72),
        (22, 72),
        (23, 72),
        (24, 72),
        (20, 70),
        (22, 70),
        (23, 70),
        (24, 70),
        (20, 71),
        (21, 71),
    ]
    .map(|(x, y)| Pos::new(x, y));
    let places = spawn_and_check(&mut w, CityId(0), 30);
    let expected: Vec<Pos> = (0..30).map(|turn| order[turn % 12]).collect();
    assert_eq!(tiles(&places), expected);

    // A crowd of newcomers is spread evenly: 600 residents on 12 tiles
    // are 50 on each.
    let more = spawn_and_check(&mut w, CityId(0), 570);
    let mut crowd: BTreeMap<Pos, usize> = BTreeMap::new();
    for tile in tiles(&places).into_iter().chain(tiles(&more)) {
        *crowd.entry(tile).or_default() += 1;
    }
    let mut sorted = order;
    sorted.sort_unstable();
    assert_eq!(crowd.keys().copied().collect::<Vec<_>>(), sorted);
    assert_eq!(crowd.values().copied().collect::<Vec<_>>(), [50; 12]);
    w.assert_conserved();
}

#[test]
fn a_site_without_a_walkable_tile_keeps_the_centre() {
    let mut w = world();
    close_the_plaza(&w);
    // All rock: every newcomer appears at the centre.
    let quarry = Rect::sized(10, 10, 4, 4);
    shelter(&w, 0, quarry);
    w.zone(ZoneId(0)).tiles_mut().fill(quarry, ROCK);
    for _ in 0..3 {
        assert_eq!(newcomer(&mut w), Pos::new(12, 12));
    }
    // One tile of floor in the rock: every newcomer appears on it.
    assert!(w.zone(ZoneId(0)).tiles_mut().set(Pos::new(10, 13), 3));
    for _ in 0..3 {
        assert_eq!(newcomer(&mut w), Pos::new(10, 13));
    }
}

#[test]
fn a_site_past_the_edge_of_its_zone_uses_only_its_tiles_in_the_zone() {
    let mut w = world();
    close_the_plaza(&w);
    // A site of zone 0 that goes 3 tiles past the east edge of the zone
    // (x = 64). Nobody appears in a zone that the site is not in. The
    // centre (63, 21) is in the zone.
    let gate = Rect::new(60, 20, 67, 23);
    shelter(&w, 0, gate);
    let places = spawn_and_check(&mut w, CityId(0), 24);
    let mut used = tiles(&places);
    assert_eq!(used[..3], [Pos::new(63, 21), Pos::new(60, 22), Pos::new(61, 22)]);
    used.sort_unstable();
    used.dedup();
    assert_eq!(used.len(), 12, "4 columns by 3 rows are in the zone");
    assert!(used.iter().all(|tile| tile.x < 64 && gate.contains(*tile)), "{used:?}");
    assert!(places.iter().all(|&(zone, _, _)| zone == ZoneId(0)));
}

#[test]
fn the_parameter_decides_on_which_tiles_newcomers_appear() {
    // The plaza of the demo world is pavement (code 2) in a zone of road
    // (code 1).
    let mut w = world();
    assert_eq!(w.param("kernel.spawn_tiles"), 0b1100_1111, "codes 0, 1, 2, 3, 6 and 7");
    assert_eq!(newcomer(&mut w), Pos::new(32, 32));
    // Only water: the plaza has none, so the centre is kept.
    w.set_param("kernel.spawn_tiles", 1 << WATER);
    w.step(1);
    for _ in 0..2 {
        assert_eq!(newcomer(&mut w), Pos::new(32, 32));
    }
    // Two tiles of water in the plaza take all newcomers in turn. The
    // turns of the site go on: the next newcomer is its fourth, and the
    // even turns have the tile after the centre.
    for pos in [Pos::new(29, 30), Pos::new(35, 35)] {
        assert!(w.zone(ZoneId(0)).tiles_mut().set(pos, WATER));
    }
    let seen: Vec<Pos> = (0..4).map(|_| newcomer(&mut w)).collect();
    assert_eq!(seen, [Pos::new(29, 30), Pos::new(35, 35), Pos::new(29, 30), Pos::new(35, 35)]);
    // A world that is made with the value in its data starts with it.
    let data = "[params.\"kernel.spawn_tiles\"]\nvalue = 0\n".to_string();
    let config = WorldConfig { data: vec![data], ..WorldConfig::default() };
    let mut none = TestWorld::with_config(&[&Demo], config);
    for _ in 0..3 {
        assert_eq!(newcomer(&mut none), Pos::new(32, 32));
    }
}

#[test]
fn each_city_has_its_own_spawn_sites_and_a_restored_world_the_same() {
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("Ashford", Pos::new(0, 0), 2),
            CityConfig::new("Brine", Pos::new(1_000, 1_000), 2),
        ],
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&[&Demo], config);
    shelter(&w, 2, Rect::sized(10, 100, 2, 2));
    shelter(&w, 5, Rect::sized(1_100, 1_010, 2, 2));
    shelter(&w, 6, Rect::sized(1_010, 1_100, 2, 2));
    let ashford = spawn_and_check(&mut w, CityId(0), 6);
    let brine = spawn_and_check(&mut w, CityId(1), 6);
    assert_eq!(distinct(&ashford), 2);
    assert_eq!(distinct(&brine), 2);
    assert!(brine.iter().all(|(zone, _, _)| zone.0 >= 4), "{brine:?}");

    // A world that is restored from a snapshot puts the next residents
    // where the first world puts them.
    let mut copy = TestWorld::restore(&[&Demo], &w.world.snapshot());
    for city in [CityId(1), CityId(0), CityId(1)] {
        assert_eq!(spawn_and_check(&mut copy, city, 3), spawn_and_check(&mut w, city, 3));
    }
    assert_eq!(copy.hash(), w.hash());
}

#[test]
fn spawn_sites_of_zones_in_another_process_do_not_count() {
    // This process hosts zones 0 and 1. The plaza of zone 0 is its only
    // spawn site, whatever zone 3 has in the other process.
    let hosted = vec![RealmId::Global, RealmId::Zone(ZoneId(0)), RealmId::Zone(ZoneId(1))];
    let config = WorldConfig { hosted: Some(hosted), ..WorldConfig::default() };
    let mut w = TestWorld::with_config(&[&Demo], config);
    let first = spawn_and_check(&mut w, CityId(0), 2);
    assert_eq!(distinct(&first), 1);
    assert!(at_site(&first, 0, PLAZA));
    shelter(&w, 1, Rect::sized(70, 10, 6, 6));
    assert_eq!(distinct(&spawn_and_check(&mut w, CityId(0), 4)), 2);
}
