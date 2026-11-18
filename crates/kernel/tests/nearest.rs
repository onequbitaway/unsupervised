//! `Realm::nearest` and the `nearby` section of the observation find the
//! nearest residents, with ties in id order, and say how many more are in
//! sight (architecture.md, sections 5.3 and 5.6).

mod common;

use common::world;
use serde_json::Value;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Idx, Pos, RealmId, ResidentId, Rng, ZoneId};

/// Tiles of sight, and the length of the nearby list: the kernel defaults.
const SIGHT: u32 = 12;
const LIST: usize = 12;

/// Puts `count` residents at random tiles of the square from `(lo, lo)` to
/// `(hi, hi)`. Every tile coordinate is a multiple of `grain`, so a coarse
/// grain puts several residents on one tile and many at equal distances.
fn scatter(
    w: &mut TestWorld<'_>,
    rng: &mut Rng,
    count: usize,
    (lo, hi): (i64, i64),
    grain: i64,
) -> Vec<(ResidentId, Pos)> {
    (0..count)
        .map(|i| {
            let x = rng.range(lo / grain, hi / grain) * grain;
            let y = rng.range(lo / grain, hi / grain) * grain;
            let pos = Pos::new(x as i32, y as i32);
            (w.spawn_at(&format!("r{i}"), pos), pos)
        })
        .collect()
}

/// The slow, obviously correct answer: everybody but `skip` within `radius`
/// of `pos`, by distance and then by id.
fn in_sight(
    all: &[(ResidentId, Pos)],
    pos: Pos,
    radius: u32,
    skip: Option<ResidentId>,
) -> Vec<(u32, ResidentId)> {
    let mut seen: Vec<(u32, ResidentId)> = all
        .iter()
        .filter(|(id, at)| Some(*id) != skip && at.dist(pos) <= radius)
        .map(|(id, at)| (at.dist(pos), *id))
        .collect();
    seen.sort_unstable();
    seen
}

#[test]
fn nearest_is_nearby_in_order_of_distance_cut_to_k() {
    let mut w = world();
    let mut rng = Rng::seeded(31);
    // All four zones of the test world, also near their borders.
    let all = scatter(&mut w, &mut rng, 400, (0, 128), 2);
    let mut cut = 0;
    for _ in 0..800 {
        let zone = ZoneId(rng.below(4) as u32);
        let realm = w.zone(zone);
        // Places inside the zone and a little outside it.
        let bounds = realm.zone_info().expect("a zone").bounds;
        let pos = Pos::new(
            rng.range(i64::from(bounds.x0) - 6, i64::from(bounds.x1) + 6) as i32,
            rng.range(i64::from(bounds.y0) - 6, i64::from(bounds.y1) + 6) as i32,
        );
        let radius = rng.below(30) as u32;
        let k = rng.below(20) as usize;

        let mut expected: Vec<(u32, ResidentId, Idx)> = realm
            .nearby(pos, radius)
            .into_iter()
            .map(|idx| (realm.pos(idx).dist(pos), realm.id_of(idx), idx))
            .collect();
        expected.sort_unstable_by_key(|&(dist, id, _)| (dist, id));
        if expected.len() > k {
            cut += 1;
        }
        expected.truncate(k);
        let expected: Vec<Idx> = expected.into_iter().map(|(_, _, idx)| idx).collect();
        assert_eq!(
            realm.nearest(pos, radius, k),
            expected,
            "{zone}, {pos}, radius {radius}, k {k}"
        );
    }
    assert!(cut > 200, "many searches found more residents than they took: {cut}");

    // `nearest` finds only residents of its own zone, like `nearby`.
    let zone0 = w.zone(ZoneId(0));
    let everybody = zone0.nearest(Pos::new(64, 64), u32::MAX, usize::MAX);
    assert_eq!(everybody.len(), zone0.len());
    assert!(zone0.len() < all.len());
    assert_eq!(zone0.nearest(Pos::new(64, 64), u32::MAX, 0), []);
    // The global realm has no residents.
    assert_eq!(w.world.realm(RealmId::Global).unwrap().nearest(Pos::new(0, 0), 99, 5), []);
}

/// Checks the `nearby` section of every resident against the slow answer
/// and returns how many lists were cut.
fn check_nearby_sections(w: &TestWorld<'_>, all: &[(ResidentId, Pos)]) -> usize {
    let mut cut = 0;
    for &(id, pos) in all {
        let observation = w.observe(id);
        let nearby = &observation.sections["nearby"];
        let listed = nearby["residents"].as_array().expect("a list");
        let seen = in_sight(all, pos, SIGHT, Some(id));
        let expected: Vec<u64> = seen.iter().take(LIST).map(|&(_, other)| other.0).collect();
        let got: Vec<u64> =
            listed.iter().map(|entry| entry["id"].as_u64().expect("an id")).collect();
        assert_eq!(got, expected, "the list of resident {id} at {pos}");
        if seen.len() > LIST {
            cut += 1;
            assert_eq!(nearby["more_residents"], (seen.len() - LIST) as u64, "resident {id}");
        } else {
            assert_eq!(nearby.get("more_residents"), None, "resident {id}");
        }
        // Each entry shows the resident where it is. One of another zone
        // names its zone.
        let home = w.zone_of(id);
        for entry in listed {
            let other = ResidentId(entry["id"].as_u64().expect("an id"));
            let at = w.pos(other);
            assert_eq!(entry["pos"], serde_json::json!(at));
            let zone = w.zone_of(other);
            let expected_zone = if zone == home { None } else { Some(Value::from(zone.0)) };
            assert_eq!(entry.get("zone").cloned(), expected_zone, "{other} seen by {id}");
        }
    }
    cut
}

#[test]
fn the_nearby_section_lists_the_nearest_of_all_in_sight_also_across_borders() {
    let mut w = world();
    let mut rng = Rng::seeded(32);
    // About 15 residents in the sight of each: some lists are cut, some
    // are not. The tiles are coarse, so many distances are equal.
    let all = scatter(&mut w, &mut rng, 420, (0, 128), 3);
    // Two frames: each zone tells the zones around it who is near the
    // border, and they take the news.
    w.step(2);
    let cut = check_nearby_sections(&w, &all);
    assert!(cut > 100 && cut < all.len() - 50, "{cut} of {} lists were cut", all.len());
    // The same holds in a world that was restored from a snapshot.
    w.snapshot_restore();
    assert_eq!(check_nearby_sections(&w, &all), cut);
}

#[test]
fn a_crowd_on_one_tile_sees_its_lowest_ids() {
    let mut w = world();
    let spot = Pos::new(63, 40);
    // 60 residents on one tile next to a zone border, 5 more one tile away
    // on the other side of the border, and 1 far away.
    let mut all: Vec<(ResidentId, Pos)> =
        (0..60).map(|i| (w.spawn_at(&format!("c{i}"), spot), spot)).collect();
    let across = spot.offset(1, 0);
    all.extend((0..5).map(|i| (w.spawn_at(&format!("a{i}"), across), across)));
    all.push((w.spawn_at("far", Pos::new(10, 100)), Pos::new(10, 100)));
    w.step(2);
    assert_eq!((w.zone_of(all[0].0), w.zone_of(all[60].0)), (ZoneId(0), ZoneId(1)));
    assert_eq!(check_nearby_sections(&w, &all), 65);

    // The first of the crowd sees ids 2 to 13; the last sees 1 to 12.
    let ids = |r: ResidentId| -> Vec<u64> {
        w.observe(r).sections["nearby"]["residents"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_u64().unwrap())
            .collect()
    };
    assert_eq!(ids(all[0].0), (2..=13).collect::<Vec<u64>>());
    assert_eq!(ids(all[59].0), (1..=12).collect::<Vec<u64>>());
    // From across the border the 4 others on the tile come first, then
    // the crowd in id order.
    assert_eq!(ids(all[60].0), [62, 63, 64, 65, 1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(w.observe(all[60].0).sections["nearby"]["more_residents"], 64 - 12);
    assert_eq!(w.observe(all[65].0).sections["nearby"]["residents"], serde_json::json!([]));
}
