//! Where new residents appear: the kernel keeps the list of spawn sites of
//! each city, and the list must always be what a scan of the sites gives
//! (architecture.md, section 5.6).

mod common;

use common::{Demo, PLAZA, world};
use unsup_kernel::protocol::SpawnSpec;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{
    CityConfig, CityId, OwnerId, Pos, RealmId, Rect, ResidentId, Site, SiteId, World, WorldConfig,
    ZoneId,
};

/// The rule of the kernel, done the slow way: every site with the tag
/// `spawn` of the city, in zone order and then in site id order.
fn scan(world: &World, city: CityId) -> Vec<(ZoneId, Pos)> {
    let mut points = Vec::new();
    for &zone in &world.layout().city(city).expect("the city exists").zones {
        if let Some(realm) = world.realm(RealmId::Zone(zone)) {
            points.extend(
                realm.sites().with_tag(Site::SPAWN_TAG).map(|site| (zone, site.rect.centre())),
            );
        }
    }
    points
}

/// Spawns `count` residents in a city and checks that each is where the
/// rule puts it. Returns the places.
fn spawn_and_check(w: &mut TestWorld<'_>, city: CityId, count: usize) -> Vec<(ZoneId, Pos)> {
    let mut places = Vec::new();
    for _ in 0..count {
        let points = scan(&w.world, city);
        assert!(!points.is_empty(), "the scenario has a spawn site");
        let spec = SpawnSpec { owner: OwnerId(1), name: "r".into(), model: None, city: Some(city) };
        let id = w.world.spawn(spec).expect("spawn");
        let place = (w.zone_of(id), w.pos(id));
        assert_eq!(place, points[(id.0 % points.len() as u64) as usize], "resident {id}");
        places.push(place);
    }
    places
}

fn shelter(w: &TestWorld<'_>, zone: u32, rect: Rect) -> SiteId {
    let realm = w.zone(ZoneId(zone));
    let id = SiteId(realm.alloc_id());
    realm.sites_mut().insert(Site::new(id, "shelter", "Shelter", rect).with_tag(Site::SPAWN_TAG))
}

fn distinct(places: &[(ZoneId, Pos)]) -> usize {
    let mut seen = places.to_vec();
    seen.sort_unstable();
    seen.dedup();
    seen.len()
}

#[test]
fn residents_are_spread_over_the_spawn_sites_also_when_the_sites_change() {
    let mut w = world();
    let city = CityId(0);
    let plaza = (ZoneId(0), PLAZA.centre());
    assert_eq!(spawn_and_check(&mut w, city, 3), [plaza; 3]);

    // New spawn sites in other zones are used from the next spawn on.
    let east = shelter(&w, 1, Rect::sized(70, 10, 6, 6));
    let south_a = shelter(&w, 3, Rect::sized(100, 100, 4, 4));
    let south_b = shelter(&w, 3, Rect::sized(110, 70, 8, 2));
    let places = spawn_and_check(&mut w, city, 8);
    assert_eq!(distinct(&places), 4, "{places:?}");
    assert_eq!(
        scan(&w.world, city),
        [
            plaza,
            (ZoneId(1), Pos::new(73, 13)),
            (ZoneId(3), Pos::new(102, 102)),
            (ZoneId(3), Pos::new(114, 71))
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
    w.zone(ZoneId(0)).sites_mut().iter_mut().for_each(|site| site.tags.clear());
    let places = spawn_and_check(&mut w, city, 9);
    assert_eq!(distinct(&places), 3);
    assert!(!places.contains(&plaza));

    // A site that moves takes the residents with it.
    w.zone(ZoneId(3)).sites_mut().get_mut(south_a).expect("the site").rect =
        Rect::sized(80, 80, 2, 2);
    let places = spawn_and_check(&mut w, city, 9);
    assert!(places.contains(&(ZoneId(3), Pos::new(81, 81))), "{places:?}");
    assert!(!places.contains(&(ZoneId(3), Pos::new(102, 102))));

    // A site that is removed is gone.
    w.zone(ZoneId(3)).sites_mut().remove(south_b);
    let places = spawn_and_check(&mut w, city, 6);
    assert_eq!(distinct(&places), 2);

    // Without a spawn site: the centre of the first zone of the city.
    w.zone(ZoneId(3)).sites_mut().remove(south_a);
    w.zone(ZoneId(1)).sites_mut().remove(east);
    assert_eq!(scan(&w.world, city), []);
    let spec = SpawnSpec { owner: OwnerId(1), name: "last".into(), model: None, city: Some(city) };
    let last = w.world.spawn(spec).expect("spawn");
    assert_eq!((w.zone_of(last), w.pos(last)), (ZoneId(0), Pos::new(32, 32)));
    w.assert_conserved();
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
    assert!(brine.iter().all(|(zone, _)| zone.0 >= 4), "{brine:?}");

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
    assert_eq!(spawn_and_check(&mut w, CityId(0), 2), [(ZoneId(0), PLAZA.centre()); 2]);
    shelter(&w, 1, Rect::sized(70, 10, 6, 6));
    assert_eq!(distinct(&spawn_and_check(&mut w, CityId(0), 4)), 2);
}
