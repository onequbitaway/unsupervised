//! Sight across a zone border: `Realm::nearby_outside`, the `nearby`
//! section of an observation and `Audience::Nearby` (architecture.md,
//! sections 5.3 and 5.5).

mod common;

use common::{Demo, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionRequest, OwnerId, SpawnSpec};
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{
    CityConfig, CityId, Envelope, Input, Outsider, Pos, RealmId, ResidentId, World, WorldConfig,
    ZoneId,
};

/// What a zone knows of the residents of other zones near a position.
fn outside(w: &TestWorld<'_>, zone: u32, pos: Pos, radius: u32) -> Vec<Outsider> {
    w.zone(ZoneId(zone)).nearby_outside(pos, radius)
}

/// The residents in the `nearby` section of a resident.
fn seen(w: &TestWorld<'_>, r: ResidentId) -> Value {
    w.observe(r).sections["nearby"]["residents"].clone()
}

/// The ids in the `nearby` section of a resident, nearest first.
fn seen_ids(w: &TestWorld<'_>, r: ResidentId) -> Vec<u64> {
    seen(w, r)
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["id"].as_u64().expect("an id"))
        .collect()
}

/// The texts of the shouts that a resident heard, oldest first.
fn shouts(w: &TestWorld<'_>, r: ResidentId) -> Vec<String> {
    w.notices(r)
        .iter()
        .filter(|notice| notice.kind == "demo.shout")
        .map(|notice| notice.data["text"].as_str().expect("text").to_string())
        .collect()
}

#[test]
fn residents_on_both_sides_of_a_border_see_each_other() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(64, 30));
    assert_eq!((w.zone_of(ada), w.zone_of(bea)), (ZoneId(0), ZoneId(1)));
    w.act_ok(bea, "work", json!({ "ticks": 30 }));
    w.step(2);

    // One tile apart, in two zones. Each sees the other, with the zone.
    assert_eq!(
        seen(&w, ada),
        json!([{ "id": bea, "name": "Bea", "pos": { "x": 64, "y": 30 }, "activity": "work", "zone": 1 }])
    );
    assert_eq!(
        seen(&w, bea),
        json!([{ "id": ada, "name": "Ada", "pos": { "x": 63, "y": 30 }, "zone": 0 }])
    );
    // Module code reads the same copy.
    assert_eq!(
        outside(&w, 0, Pos::new(63, 30), 12),
        [Outsider {
            id: bea,
            zone: ZoneId(1),
            pos: Pos::new(64, 30),
            name: "Bea".into(),
            activity: Some("work"),
        }]
    );
    assert_eq!(
        outside(&w, 1, Pos::new(64, 30), 1),
        [Outsider {
            id: ada,
            zone: ZoneId(0),
            pos: Pos::new(63, 30),
            name: "Ada".into(),
            activity: None
        }]
    );
    // `nearby` still finds only the rows of the zone.
    let zone0 = w.zone(ZoneId(0));
    let here: Vec<ResidentId> =
        zone0.nearby(Pos::new(63, 30), 12).into_iter().map(|idx| zone0.id_of(idx)).collect();
    assert_eq!(here, [ada]);
    // The zones that are not near know nothing of the two.
    assert_eq!(outside(&w, 2, Pos::new(63, 64), 64), []);
    assert_eq!(outside(&w, 3, Pos::new(64, 64), 64), []);
    w.assert_conserved();
}

#[test]
fn residents_at_a_corner_see_each_other() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 63));
    let dia = w.spawn_at("Dia", Pos::new(64, 64));
    assert_eq!((w.zone_of(ada), w.zone_of(dia)), (ZoneId(0), ZoneId(3)));
    w.step(2);
    assert_eq!(
        seen(&w, ada),
        json!([{ "id": dia, "name": "Dia", "pos": { "x": 64, "y": 64 }, "zone": 3 }])
    );
    assert_eq!(
        seen(&w, dia),
        json!([{ "id": ada, "name": "Ada", "pos": { "x": 63, "y": 63 }, "zone": 0 }])
    );
    // The two other zones at the corner know both, ordered by id.
    for zone in [1, 2] {
        let known: Vec<(ResidentId, ZoneId)> = outside(&w, zone, Pos::new(64, 63), 2)
            .into_iter()
            .map(|outsider| (outsider.id, outsider.zone))
            .collect();
        assert_eq!(known, [(ada, ZoneId(0)), (dia, ZoneId(3))], "zone {zone}");
    }
}

#[test]
fn the_copy_is_one_frame_old() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(70, 30));
    w.step(2);
    let copy = |w: &TestWorld<'_>| {
        let known = outside(w, 0, Pos::new(63, 30), 12);
        assert_eq!(known.len(), 1);
        (known[0].pos, known[0].activity)
    };
    assert_eq!(copy(&w), (Pos::new(70, 30), None));

    // Bea walks one tile in frame 3. Zone 0 hears of it in frame 4.
    w.act_ok(bea, "step", json!({ "dx": 1, "dy": 0 }));
    w.step(1);
    assert_eq!(w.pos(bea), Pos::new(71, 30));
    assert_eq!(copy(&w), (Pos::new(70, 30), None), "the copy is as old as the last frame");
    w.step(1);
    assert_eq!(copy(&w), (Pos::new(71, 30), None));

    // The same for the activity: when it starts, and when it ends.
    w.act_ok(bea, "work", json!({ "ticks": 3 }));
    w.step(1);
    assert_eq!(w.activity(bea), Some("work"));
    assert_eq!(copy(&w), (Pos::new(71, 30), None));
    w.step(1);
    assert_eq!(copy(&w), (Pos::new(71, 30), Some("work")));
    assert_eq!(seen(&w, ada)[0]["activity"], json!("work"));
    w.step(1);
    assert_eq!(w.activity(bea), None, "the work ended in this frame");
    assert_eq!(copy(&w), (Pos::new(71, 30), Some("work")));
    w.step(1);
    assert_eq!(copy(&w), (Pos::new(71, 30), None));
    assert!(seen(&w, ada)[0].get("activity").is_none());
}

#[test]
fn a_zone_knows_only_the_residents_within_the_sight_radius_of_its_border() {
    let mut w = world();
    let _ada = w.spawn_at("Ada", Pos::new(63, 30));
    // 12 tiles from the last column of zone 0: in sight. 17 tiles: not.
    let bea = w.spawn_at("Bea", Pos::new(75, 30));
    let _cyd = w.spawn_at("Cyd", Pos::new(80, 30));
    assert_eq!(w.param("kernel.sight_radius"), 12);
    w.step(2);
    let known = |w: &TestWorld<'_>, radius: u32| -> Vec<ResidentId> {
        outside(w, 0, Pos::new(63, 30), radius).into_iter().map(|outsider| outsider.id).collect()
    };
    assert_eq!(known(&w, 64), [bea], "Cyd is 17 tiles from zone 0");
    // The radius of the question counts too.
    assert_eq!(known(&w, 12), [bea]);
    assert_eq!(known(&w, 11), []);

    // Bea walks one tile away: out of the band, and out of the copy one
    // frame later.
    w.act_ok(bea, "step", json!({ "dx": 1, "dy": 0 }));
    w.step(1);
    assert_eq!(known(&w, 64), [bea]);
    w.step(1);
    assert_eq!(known(&w, 64), []);
    // She walks back: the copy has her again, with her name.
    w.act_ok(bea, "step", json!({ "dx": -1, "dy": 0 }));
    w.step(2);
    assert_eq!(
        outside(&w, 0, Pos::new(63, 30), 64),
        [Outsider {
            id: bea,
            zone: ZoneId(1),
            pos: Pos::new(75, 30),
            name: "Bea".into(),
            activity: None
        }]
    );
}

#[test]
fn a_nearby_audience_reaches_across_a_zone_border() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let same_zone = w.spawn_at("Same", Pos::new(60, 30));
    let across = w.spawn_at("Across", Pos::new(64, 30));
    let far = w.spawn_at("Far", Pos::new(69, 30));
    assert_eq!((w.zone_of(across), w.zone_of(far)), (ZoneId(1), ZoneId(1)));
    w.step(2);

    w.act_ok(ada, "shout", json!({ "text": "hello" }));
    // In the zone of the event the notice is there at once. It crosses the
    // border with the next frame.
    assert_eq!(w.notices(same_zone).len(), 1);
    assert!(w.notices(across).is_empty());
    let frames = w.step(1);
    let notices = w.notices(across);
    assert_eq!(notices.len(), 1, "one tile away, in another zone");
    assert_eq!(
        (notices[0].kind.as_str(), notices[0].from, notices[0].data.clone()),
        ("demo.shout", Some(ada), json!({ "untrusted": true, "from": ada, "text": "hello" }))
    );
    assert!(w.notices(far).is_empty(), "6 tiles is outside the radius, also across a border");
    assert!(w.notices(ada).is_empty(), "the actor does not get its own event");
    assert_eq!(frames[0].woke, [same_zone, across]);

    // The other direction: both residents of zone 0 are within 5 tiles.
    w.act_ok(across, "shout", json!({ "text": "hello back" }));
    w.step(1);
    assert_eq!(shouts(&w, ada), ["hello back"]);
    assert_eq!(shouts(&w, same_zone), ["hello", "hello back"]);
    assert_eq!(shouts(&w, far), ["hello back"], "Far is 5 tiles from Across, in the same zone");
    // Nobody gets a notice twice.
    w.step(3);
    assert_eq!(shouts(&w, across), ["hello"]);
    assert_eq!(shouts(&w, ada), ["hello back"]);
    assert_eq!(w.world.dropped_messages(), 0);
    w.assert_conserved();
}

#[test]
fn a_resident_that_walks_across_a_border_stays_in_sight_and_hears_each_event_once() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(62, 30));
    let eve = w.spawn_at("Eve", Pos::new(61, 30));
    w.step(1);
    // Eve walks east, one tile per frame, from column 61 to column 68. She
    // crosses the border in the third frame. Ada shouts before each step;
    // Eve is at most 5 tiles away each time.
    for frame in 0..7 {
        w.act_ok(ada, "shout", json!({ "text": frame.to_string() }));
        w.act_ok(eve, "step", json!({ "dx": 1, "dy": 0 }));
        w.step(1);
        assert_eq!(w.pos(eve), Pos::new(62 + frame, 30));
        // Each sees the other in every frame, once.
        assert_eq!(seen_ids(&w, ada), [eve.0], "after frame {frame}");
        assert_eq!(seen_ids(&w, eve), [ada.0], "after frame {frame}");
    }
    assert_eq!(w.zone_of(eve), ZoneId(1));
    w.step(2);
    assert_eq!(shouts(&w, eve), ["0", "1", "2", "3", "4", "5", "6"]);
    assert!(shouts(&w, ada).is_empty());
    // Zone 0 has no row of Eve and one copy of her.
    assert_eq!(w.zone(ZoneId(0)).idx_of(eve), None);
    assert_eq!(
        outside(&w, 0, Pos::new(62, 30), 12),
        [Outsider {
            id: eve,
            zone: ZoneId(1),
            pos: Pos::new(68, 30),
            name: "Eve".into(),
            activity: None
        }]
    );
    // Eve walks back into zone 0: she is a row there again, and no copy.
    for _ in 0..5 {
        w.act_ok(eve, "step", json!({ "dx": -1, "dy": 0 }));
        w.step(1);
        assert_eq!(seen_ids(&w, ada), [eve.0]);
    }
    assert_eq!((w.zone_of(eve), w.pos(eve)), (ZoneId(0), Pos::new(63, 30)));
    w.step(1);
    assert_eq!(outside(&w, 0, Pos::new(62, 30), 64), []);
    assert_eq!(outside(&w, 1, Pos::new(64, 30), 64).len(), 2, "zone 1 knows Ada and Eve");
    w.assert_conserved();
}

#[test]
fn a_resident_that_crosses_and_goes_far_leaves_the_copy_of_the_zone_it_left() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(62, 30));
    let eve = w.spawn_at("Eve", Pos::new(63, 30));
    w.step(1);
    w.act_ok(eve, "step", json!({ "dx": 1, "dy": 0 }));
    w.step(1);
    // Eve is in zone 1 now. Zone 0 kept a copy of her when she left.
    assert_eq!(w.zone_of(eve), ZoneId(1));
    assert_eq!(seen_ids(&w, ada), [eve.0]);
    // She goes far from the border at once. Zone 1 never told zone 0 of
    // her, but it knows that zone 0 has the copy, and takes it away.
    w.act_ok(eve, "jump", json!({ "zone": 1, "x": 100, "y": 30 }));
    w.step(2);
    assert_eq!(seen_ids(&w, ada), [] as [u64; 0]);
    assert_eq!(outside(&w, 0, Pos::new(62, 30), 200), []);
    // The same when she leaves the world right after the border.
    let fay = w.spawn_at("Fay", Pos::new(63, 31));
    w.step(1);
    w.act_ok(fay, "step", json!({ "dx": 1, "dy": 0 }));
    w.step(1);
    assert_eq!(seen_ids(&w, ada), [fay.0]);
    w.act_ok(fay, "leave", json!({}));
    w.step(1);
    assert_eq!(seen_ids(&w, ada), [] as [u64; 0]);
}

#[test]
fn a_resident_that_leaves_the_world_leaves_the_copies() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(64, 30));
    w.step(2);
    assert_eq!(seen_ids(&w, ada), [bea.0]);
    w.despawn(bea);
    w.step(1);
    assert_eq!(seen_ids(&w, ada), [] as [u64; 0]);
    assert_eq!(outside(&w, 0, Pos::new(63, 30), 64), []);
    // A shout finds nobody on the other side, and no notice is lost.
    w.act_ok(ada, "shout", json!({ "text": "anyone?" }));
    w.step(2);
    assert_eq!(w.world.dropped_messages(), 0);
}

#[test]
fn the_sight_radius_decides_what_a_zone_tells_the_zones_around_it() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(70, 30));
    w.step(2);
    let known = |w: &TestWorld<'_>, zone: u32| -> Vec<ResidentId> {
        outside(w, zone, Pos::new(64, 30), 64).into_iter().map(|outsider| outsider.id).collect()
    };
    assert_eq!((known(&w, 0), known(&w, 1)), (vec![bea], vec![ada]));

    // Bea is 7 tiles from zone 0. With a sight radius of 3 she is not in
    // the copy any more; Ada, 1 tile from zone 1, is.
    w.set_param("kernel.sight_radius", 3);
    w.step(2);
    assert_eq!((known(&w, 0), known(&w, 1)), (vec![], vec![ada]));
    // Without sight nothing crosses a border.
    w.set_param("kernel.sight_radius", 0);
    w.step(2);
    assert_eq!((known(&w, 0), known(&w, 1)), (vec![], vec![]));
    w.act_ok(ada, "shout", json!({ "text": "hello" }));
    w.step(2);
    assert!(w.notices(bea).is_empty());
    // The copies come back with the sight.
    w.set_param("kernel.sight_radius", 12);
    w.step(2);
    assert_eq!(
        outside(&w, 0, Pos::new(64, 30), 64),
        [Outsider {
            id: bea,
            zone: ZoneId(1),
            pos: Pos::new(70, 30),
            name: "Bea".into(),
            activity: None
        }]
    );
    assert_eq!(known(&w, 1), [ada]);
}

#[test]
fn the_nearby_section_mixes_both_sides_by_distance() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let here_3 = w.spawn_at("Here3", Pos::new(60, 30));
    let across_1 = w.spawn_at("Across1", Pos::new(64, 31));
    let here_1 = w.spawn_at("Here1", Pos::new(62, 30));
    let across_3 = w.spawn_at("Across3", Pos::new(66, 30));
    let across_9 = w.spawn_at("Across9", Pos::new(72, 30));
    w.step(2);
    // By distance, then by id, whatever the zone.
    assert_eq!(seen_ids(&w, ada), [across_1.0, here_1.0, here_3.0, across_3.0, across_9.0]);
    // The bound of the list counts the residents of both sides.
    w.set_param("kernel.nearby_max", 3);
    w.step(1);
    let nearby = w.observe(ada).sections["nearby"].clone();
    assert_eq!(seen_ids(&w, ada), [across_1.0, here_1.0, here_3.0]);
    assert_eq!(nearby["more_residents"], json!(2));
    assert!(nearby["residents"][0].get("zone").is_some());
    assert!(nearby["residents"][1].get("zone").is_none(), "a resident of this zone has no `zone`");
}

#[test]
fn sight_across_a_border_reaches_at_most_one_zone_size() {
    // Zones of 8 tiles, and the sight radius is 12. No module: the kernel
    // alone does this.
    let config = WorldConfig {
        cities: vec![CityConfig::new("Tiny", Pos::new(0, 0), 3)],
        zone_size: 8,
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&[], config);
    let ada = w.spawn_at("Ada", Pos::new(7, 4));
    let bea = w.spawn_at("Bea", Pos::new(15, 4));
    let cyd = w.spawn_at("Cyd", Pos::new(16, 4));
    assert_eq!((w.zone_of(ada), w.zone_of(bea), w.zone_of(cyd)), (ZoneId(0), ZoneId(1), ZoneId(2)));
    w.step(2);
    // Bea is 8 tiles from zone 0: in the copy. Cyd is 9 tiles away, two
    // zones on: not in the copy, though Ada could see 12 tiles.
    assert_eq!(seen_ids(&w, ada), [bea.0]);
    // Zone 1 is next to both.
    assert_eq!(seen_ids(&w, bea), [cyd.0, ada.0]);
    assert_eq!(seen_ids(&w, cyd), [bea.0]);
}

#[test]
fn sight_reaches_a_near_city_up_to_one_zone_size() {
    // Two cities of one zone of 8 tiles, with 5 tiles of nothing between
    // them: columns 0 to 7 and columns 13 to 20.
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("West", Pos::new(0, 0), 1),
            CityConfig::new("East", Pos::new(13, 0), 1),
        ],
        zone_size: 8,
        ..WorldConfig::default()
    };
    let mut w = TestWorld::with_config(&[], config);
    let ada = w.spawn_at("Ada", Pos::new(7, 4));
    let bea = w.spawn_at("Bea", Pos::new(15, 4));
    let cyd = w.spawn_at("Cyd", Pos::new(16, 4));
    assert_eq!((w.zone_of(ada), w.zone_of(bea), w.zone_of(cyd)), (ZoneId(0), ZoneId(1), ZoneId(1)));
    w.step(2);
    // Bea is 8 tiles from the west zone, Cyd is 9. The sight radius of 12
    // would reach both; one zone size reaches Bea only.
    assert_eq!(seen_ids(&w, ada), [bea.0]);
    assert_eq!(seen_ids(&w, bea), [cyd.0, ada.0]);
    assert_eq!(seen_ids(&w, cyd), [bea.0, ada.0], "Ada is 6 tiles from the east zone");
}

#[test]
fn a_skipped_zone_keeps_its_copy_and_catches_up() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(70, 30));
    w.step(2);
    // Zone 0 does not tick for three frames while Bea walks.
    for _ in 0..3 {
        w.act_ok(bea, "step", json!({ "dx": 1, "dy": 0 }));
        w.world.apply(Input::Step { skip: vec![ZoneId(0)] });
    }
    assert_eq!(w.pos(bea), Pos::new(73, 30));
    assert_eq!(seen(&w, ada)[0]["pos"], json!({ "x": 70, "y": 30 }));
    // Its next tick takes all the news in order.
    w.step(1);
    assert_eq!(seen(&w, ada)[0]["pos"], json!({ "x": 73, "y": 30 }));
}

/// Residents near borders, some with changes that no tick has seen yet.
fn border_setup(w: &mut TestWorld<'_>) {
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(64, 30));
    let cyd = w.spawn_at("Cyd", Pos::new(63, 63));
    let dia = w.spawn_at("Dia", Pos::new(70, 66));
    w.step(3);
    // Changes between frames: the snapshot of the determinism check is
    // taken right after them.
    w.set_pos(ada, Pos::new(62, 31));
    w.act_ok(bea, "work", json!({ "ticks": 4 }));
    w.act_ok(cyd, "step", json!({ "dx": 1, "dy": 1 }));
    w.act_ok(dia, "shout", json!({ "text": "now" }));
}

/// The four walk across borders, work and shout.
fn border_run(w: &mut TestWorld<'_>) {
    let ids: Vec<ResidentId> = (1..=4).map(ResidentId).collect();
    for frame in 0..40u32 {
        for (n, &id) in ids.iter().enumerate() {
            let n = n as u32;
            match (frame + n) % 5 {
                0 => {
                    let _ = w.act(id, "shout", json!({ "text": frame.to_string() }));
                }
                1 | 2 => {
                    let dx = if (frame / 10 + n).is_multiple_of(2) { 1 } else { -1 };
                    let _ = w.act(id, "step", json!({ "dx": dx, "dy": 0 }));
                }
                3 => {
                    let _ = w.act(id, "work", json!({ "ticks": 2 }));
                }
                _ => {}
            }
        }
        if frame == 17 {
            w.world.apply(Input::Step { skip: vec![ZoneId(1)] });
        } else {
            w.step(1);
        }
    }
}

#[test]
fn sight_across_borders_is_deterministic_and_survives_a_snapshot() {
    let hash = assert_deterministic(&[&Demo], &WorldConfig::default(), border_setup, border_run);

    // The scenario did what it says: copies exist and shouts crossed.
    let mut w = world();
    border_setup(&mut w);
    border_run(&mut w);
    assert_eq!(w.hash(), hash);
    let copies: usize = (0..4).map(|zone| outside(&w, zone, Pos::new(64, 64), 200).len()).sum();
    assert!(copies >= 4, "{copies} copies");
    let heard: usize = (1..=4).map(|id| shouts(&w, ResidentId(id)).len()).sum();
    assert!(heard >= 10, "{heard} shouts were heard");
    w.assert_conserved();

    // A restored world answers the same questions the same way.
    let before: Vec<Value> = (1..=4).map(|id| seen(&w, ResidentId(id))).collect();
    let known = outside(&w, 0, Pos::new(63, 30), 64);
    w.snapshot_restore();
    let after: Vec<Value> = (1..=4).map(|id| seen(&w, ResidentId(id))).collect();
    assert_eq!(after, before);
    assert_eq!(outside(&w, 0, Pos::new(63, 30), 64), known);
}

#[test]
fn a_snapshot_keeps_a_change_that_the_zone_has_not_settled() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let _bea = w.spawn_at("Bea", Pos::new(64, 30));
    w.act_ok(ada, "work", json!({ "ticks": 30 }));
    w.step(2);
    let copy = |w: &TestWorld<'_>| outside(w, 1, Pos::new(64, 30), 12)[0].activity;
    assert_eq!(copy(&w), Some("work"));
    // A tool stops the work through the realm, and the world is saved
    // before it settles. The restored world still tells zone 1.
    w.with_resident(ada, |realm, idx| realm.interrupt(idx, "stop"));
    w.snapshot_restore();
    w.step(2);
    assert_eq!(w.activity(ada), None);
    assert_eq!(copy(&w), None);
}

/// Replaces every occurrence of `from` in `bytes` with `to`. Both have the
/// same length, so the MessagePack structure stays valid.
fn patch_all(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut out = bytes.to_vec();
    let mut count = 0;
    for at in 0..=out.len() - from.len() {
        if &out[at..at + from.len()] == from {
            out[at..at + to.len()].copy_from_slice(to);
            count += 1;
        }
    }
    assert!(count > 0, "the snapshot has the pattern");
    out
}

#[test]
fn a_snapshot_from_before_sight_across_borders_fills_the_copies_again() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let bea = w.spawn_at("Bea", Pos::new(64, 30));
    w.step(2);
    let known = outside(&w, 0, Pos::new(63, 30), 12);
    assert_eq!(known.len(), 1);

    // An older kernel wrote none of the three border fields. Give them
    // names that this kernel does not read.
    let mut bytes = w.world.snapshot();
    for field in ["border_shown", "border_ghosts"] {
        let renamed = field.replace("border", "xxxxxx");
        bytes = patch_all(&bytes, field.as_bytes(), renamed.as_bytes());
    }
    let mut old = TestWorld::restore(&[&Demo], &bytes);
    assert_eq!(outside(&old, 0, Pos::new(63, 30), 12), [], "the old snapshot has no copy");
    // Every resident is checked in the next tick, and the zones around
    // hear of it in the tick after that.
    old.step(2);
    assert_eq!(outside(&old, 0, Pos::new(63, 30), 12), known);
    assert_eq!(seen_ids(&old, bea), [ada.0]);
    old.assert_conserved();
}

// ---- Zones in two processes ------------------------------------------------------

/// Runs one frame and returns how many messages about residents near a
/// border the world sent to zones in other processes.
fn border_messages(world: &mut World) -> usize {
    world.step();
    world.take_external().iter().filter(|envelope| envelope.kind() == "kernel.border").count()
}

#[test]
fn a_zone_tells_the_zones_around_it_only_what_changed() {
    // This process hosts zone 0. What zone 0 tells the other zones shows
    // in the outbox for other processes.
    let hosted = vec![RealmId::Global, RealmId::Zone(ZoneId(0))];
    let config = WorldConfig { hosted: Some(hosted), ..WorldConfig::default() };
    let mut w = TestWorld::with_config(&[&Demo], config);
    let ada = w.spawn_at("Ada", Pos::new(63, 30));
    let _far = w.spawn_at("Far", Pos::new(30, 30));
    // Ada is near zone 1 only: one message tells zone 1 that she is there.
    assert_eq!(border_messages(&mut w.world), 1);
    // Nothing changes: nothing is told.
    for _ in 0..5 {
        assert_eq!(border_messages(&mut w.world), 0);
    }

    // A routine starts `work` again each time it ends. The first start is
    // news. The later ones are not: the copy shows `work` already.
    let routine = json!({ "name": "work", "when": { "type": "always" },
        "then": { "verb": "work", "args": { "ticks": 2 } } });
    w.act_ok(ada, "set_routines", json!({ "routines": [routine] }));
    let told: Vec<usize> = (0..9).map(|_| border_messages(&mut w.world)).collect();
    assert_eq!(told, [1, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert!(w.reports(ada).is_empty(), "a routine action that succeeds writes no report");
    assert_eq!(w.cash(ada).cents(), 1_000 + 4 * 500, "four shifts ended in these frames");

    // Another verb is news, and so is the end of all work.
    w.act_ok(ada, "set_routines", json!({ "routines": [] }));
    w.act_ok(ada, "nap", json!({ "ticks": 2 }));
    let told: Vec<usize> = (0..5).map(|_| border_messages(&mut w.world)).collect();
    assert_eq!(told, [0, 1, 0, 1, 0], "`work` ends and `nap` starts, then `nap` ends");
    // A resident far from every border is never told about.
    w.act_ok(ResidentId(2), "work", json!({ "ticks": 1 }));
    assert_eq!((0..3).map(|_| border_messages(&mut w.world)).sum::<usize>(), 0);
}

/// Moves the external messages of `from` to `to`, through bytes, the way
/// two processes would.
fn carry(from: &mut World, to: &mut World) {
    let messages: Vec<Envelope> = from
        .take_external()
        .iter()
        .map(|envelope| {
            Envelope::from_bytes(&envelope.to_bytes()).expect("an envelope survives the wire")
        })
        .collect();
    to.deliver_external(messages);
}

#[test]
fn sight_crosses_a_border_between_two_processes() {
    let hosted =
        |realms: Vec<RealmId>| WorldConfig { hosted: Some(realms), ..WorldConfig::default() };
    let mut west = World::new(
        &[&Demo],
        hosted(vec![RealmId::Global, RealmId::Zone(ZoneId(0)), RealmId::Zone(ZoneId(2))]),
    );
    let mut east =
        World::new(&[&Demo], hosted(vec![RealmId::Zone(ZoneId(1)), RealmId::Zone(ZoneId(3))]));
    let spawn = |world: &mut World, name: &str| {
        let spec = SpawnSpec {
            owner: OwnerId(1),
            name: name.to_string(),
            model: None,
            city: Some(CityId(0)),
        };
        world.spawn(spec).expect("spawn in zone 0")
    };
    let ada = spawn(&mut west, "Ada");
    let bea = spawn(&mut west, "Bea");
    {
        let zone0 = west.realm(RealmId::Zone(ZoneId(0))).expect("hosted");
        zone0.set_pos(zone0.idx_of(ada).expect("in zone 0"), Pos::new(63, 30)).expect("zone 0");
        zone0.set_pos(zone0.idx_of(bea).expect("in zone 0"), Pos::new(64, 30)).expect("zone 1");
    }
    west.settle();
    carry(&mut west, &mut east);
    assert_eq!((west.locate(bea), east.locate(bea)), (None, Some(ZoneId(1))));
    east.act(bea, ActionRequest::new("work", json!({ "ticks": 30 }))).expect("bea acts");
    for _ in 0..3 {
        west.step();
        east.step();
        carry(&mut west, &mut east);
        carry(&mut east, &mut west);
    }
    west.step();
    east.step();

    let zone0 = west.realm(RealmId::Zone(ZoneId(0))).expect("hosted");
    assert_eq!(
        zone0.nearby_outside(Pos::new(63, 30), 12),
        [Outsider {
            id: bea,
            zone: ZoneId(1),
            pos: Pos::new(64, 30),
            name: "Bea".into(),
            activity: Some("work"),
        }]
    );
    let zone1 = east.realm(RealmId::Zone(ZoneId(1))).expect("hosted");
    assert_eq!(
        zone1.nearby_outside(Pos::new(64, 30), 12),
        [Outsider {
            id: ada,
            zone: ZoneId(0),
            pos: Pos::new(63, 30),
            name: "Ada".into(),
            activity: None
        }]
    );

    // A shout in the west reaches the resident in the east.
    west.act(ada, ActionRequest::new("shout", json!({ "text": "over there" }))).expect("ada acts");
    carry(&mut west, &mut east);
    east.step();
    let heard = east.observe(bea, &Default::default()).expect("bea is in the east").notices;
    assert_eq!(heard.len(), 1);
    assert_eq!((heard[0].kind.as_str(), heard[0].from), ("demo.shout", Some(ada)));
    west.audit().expect("west is balanced");
    east.audit().expect("east is balanced");
}
