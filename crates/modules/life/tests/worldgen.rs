//! World generation: districts, roads, sites and tiles.

mod common;

use std::collections::BTreeSet;

use common::{all_sites, cities, tile_at, walkable, world, world_with};
use unsup_kernel::protocol::CityId;
use unsup_kernel::{Holder, OrgId, Pos, RealmId, WorldConfig, ZoneId};
use unsup_life::names::{district, key, site, tag, tile};
use unsup_life::{SiteIndex, ZoneFacts};

/// The kinds of site that are buildings: walls with a door.
const OPEN_LAND: [&str; 4] = [site::FARM, site::MINE, site::FOREST, site::PARK];

/// The kinds of site that are never sold.
const PUBLIC: [&str; 12] = [
    site::SHELTER,
    site::BANK,
    site::EXCHANGE,
    site::CITY_HALL,
    site::POLICE_STATION,
    site::COURTHOUSE,
    site::JAIL,
    site::HOSPITAL,
    site::SCHOOL,
    site::STATION,
    site::PARK,
    site::DEPOT,
];

fn districts_of(w: &unsup_kernel::testing::TestWorld<'_>) -> Vec<String> {
    w.world
        .layout()
        .zones()
        .iter()
        .map(|zone| w.zone(zone.id).res::<ZoneFacts>().district.clone())
        .collect()
}

#[test]
fn the_same_seed_gives_the_same_world_and_another_seed_gives_another() {
    let first = world_with(WorldConfig::with_seed(11));
    let second = world_with(WorldConfig::with_seed(11));
    let other = world_with(WorldConfig::with_seed(12));
    assert_eq!(first.hash(), second.hash());
    assert_ne!(first.hash(), other.hash());
    for zone in first.world.layout().zones() {
        let (a, b) = (first.zone(zone.id), second.zone(zone.id));
        assert_eq!(a.tiles().as_slice(), b.tiles().as_slice());
        assert_eq!(*a.sites(), *b.sites());
        assert!(!a.sites().is_empty());
    }
}

#[test]
fn every_city_has_every_kind_of_site() {
    // Cities of one, four and nine zones.
    let w = world_with(cities(5, &[1, 2, 3]));
    let layout = w.world.layout();
    assert_eq!(layout.cities().len(), 3);
    for city in layout.cities() {
        let kinds: BTreeSet<String> = all_sites(&w)
            .into_iter()
            .filter(|(zone, _)| city.zones.contains(zone))
            .map(|(_, site)| site.kind)
            .collect();
        let expected: BTreeSet<String> = site::ALL.iter().map(|kind| (*kind).to_string()).collect();
        assert_eq!(kinds, expected, "city {}", city.name);
    }
}

#[test]
fn a_city_with_one_zone_is_mixed_and_a_large_city_has_every_district_type() {
    let w = world_with(cities(3, &[1, 2, 3]));
    let names = districts_of(&w);
    assert_eq!(names[0], district::MIXED);

    // Four zones: the centre, homes, food and work. No commercial district.
    let mut four: Vec<&str> = names[1..5].iter().map(String::as_str).collect();
    four.sort_unstable();
    assert_eq!(
        four,
        [district::DOWNTOWN, district::INDUSTRIAL, district::RESIDENTIAL, district::RURAL]
    );

    // Nine zones: every type, and downtown in the centre zone.
    let nine: BTreeSet<&str> = names[5..14].iter().map(String::as_str).collect();
    let all: BTreeSet<&str> = [
        district::DOWNTOWN,
        district::RESIDENTIAL,
        district::COMMERCIAL,
        district::INDUSTRIAL,
        district::RURAL,
    ]
    .into();
    assert_eq!(nine, all);
    assert_eq!(names[5 + 4], district::DOWNTOWN);
}

#[test]
fn every_site_starts_with_the_city_as_owner_a_value_and_its_tags() {
    let w = world_with(cities(8, &[1, 2]));
    let layout = w.world.layout();
    let sites = all_sites(&w);
    assert!(sites.len() > 60, "{} sites", sites.len());
    for (zone, place) in &sites {
        let city = layout.zone(*zone).expect("the zone exists").city;
        assert_eq!(place.owner, Some(Holder::Org(OrgId::city(city))), "{}", place.name);
        assert!(place.attr(key::VALUE) > 0, "{} has no value", place.name);
        assert!(place.tenants.is_empty());
        assert_eq!(
            place.has_tag(tag::PUBLIC),
            PUBLIC.contains(&place.kind.as_str()),
            "{}",
            place.name
        );
        let spawn = place.kind == site::SHELTER || place.kind == site::STATION;
        assert_eq!(place.has_tag(tag::SPAWN), spawn, "{}", place.name);
        let beds = match place.kind.as_str() {
            site::HOME => 2,
            site::APARTMENT => 8,
            site::SHELTER => 24,
            _ => 0,
        };
        assert_eq!(place.attr(key::BEDS), beds, "{}", place.name);
    }
    // The value of a kind comes from the data file, give or take 15 percent.
    let home = sites.iter().find(|(_, place)| place.kind == site::HOME).expect("a home");
    let value = home.1.attr(key::VALUE);
    assert!((3_400_000..=4_600_000).contains(&value), "{value}");
}

#[test]
fn buildings_have_walls_a_door_and_floor_and_open_land_has_no_wall() {
    let w = world();
    let sites = all_sites(&w);
    let mut buildings = 0;
    for (_, place) in &sites {
        let r = place.rect;
        let ring: Vec<Pos> = (r.x0..r.x1)
            .flat_map(|x| [Pos::new(x, r.y0), Pos::new(x, r.y1 - 1)])
            .chain((r.y0 + 1..r.y1 - 1).flat_map(|y| [Pos::new(r.x0, y), Pos::new(r.x1 - 1, y)]))
            .collect();
        let count = |code: u8| ring.iter().filter(|pos| tile_at(&w, **pos) == code).count();
        if OPEN_LAND.contains(&place.kind.as_str()) {
            assert_eq!(count(tile::WALL), 0, "{}", place.name);
            continue;
        }
        buildings += 1;
        // The ring is wall, but for one or two doors. A door is floor.
        let doors = count(tile::FLOOR);
        assert!((1..=2).contains(&doors), "{} has {doors} doors", place.name);
        assert_eq!(count(tile::WALL) + doors, ring.len(), "{}", place.name);
        // Every tile inside is floor.
        for y in r.y0 + 1..r.y1 - 1 {
            for x in r.x0 + 1..r.x1 - 1 {
                assert_eq!(tile_at(&w, Pos::new(x, y)), tile::FLOOR, "{}", place.name);
            }
        }
        // The tile in the middle is where a newcomer appears.
        assert!(walkable(&w, r.centre()), "{}", place.name);
    }
    assert!(buildings > 30, "{buildings} buildings");

    let land = |kind: &str| {
        let (_, place) =
            sites.iter().find(|(_, place)| place.kind == kind).expect("the kind exists");
        let r = place.rect;
        let mut codes = BTreeSet::new();
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                codes.insert(tile_at(&w, Pos::new(x, y)));
            }
        }
        codes
    };
    assert_eq!(land(site::FARM), BTreeSet::from([tile::FIELD]));
    assert_eq!(land(site::FOREST), BTreeSet::from([tile::TREES]));
    assert!(land(site::MINE).contains(&tile::ROCK));
    assert!(land(site::PARK).contains(&tile::WATER));
}

#[test]
fn the_edge_of_every_zone_is_road_and_sites_do_not_overlap() {
    let w = world_with(cities(2, &[2]));
    for zone in w.world.layout().zones() {
        let b = zone.bounds;
        for i in 0..64 {
            for pos in [
                Pos::new(b.x0 + i, b.y0),
                Pos::new(b.x0 + i, b.y1 - 1),
                Pos::new(b.x0, b.y0 + i),
                Pos::new(b.x1 - 1, b.y0 + i),
            ] {
                assert_eq!(tile_at(&w, pos), tile::ROAD, "{pos}");
            }
        }
        let realm = w.zone(zone.id);
        let sites = realm.sites();
        let list: Vec<_> = sites.iter().collect();
        for (i, a) in list.iter().enumerate() {
            assert!(
                a.rect.x0 >= b.x0 && a.rect.x1 <= b.x1 && a.rect.y0 >= b.y0 && a.rect.y1 <= b.y1
            );
            for other in &list[i + 1..] {
                assert!(!a.rect.intersects(&other.rect), "{} and {}", a.name, other.name);
            }
        }
        // No road tile lies inside a site.
        for place in &list {
            for y in place.rect.y0..place.rect.y1 {
                for x in place.rect.x0..place.rect.x1 {
                    assert_ne!(tile_at(&w, Pos::new(x, y)), tile::ROAD, "{}", place.name);
                }
            }
        }
    }
}

#[test]
fn newcomers_appear_inside_a_shelter_or_a_station() {
    let mut w = world();
    let mut places = BTreeSet::new();
    for i in 0..6 {
        let id = w.spawn(&format!("r{i}"));
        let pos = w.pos(id);
        let zone = w.zone_of(id);
        let realm = w.zone(zone);
        let sites = realm.sites();
        let place = sites.at(pos).expect("a newcomer stands in a site");
        assert!(place.has_tag(tag::SPAWN));
        assert_eq!(tile_at(&w, pos), tile::FLOOR);
        places.insert(place.id);
    }
    // The default city has more than one spawn site, and newcomers spread
    // over them.
    assert!(places.len() > 1);
}

#[test]
fn the_global_realm_knows_the_place_of_every_site() {
    let mut w = world_with(cities(4, &[2, 1]));
    // The zones tell the global realm in the first frame.
    w.step(1);
    let sites = all_sites(&w);
    let global = w.world.realm(RealmId::Global).expect("the global realm");
    let index = global.res::<SiteIndex>();
    assert_eq!(index.len(), sites.len());
    let layout = w.world.layout();
    for (zone, place) in &sites {
        let found = index.get(place.id).expect("the site is in the index");
        assert_eq!(found.zone, *zone);
        assert_eq!(Some(found.city), layout.zone(*zone).map(|info| info.city));
        assert!(place.rect.contains(found.tile));
        assert!(walkable(&w, found.tile));
    }
    assert_eq!(layout.zone(ZoneId(4)).map(|info| info.city), Some(CityId(1)));
}

#[test]
fn a_small_zone_still_gets_roads_and_what_fits() {
    let mut config = WorldConfig::with_seed(1);
    config.zone_size = 16;
    let w = world_with(config);
    for zone in w.world.layout().zones() {
        let b = zone.bounds;
        assert_eq!(tile_at(&w, Pos::new(b.x0, b.y0)), tile::ROAD);
        assert_eq!(tile_at(&w, Pos::new(b.x1 - 1, b.y1 - 1)), tile::ROAD);
        // One block of 14 by 14 tiles: at most four small lots.
        let count = w.zone(zone.id).sites().len();
        assert!((1..=4).contains(&count), "{count} sites");
    }
    // The first kinds that a city needs are there.
    let kinds: BTreeSet<String> = all_sites(&w).into_iter().map(|(_, site)| site.kind).collect();
    assert!(kinds.contains(site::SHELTER));
    assert!(kinds.contains(site::STATION));
}

#[test]
fn every_tile_where_a_resident_can_stand_can_be_reached_on_foot() {
    // A walk moves in eight directions and does not cut the corner of a
    // tile that is not walkable. Flood every zone from its north-west
    // corner, which is a road, with the same rule.
    const DIRS: [(i32, i32); 8] =
        [(0, -1), (1, 0), (0, 1), (-1, 0), (1, -1), (1, 1), (-1, 1), (-1, -1)];
    let mut zones = 0;
    for seed in [1, 2, 3] {
        let w = world_with(cities(seed, &[1, 3]));
        for zone in w.world.layout().zones() {
            let realm = w.zone(zone.id);
            let tiles = realm.tiles();
            let open = |pos: Pos| tiles.get(pos).is_some_and(tile::is_walkable);
            let start = Pos::new(zone.bounds.x0, zone.bounds.y0);
            assert!(open(start));
            let mut reached = BTreeSet::from([(start.x, start.y)]);
            let mut next = vec![start];
            while let Some(here) = next.pop() {
                for (dx, dy) in DIRS {
                    let there = Pos::new(here.x + dx, here.y + dy);
                    let corner_free = dx == 0
                        || dy == 0
                        || (open(Pos::new(there.x, here.y)) && open(Pos::new(here.x, there.y)));
                    if open(there) && corner_free && reached.insert((there.x, there.y)) {
                        next.push(there);
                    }
                }
            }
            let b = zone.bounds;
            let walkable = (b.y0..b.y1)
                .flat_map(|y| (b.x0..b.x1).map(move |x| Pos::new(x, y)))
                .filter(|pos| open(*pos))
                .count();
            assert!(walkable > 2_000, "zone {} has {walkable} walkable tiles", zone.id);
            assert_eq!(reached.len(), walkable, "zone {} of seed {seed}", zone.id);
            zones += 1;
        }
    }
    assert_eq!(zones, 30);
}
