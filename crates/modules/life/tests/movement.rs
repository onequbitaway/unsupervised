//! Walking: paths, speed, visible steps, zone borders and refusals.

mod common;

use common::{
    add_params, cities, paint, report, result, site, site_in_zone, tile_at, walkable, world,
    world_with,
};
use unsup_kernel::protocol::{ActionStatus, ErrorCode, FrameOutput};
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Pos, Rect, ResidentId, SiteId, WorldConfig, ZoneId};
use unsup_life::Walkers;
use unsup_life::names::tile;

/// Returns the positions that the frames showed for a resident, in order.
fn seen(frames: &[FrameOutput], resident: ResidentId) -> Vec<Pos> {
    frames
        .iter()
        .flat_map(|frame| frame.zones.iter())
        .flat_map(|zone| zone.moved.iter())
        .filter(|(id, _)| *id == resident)
        .map(|(_, pos)| *pos)
        .collect()
}

/// Builds a closed room in zone 0: grass inside a ring of wall.
fn room(w: &TestWorld<'_>, outer: Rect) {
    paint(w, outer, tile::WALL);
    paint(w, Rect::new(outer.x0 + 1, outer.y0 + 1, outer.x1 - 1, outer.y1 - 1), tile::GRASS);
}

/// Spawns a resident at a position and runs one frame, so that the next
/// frames show only what the resident does itself.
fn place(w: &mut TestWorld<'_>, name: &str, pos: Pos) -> ResidentId {
    let id = w.spawn_at(name, pos);
    w.step(1);
    id
}

/// Returns the number of residents that walk, in all zones.
fn walkers(w: &TestWorld<'_>) -> usize {
    w.world.layout().zones().iter().map(|zone| w.zone(zone.id).res::<Walkers>().len()).sum()
}

fn started_ticks(status: &ActionStatus) -> u32 {
    match status {
        ActionStatus::Started { ticks, .. } => *ticks,
        other => panic!("the action did not start: {other:?}"),
    }
}

#[test]
fn a_path_goes_around_a_wall_and_through_the_door() {
    let mut w = world();
    // A room with a wall in the middle. The only way from the left half to
    // the right half is the door at (30, 29).
    room(&w, Rect::new(20, 20, 41, 33));
    paint(&w, Rect::new(30, 21, 31, 32), tile::WALL);
    let door = Pos::new(30, 29);
    paint(&w, Rect::new(door.x, door.y, door.x + 1, door.y + 1), tile::FLOOR);

    let ada = place(&mut w, "Ada", Pos::new(25, 22));
    let target = Pos::new(35, 22);
    let status = w.act_ok(ada, "move_to", json!({ "x": target.x, "y": target.y }));
    // Seven steps on grass to the tile before the door, one onto the door
    // (floor), one out of it onto grass, seven more on grass:
    // 7 * 2 + 1 + 2 + 7 * 2 ticks.
    assert_eq!(started_ticks(&status), 31);

    let frames = w.step(31);
    let path = seen(&frames, ada);
    assert_eq!(path.len(), 16);
    assert!(path.contains(&door), "the path {path:?} misses the door");
    let mut before = Pos::new(25, 22);
    for pos in &path {
        assert_eq!(before.dist(*pos), 1, "a step from {before} to {pos}");
        assert!(walkable(&w, *pos), "the path steps on {pos}");
        before = *pos;
    }
    assert_eq!(w.pos(ada), target);
    assert_eq!(w.activity(ada), None);
    assert_eq!(result(&w, ada), json!({ "x": 35, "y": 22, "steps": 16, "ticks": 31 }));
    assert_eq!(walkers(&w), 0);
}

#[test]
fn a_walker_moves_one_tile_in_each_frame_that_it_moves() {
    let mut w = world();
    room(&w, Rect::new(20, 20, 41, 23));
    paint(&w, Rect::new(21, 21, 40, 22), tile::ROAD);
    let ada = place(&mut w, "Ada", Pos::new(21, 21));
    w.act_ok(ada, "move_to", json!({ "x": 31, "y": 21 }));
    for step in 1..=10 {
        let frames = w.step(1);
        // Exactly one visible move in each frame, to the next tile.
        assert_eq!(seen(&frames, ada), [Pos::new(21 + step, 21)]);
    }
    assert_eq!(w.activity(ada), None);
}

#[test]
fn roads_are_faster_than_grass_and_trees_are_slower() {
    let mut w = world();
    room(&w, Rect::new(20, 20, 41, 23));
    let ada = place(&mut w, "Ada", Pos::new(21, 21));
    let mut at = 21;
    for (code, ticks) in [
        (tile::ROAD, 10),
        (tile::PAVEMENT, 10),
        (tile::FLOOR, 10),
        (tile::GRASS, 20),
        (tile::FIELD, 20),
        (tile::TREES, 40),
    ] {
        paint(&w, Rect::new(21, 21, 40, 22), code);
        // Walk ten tiles to the other end of the corridor and back.
        at = if at == 21 { 31 } else { 21 };
        let status = w.act_ok(ada, "move_to", json!({ "x": at, "y": 21 }));
        assert_eq!(started_ticks(&status), ticks, "{}", tile::name(code));
        assert_eq!(w.run_until_idle(ada, 100), ticks);
        assert_eq!(w.pos(ada), Pos::new(at, 21));
        assert_eq!(result(&w, ada)["ticks"], ticks);
    }
}

#[test]
fn the_cost_of_a_step_is_a_parameter() {
    let mut config = WorldConfig::default();
    add_params(&mut config, &[("life.walk_ticks_grass", 5)]);
    let mut w = world_with(config);
    room(&w, Rect::new(20, 20, 41, 23));
    let ada = place(&mut w, "Ada", Pos::new(21, 21));
    let status = w.act_ok(ada, "move_to", json!({ "x": 25, "y": 21 }));
    assert_eq!(started_ticks(&status), 20);
}

#[test]
fn a_walk_to_a_site_ends_inside_and_enters_through_a_door() {
    let mut w = world();
    let (zone, home) = site(&w, "home");
    let r = home.rect;
    // Start on the road at the corner of the zone.
    let corner = w.world.layout().zone(zone).expect("the zone").bounds;
    let ada = place(&mut w, "Ada", Pos::new(corner.x0, corner.y0));
    let status = w.act_ok(ada, "move_to", json!({ "site": home.id }));
    let ticks = started_ticks(&status);
    let frames = w.step(ticks);
    let path = seen(&frames, ada);
    let end = w.pos(ada);
    assert!(r.contains(end), "{end} is not in {r:?}");
    assert_eq!(tile_at(&w, end), tile::FLOOR);
    assert_eq!(end, r.centre());
    // The path crossed the wall ring exactly once, at a floor tile.
    let on_ring = |pos: &Pos| {
        r.contains(*pos)
            && (pos.x == r.x0 || pos.x == r.x1 - 1 || pos.y == r.y0 || pos.y == r.y1 - 1)
    };
    let crossings: Vec<Pos> = path.iter().copied().filter(on_ring).collect();
    assert_eq!(crossings.len(), 1, "{path:?}");
    assert_eq!(tile_at(&w, crossings[0]), tile::FLOOR);
    assert!(path.iter().all(|pos| walkable(&w, *pos)));
    let out = result(&w, ada);
    assert_eq!(out["site"], json!(home.id));
    assert_eq!(out["steps"], json!(path.len()));
}

#[test]
fn a_walk_across_zone_borders_arrives() {
    let mut w = world();
    // From the north-west corner of zone 0 to the south-east corner of
    // zone 3. Both are roads.
    let ada = place(&mut w, "Ada", Pos::new(0, 0));
    assert_eq!(w.zone_of(ada), ZoneId(0));
    let target = Pos::new(127, 127);
    let ack = w.act(ada, "move_to", json!({ "x": target.x, "y": target.y }));
    assert_eq!(ack.status, ActionStatus::Pending);

    let mut zones = vec![ZoneId(0)];
    let mut frames = 0u64;
    let mut path = Vec::new();
    while w.activity(ada).is_some() {
        path.extend(seen(&w.step(1), ada));
        frames += 1;
        assert!(frames < 1_000, "the walk does not end");
        if zones.last() != Some(&w.zone_of(ada)) {
            zones.push(w.zone_of(ada));
        }
    }
    assert_eq!(w.pos(ada), target);
    // The walk went through a zone between the two: the resident migrated
    // twice.
    assert_eq!(zones.len(), 3, "{zones:?}");
    assert_eq!(zones[2], ZoneId(3));
    // Every step was one tile, also across the borders.
    let mut before = Pos::new(0, 0);
    for pos in &path {
        assert_eq!(before.dist(*pos), 1, "a step from {before} to {pos}");
        before = *pos;
    }
    let out = result(&w, ada);
    assert_eq!(out["steps"], json!(path.len()));
    assert_eq!(out["ticks"], json!(frames));
    // The target is 127 tiles away in both directions. A step moves one
    // tile, and the way along the roads takes 254 ticks. Each zone plans
    // its own part, so the whole walk is near the best but not always the
    // best.
    assert!(path.len() >= 127, "{} steps", path.len());
    assert!((127..=300).contains(&frames), "{frames} ticks");
    assert_eq!(walkers(&w), 0);
    w.assert_conserved();
}

#[test]
fn a_walk_to_a_site_of_another_district_asks_where_it_is() {
    let mut w = world();
    let ada = place(&mut w, "Ada", Pos::new(0, 0));
    // A site in the zone that is farthest from zone 0.
    let far = site_in_zone(&w, ZoneId(3), "home")
        .or_else(|| w.zone(ZoneId(3)).sites().iter().next().cloned())
        .expect("zone 3 has a site");
    let ack = w.act(ada, "move_to", json!({ "site": far.id }));
    assert_eq!(ack.status, ActionStatus::Pending);
    assert!(w.run_until_idle(ada, 1_000) > 2);
    assert_eq!(w.zone_of(ada), ZoneId(3));
    assert!(far.rect.contains(w.pos(ada)));
    assert!(walkable(&w, w.pos(ada)));
    assert_eq!(result(&w, ada)["site"], json!(far.id));

    // A site that does not exist: the answer is an error.
    let ack = w.act(ada, "move_to", json!({ "site": SiteId(12_345) }));
    assert_eq!(ack.status, ActionStatus::Pending);
    w.run_until_idle(ada, 10);
    let failed = report(&w, ada);
    assert!(!failed.ok);
    assert_eq!(failed.error.expect("an error").code, ErrorCode::NotFound);
    assert_eq!(walkers(&w), 0);
}

#[test]
fn a_target_that_is_blocked_or_cannot_be_reached_is_refused() {
    let mut w = world_with(cities(0, &[2, 1]));
    room(&w, Rect::new(20, 20, 41, 33));
    let ada = place(&mut w, "Ada", Pos::new(0, 0));

    // A wall.
    let error = w.act_err(ada, "move_to", json!({ "x": 20, "y": 20 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.details, json!({ "tile": "wall" }));
    // A walkable tile inside a closed room.
    let error = w.act_err(ada, "move_to", json!({ "x": 25, "y": 25 }));
    assert_eq!(error.code, ErrorCode::NotFound);
    // A tile outside the world.
    let error = w.act_err(ada, "move_to", json!({ "x": -5, "y": 3 }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
    // A tile in another city.
    let other = w.world.layout().zone(ZoneId(4)).expect("the second city").bounds;
    let error = w.act_err(ada, "move_to", json!({ "x": other.x0, "y": other.y0 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    // No target, half a target, two targets.
    for args in [json!({}), json!({ "x": 3 }), json!({ "x": 3, "y": 3, "site": 9 })] {
        assert_eq!(w.act_err(ada, "move_to", args).code, ErrorCode::InvalidArgs);
    }
    // Nothing changed.
    assert_eq!(w.pos(ada), Pos::new(0, 0));
    assert_eq!(w.activity(ada), None);
    assert_eq!(walkers(&w), 0);

    // The place where the resident stands: done at once, no steps.
    let status = w.act_ok(ada, "move_to", json!({ "x": 0, "y": 0 }));
    assert_eq!(
        status,
        ActionStatus::Done { result: json!({ "x": 0, "y": 0, "steps": 0, "ticks": 0 }) }
    );
}

#[test]
fn the_search_for_a_path_is_bounded() {
    let mut config = WorldConfig::default();
    add_params(&mut config, &[("life.path_search_max", 16)]);
    let mut w = world_with(config);
    let ada = place(&mut w, "Ada", Pos::new(0, 0));
    // Near: found within the limit.
    w.act_ok(ada, "move_to", json!({ "x": 5, "y": 0 }));
    w.run_until_idle(ada, 10);
    assert_eq!(w.pos(ada), Pos::new(5, 0));
    // Far: the search stops.
    let error = w.act_err(ada, "move_to", json!({ "x": 60, "y": 60 }));
    assert_eq!(error.code, ErrorCode::NotFound);
    assert!(error.message.contains("16 tiles"), "{}", error.message);
}

#[test]
fn cancel_stops_a_walk_where_the_resident_is() {
    let mut w = world();
    let ada = place(&mut w, "Ada", Pos::new(0, 0));
    w.act_ok(ada, "move_to", json!({ "x": 30, "y": 0 }));
    w.step(7);
    assert_eq!(w.pos(ada), Pos::new(7, 0));
    w.act_ok(ada, "cancel", json!({}));
    assert_eq!(walkers(&w), 0);
    w.step(5);
    assert_eq!(w.pos(ada), Pos::new(7, 0));
    let stopped = report(&w, ada);
    assert_eq!(stopped.error.expect("an error").code, ErrorCode::Interrupted);

    // A walk to another zone stops the same way.
    let ack = w.act(ada, "move_to", json!({ "x": 100, "y": 0 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    w.step(3);
    assert_eq!(w.pos(ada), Pos::new(10, 0));
    w.act_ok(ada, "cancel", json!({}));
    w.step(3);
    assert_eq!(w.pos(ada), Pos::new(10, 0));
    assert_eq!(walkers(&w), 0);

    // The next walk starts from there.
    w.act_ok(ada, "move_to", json!({ "x": 12, "y": 0 }));
    w.run_until_idle(ada, 5);
    assert_eq!(w.pos(ada), Pos::new(12, 0));
}

#[test]
fn planned_walks_follow_each_other() {
    let mut w = world();
    room(&w, Rect::new(20, 20, 41, 23));
    paint(&w, Rect::new(21, 21, 40, 22), tile::ROAD);
    let ada = place(&mut w, "Ada", Pos::new(21, 21));
    w.act_ok(ada, "move_to", json!({ "x": 25, "y": 21 }));
    let queued = w.act_ok(ada, "move_to", json!({ "x": 22, "y": 21 }));
    assert_eq!(queued, ActionStatus::Queued { position: 1 });
    // Four tiles there and three tiles back.
    assert_eq!(w.run_until_idle(ada, 20), 7);
    assert_eq!(w.pos(ada), Pos::new(22, 21));
}

#[test]
fn a_walker_that_something_else_moves_stops_where_it_was_put() {
    let mut w = world();
    room(&w, Rect::new(20, 20, 41, 23));
    paint(&w, Rect::new(21, 21, 40, 22), tile::ROAD);
    let ada = place(&mut w, "Ada", Pos::new(21, 21));

    // A walk inside one zone. After three steps, something puts the walker
    // back at the start: an arrest, for example.
    w.act_ok(ada, "move_to", json!({ "x": 35, "y": 21 }));
    w.step(3);
    assert_eq!(w.pos(ada), Pos::new(24, 21));
    w.set_pos(ada, Pos::new(21, 21));
    w.step(3);
    // The walk did not go on from the new place.
    assert_eq!(w.pos(ada), Pos::new(21, 21));
    assert_eq!(w.activity(ada), None);
    assert_eq!(walkers(&w), 0);
    let stopped = report(&w, ada).error.expect("an error");
    assert_eq!(stopped.code, ErrorCode::Interrupted);
    assert!(stopped.message.contains("moved during the walk"), "{}", stopped.message);

    // The same just before the last step, which the end of the action makes.
    w.act_ok(ada, "move_to", json!({ "x": 25, "y": 21 }));
    w.step(3);
    w.set_pos(ada, Pos::new(30, 21));
    w.step(2);
    assert_eq!(w.pos(ada), Pos::new(30, 21));
    assert_eq!(w.activity(ada), None);
    assert!(!report(&w, ada).ok);

    // A walk to another zone. The walker is put in a third zone on the way;
    // the walk must not go on from there.
    let bob = place(&mut w, "Bob", Pos::new(0, 0));
    let ack = w.act(bob, "move_to", json!({ "x": 100, "y": 0 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    w.step(5);
    assert_eq!(w.pos(bob), Pos::new(5, 0));
    w.set_pos(bob, Pos::new(0, 100));
    assert_eq!(w.zone_of(bob), ZoneId(2));
    w.step(3);
    assert_eq!(w.pos(bob), Pos::new(0, 100));
    assert_eq!(w.activity(bob), None);
    assert_eq!(report(&w, bob).error.expect("an error").code, ErrorCode::Conflict);
    assert_eq!(walkers(&w), 0);
}

#[test]
fn a_way_that_gets_blocked_stops_the_walk_in_front_of_the_block() {
    let mut w = world();
    room(&w, Rect::new(20, 20, 41, 23));
    paint(&w, Rect::new(21, 21, 40, 22), tile::ROAD);
    let ada = place(&mut w, "Ada", Pos::new(21, 21));
    w.act_ok(ada, "move_to", json!({ "x": 35, "y": 21 }));
    w.step(3);
    // A wall across the corridor, five tiles ahead.
    paint(&w, Rect::new(29, 21, 30, 22), tile::WALL);
    w.step(20);
    assert_eq!(w.pos(ada), Pos::new(28, 21));
    assert_eq!(w.activity(ada), None);
    let stopped = report(&w, ada).error.expect("an error");
    assert_eq!(stopped.code, ErrorCode::Interrupted);
    assert!(stopped.message.contains("blocked at (29, 21)"), "{}", stopped.message);
    assert_eq!(walkers(&w), 0);

    // The block is the last tile of the walk: the end of the action finds it.
    w.act_ok(ada, "move_to", json!({ "x": 26, "y": 21 }));
    paint(&w, Rect::new(26, 21, 27, 22), tile::WALL);
    w.step(5);
    assert_eq!(w.pos(ada), Pos::new(27, 21));
    let stopped = report(&w, ada).error.expect("an error");
    assert_eq!(stopped.code, ErrorCode::Conflict);
}

#[test]
fn the_first_walk_of_a_newcomer_shows_every_step_to_viewers() {
    // The scenario of the server test `skeleton`: seed 7, one city of 2 by 2
    // districts, a newcomer that walks three tiles east and two south.
    let mut config = WorldConfig::with_seed(7);
    config.cities = vec![unsup_kernel::CityConfig::new("Latent City", Pos::new(0, 0), 2)];
    let mut w = world_with(config);
    let ada = w.spawn("Ada");
    let start = w.pos(ada);
    let target = Pos::new(start.x + 3, start.y + 2);
    let status = w.act_ok(ada, "move_to", json!({ "x": target.x, "y": target.y }));
    assert_eq!(started_ticks(&status), 3);
    let path = seen(&w.step(3), ada);
    assert_eq!(path.len(), 3, "{path:?}");
    assert_eq!(path[2], target);
    assert_ne!(path[0], target);
    assert_eq!(start.dist(path[0]), 1);
}

#[test]
fn a_walk_to_another_zone_that_takes_too_long_fails_with_a_timeout() {
    let mut config = WorldConfig::default();
    add_params(&mut config, &[("life.walk_timeout", 60)]);
    let mut w = world_with(config);
    let ada = place(&mut w, "Ada", Pos::new(0, 0));
    // 127 tiles away: more than 60 ticks.
    let ack = w.act(ada, "move_to", json!({ "x": 127, "y": 127 }));
    assert_eq!(ack.status, ActionStatus::Pending);
    assert_eq!(w.run_until_idle(ada, 100), 60);
    let failed = report(&w, ada).error.expect("an error");
    assert_eq!(failed.code, ErrorCode::Timeout);
    assert_eq!(walkers(&w), 0);
    // The resident stays where the walk stopped.
    let stopped = w.pos(ada);
    assert_ne!(stopped, Pos::new(0, 0));
    w.step(5);
    assert_eq!(w.pos(ada), stopped);
}
