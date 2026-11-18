//! The observation sections `home` and `skills`, the verb `inspect`, and
//! the default routines.

mod common;

use common::{make_owner, site, with_params, world, world_with};
use unsup_kernel::protocol::{ActionStatus, Detail, ErrorCode, ObserveOpts};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{Holder, Money, Pos};
use unsup_life::default_routines;

fn done(status: ActionStatus) -> Value {
    match status {
        ActionStatus::Done { result } => result,
        other => panic!("the action did not finish at once: {other:?}"),
    }
}

#[test]
fn the_home_section_names_the_shelter_of_a_newcomer() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let observation = w.observe(ada);
    let home = observation.sections.get("home").expect("a home section");
    let (zone, shelter) = common::all_sites(&w)
        .into_iter()
        .find(|(_, place)| Some(json!(place.id)) == home.get("site").cloned())
        .expect("the site of the home section exists");
    assert_eq!(shelter.kind, "shelter");
    assert_eq!(
        *home,
        json!({
            "site": shelter.id,
            "kind": "shelter",
            "name": shelter.name,
            "zone": zone,
            "pos": shelter.rect.centre(),
            "sleep": "fair",
        })
    );
    // A newcomer has no skill above level 0: no skills section.
    assert!(!observation.sections.contains_key("skills"));
    // The sections are small.
    assert!(home.to_string().len() < 200);
}

#[test]
fn the_home_section_follows_the_place_where_the_resident_sleeps() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let (zone, home) = site(&w, "home");
    make_owner(&w, zone, home.id, ada);
    w.set_pos(ada, home.rect.centre());
    w.set_attr(ada, "need.energy", 500);
    w.act_ok(ada, "sleep", json!({ "ticks": 30 }));
    w.run_until_idle(ada, 40);
    let section = w.observe(ada).sections.get("home").cloned().expect("a home section");
    assert_eq!(section["site"], json!(home.id));
    assert_eq!(section["kind"], "home");
    assert_eq!(section["sleep"], "good");

    // From another zone the section still names the home.
    let elsewhere = w
        .world
        .layout()
        .zones()
        .iter()
        .find(|info| info.id != zone)
        .map(|info| Pos::new(info.bounds.x0, info.bounds.y0))
        .expect("another zone");
    w.set_pos(ada, elsewhere);
    let section = w.observe(ada).sections.get("home").cloned().expect("a home section");
    assert_eq!(section["site"], json!(home.id));
    assert_eq!(section["zone"], json!(zone));
    assert_eq!(section["sleep"], "good");
    assert_eq!(section.get("name"), None);

    // A sleep on the street does not change it.
    w.set_attr(ada, "need.energy", 500);
    w.act_ok(ada, "sleep", json!({ "ticks": 30 }));
    w.run_until_idle(ada, 40);
    assert_eq!(w.observe(ada).sections["home"]["site"], json!(home.id));
}

#[test]
fn the_skills_section_has_levels_and_full_detail_adds_experience() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.set_attr(ada, "skill.farming", 450);
    w.set_attr(ada, "skill.law", 100);
    w.set_attr(ada, "skill.combat", 99);
    let brief = w.observe(ada);
    assert_eq!(brief.sections["skills"], json!({ "farming": 2, "law": 1 }));
    // The experience is not in the brief attributes.
    assert_eq!(brief.sections["self"]["attrs"].get("skill.farming"), None);
    assert_eq!(brief.sections["self"]["attrs"]["need.food"], 1_000);
    assert_eq!(brief.sections["self"]["attrs"]["age.years"], 18);

    let full = w.observe_with(ada, &ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() });
    assert_eq!(
        full.sections["skills"],
        json!({
            "farming": { "level": 2, "xp": 450, "next": 900 },
            "law": { "level": 1, "xp": 100, "next": 400 },
        })
    );
    assert_eq!(full.sections["self"]["attrs"]["skill.farming"], 450);
}

#[test]
fn inspect_shows_a_resident_a_site_or_the_place() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 0));
    let bob = w.spawn_at("Bob", Pos::new(14, 0));
    w.set_attr(bob, "skill.cooking", 400);
    w.set_attr(bob, "need.health", 300);
    w.set_condition(bob, "asleep", None);

    let seen = done(w.act_ok(ada, "inspect", json!({ "resident": bob })));
    assert_eq!(
        seen,
        json!({
            "resident": bob,
            "name": "Bob",
            "model": null,
            "age": 18,
            "pos": { "x": 14, "y": 0 },
            "distance": 4,
            "activity": null,
            "conditions": ["asleep"],
            "health": "hurt",
            "skills": { "cooking": 2 },
            "site": null,
        })
    );

    // The place where Ada stands: a road in a district.
    let here = done(w.act_ok(ada, "inspect", json!({})));
    assert_eq!(here["tile"], "road");
    assert_eq!(here["pos"], json!({ "x": 10, "y": 0 }));
    assert_eq!(here["zone"], 0);
    assert_eq!(here["city"], 0);
    assert!(
        ["downtown", "residential", "industrial", "rural"]
            .contains(&here["district"].as_str().expect("a district")),
        "{here}"
    );
    assert_eq!(here["site"], Value::Null);

    // A site: all that the brief observation leaves out.
    let (zone, shop) = site(&w, "shop");
    w.set_pos(ada, shop.rect.centre());
    w.step(1);
    let details = done(w.act_ok(ada, "inspect", json!({ "site": shop.id })));
    assert_eq!(details["kind"], "shop");
    assert_eq!(details["name"], json!(shop.name));
    assert_eq!(details["rect"], json!(shop.rect));
    assert_eq!(details["owner"], json!({ "Org": 100 }));
    assert_eq!(details["yours"], false);
    assert_eq!(details["tenants"], 0);
    assert_eq!(details["attrs"]["value"], json!(shop.attr("value")));
    assert_eq!(details["stand"], json!(shop.rect.centre()));
    assert_eq!(details["sleep"], "poor");
    assert_eq!(details["doors"].as_array().map(Vec::len), Some(1));
    assert_eq!(details.get("till"), None);
    // The owner sees the till.
    make_owner(&w, zone, shop.id, ada);
    w.give_money(ada, Money(70));
    w.with_resident(ada, |realm, _| {
        realm
            .ledger_mut()
            .transfer_money(Holder::Resident(ada), Holder::Site(shop.id), Money(70), "test")
            .expect("the money moves");
    });
    let details = done(w.act_ok(ada, "inspect", json!({ "site": shop.id })));
    assert_eq!((details["yours"].clone(), details["till"].clone()), (json!(true), json!(70)));
    // Standing in it, the place names the site too.
    let here = done(w.act_ok(ada, "inspect", json!({})));
    assert_eq!(here["site"]["site"], json!(shop.id));
    assert_eq!(here["tile"], "floor");
}

#[test]
fn inspect_refusals() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(0, 0));
    let far = w.spawn_at("Far", Pos::new(40, 0));
    let away = w.spawn_at("Away", Pos::new(100, 100));
    // Sight is 12 tiles.
    assert_eq!(w.act_err(ada, "inspect", json!({ "resident": far })).code, ErrorCode::NotHere);
    assert_eq!(w.act_err(ada, "inspect", json!({ "resident": away })).code, ErrorCode::NotHere);
    assert_eq!(w.act_err(ada, "inspect", json!({ "site": 77 })).code, ErrorCode::NotFound);
    w.step(1);
    let (_, farthest) = common::all_sites(&w)
        .into_iter()
        .filter(|(zone, _)| zone.0 == 0)
        .max_by_key(|(_, place)| place.rect.dist_to(Pos::new(0, 0)))
        .expect("zone 0 has sites");
    assert_eq!(w.act_err(ada, "inspect", json!({ "site": farthest.id })).code, ErrorCode::NotHere);
    let error = w.act_err(ada, "inspect", json!({ "site": farthest.id, "resident": far }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
}

/// Returns the routines of a resident as its owner sees them: the name
/// and the number of runs of each.
fn routines_of(
    w: &unsup_kernel::testing::TestWorld<'_>,
    r: unsup_kernel::ResidentId,
) -> Vec<(String, u64)> {
    let full = w.observe_with(r, &ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() });
    full.sections["self"]["routines"]
        .as_array()
        .expect("routines")
        .iter()
        .map(|routine| {
            (
                routine["name"].as_str().expect("a name").to_string(),
                routine["runs"].as_u64().expect("runs"),
            )
        })
        .collect()
}

#[test]
fn a_newcomer_has_the_default_routines_and_they_keep_it_fed_and_rested() {
    // The reflexes are off: only the routines act.
    let mut w = world_with(with_params(&[("life.reflexes", 0)]));
    let ada = w.spawn("Ada");
    // Nobody sent `set_routines`: the two routines come with the spawn.
    assert_eq!(
        routines_of(&w, ada),
        [("eat when hungry".to_string(), 0), ("sleep when tired".to_string(), 0)]
    );
    // They are the routines that `default_routines` returns.
    let routines = default_routines(w.world.blueprint());
    assert_eq!(routines.len(), 2);
    assert_eq!(routines[0].then.verb, "eat");
    assert_eq!(routines[1].then.verb, "sleep");

    w.give_item(ada, "bread", 30);
    for _ in 0..20 {
        w.step(1_440);
    }
    assert!(w.is_alive(ada));
    // The routine eats when food is below 400 and sleeps when energy is
    // below 250, so neither need stays low for long.
    assert!(w.attr(ada, "need.food") >= 386, "food {}", w.attr(ada, "need.food"));
    assert_eq!(w.attr(ada, "need.health"), 1_000);
    let left = w.qty(Holder::Resident(ada), "bread");
    assert!((5..=10).contains(&left), "{left} breads left");
    // Full detail shows how often each routine ran. About one meal a day.
    // One sleep in a shelter and the day after it take about 27 hours.
    let runs: Vec<u64> = routines_of(&w, ada).into_iter().map(|(_, runs)| runs).collect();
    assert!((20..=24).contains(&runs[0]) && (15..=19).contains(&runs[1]), "{runs:?}");
    w.assert_conserved();

    // Without food the routine `eat` does not fire, and the resident
    // starves.
    let bob = w.spawn("Bob");
    for _ in 0..8 {
        w.step(1_440);
    }
    assert!(!w.is_alive(bob));
    assert!(w.is_alive(ada));
}

#[test]
fn the_owner_can_replace_the_routines_and_a_world_can_turn_them_off() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(routines_of(&w, ada).len(), 2);
    // The verb replaces the list. The defaults come back the same way.
    assert_eq!(
        done(w.act_ok(ada, "set_routines", json!({ "routines": [] }))),
        json!({ "routines": 0 })
    );
    assert_eq!(routines_of(&w, ada), []);
    let routines = default_routines(w.world.blueprint());
    let installed = done(w.act_ok(ada, "set_routines", json!({ "routines": routines })));
    assert_eq!(installed, json!({ "routines": 2 }));
    assert_eq!(routines_of(&w, ada).len(), 2);

    // With `life.spawn_routines` at 0 a newcomer starts without routines.
    let mut w = world_with(with_params(&[("life.spawn_routines", 0)]));
    let bob = w.spawn("Bob");
    assert_eq!(routines_of(&w, bob), []);

    // The thresholds are the parameters of the world.
    let mut w = world_with(with_params(&[
        ("life.routine_eat_below", 900),
        ("life.routine_sleep_below", 0),
    ]));
    let cyd = w.spawn("Cyd");
    w.give_item(cyd, "bread", 1);
    // Food 1000 - 8 * 14 = 888 after eight hours: below 900, Cyd eats.
    w.step(8 * 60 + 15);
    assert_eq!(w.qty(Holder::Resident(cyd), "bread"), 0);
    assert_eq!(routines_of(&w, cyd)[0], ("eat when hungry".to_string(), 1));
    assert_eq!(routines_of(&w, cyd)[1], ("sleep when tired".to_string(), 0));
}

#[test]
fn a_resident_that_sleeps_sees_only_what_is_next_to_it() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 0));
    let near = w.spawn_at("Near", Pos::new(11, 0));
    let far = w.spawn_at("Far", Pos::new(14, 0));
    // Awake: both are in sight.
    assert_eq!(done(w.act_ok(ada, "inspect", json!({ "resident": far })))["distance"], 4);
    w.step(1);
    // Asleep: sight is one tile.
    w.set_condition(ada, "asleep", None);
    assert_eq!(done(w.act_ok(ada, "inspect", json!({ "resident": near })))["distance"], 1);
    assert_eq!(w.act_err(ada, "inspect", json!({ "resident": far })).code, ErrorCode::NotHere);
    // The place where it lies is always in sight.
    assert_eq!(done(w.act_ok(ada, "inspect", json!({})))["tile"], "road");
}

#[test]
fn inspect_tells_how_healthy_a_resident_looks() {
    let mut w = world();
    let ada = w.spawn_at("Ada", Pos::new(10, 0));
    let bob = w.spawn_at("Bob", Pos::new(11, 0));
    // Critical below 250, hurt below 500, else good. Two free actions fit
    // in one tick, so a frame runs between the pairs.
    for (health, looks) in [(1_000, "good"), (500, "good"), (499, "hurt"), (250, "hurt")] {
        w.set_attr(bob, "need.health", health);
        assert_eq!(done(w.act_ok(ada, "inspect", json!({ "resident": bob })))["health"], looks);
        w.step(1);
    }
    w.set_attr(bob, "need.health", 249);
    assert_eq!(done(w.act_ok(ada, "inspect", json!({ "resident": bob })))["health"], "critical");
}

#[test]
fn a_newcomer_in_a_station_gets_a_bed_in_the_nearest_shelter_of_the_city() {
    // One shelter in the whole city, so that the station is in a zone
    // without a shelter.
    let mut w = world_with(with_params(&[("life.gen.zones_per.shelter", 10_000)]));
    let (shelter_zone, shelter) = site(&w, "shelter");
    let (station_zone, station) = site(&w, "station");
    assert_ne!(shelter_zone, station_zone);
    // The zones learn about the shelters of the other zones in the first
    // frame.
    w.step(1);
    let newcomers = [w.spawn("Ada"), w.spawn("Bob")];
    let in_station = newcomers
        .into_iter()
        .find(|id| station.rect.contains(w.pos(*id)))
        .expect("one of two newcomers appears in the station");
    let home = w.observe(in_station).sections.get("home").cloned().expect("a home section");
    assert_eq!(
        home,
        json!({
            "site": shelter.id,
            "kind": "shelter",
            "zone": shelter_zone,
            "pos": shelter.rect.centre(),
            "sleep": "fair",
        })
    );
    // The bed is a place to walk to.
    w.act(in_station, "move_to", json!({ "site": shelter.id }));
    w.run_until_idle(in_station, 600);
    assert!(shelter.rect.contains(w.pos(in_station)));
}

#[test]
fn inspect_sees_a_resident_on_the_other_side_of_a_district_border() {
    let mut w = world();
    // The border between zone 0 and zone 1 is between x = 63 and x = 64.
    let ada = w.spawn_at("Ada", Pos::new(60, 10));
    let bob = w.spawn_at("Bob", Pos::new(66, 12));
    let far = w.spawn_at("Far", Pos::new(73, 10));
    assert_ne!(w.zone_of(ada), w.zone_of(bob));
    w.act_ok(bob, "rest", json!({ "ticks": 30 }));
    // A zone tells its neighbours what changed at the end of its tick, and
    // they take the news at the start of their next tick.
    w.step(2);
    // Across the border a resident shows its name, its place and what it
    // does. The zone of the observer does not know more.
    assert_eq!(
        done(w.act_ok(ada, "inspect", json!({ "resident": bob }))),
        json!({
            "resident": bob,
            "name": "Bob",
            "pos": { "x": 66, "y": 12 },
            "distance": 6,
            "activity": "rest",
            "zone": 1,
            "across_border": true,
        })
    );
    // Sight is 12 tiles, also across the border.
    assert_eq!(w.param("kernel.sight_radius"), 12);
    assert_eq!(w.act_err(ada, "inspect", json!({ "resident": far })).code, ErrorCode::NotHere);
    // A resident that sleeps sees one tile, also across the border.
    w.step(1);
    w.set_condition(ada, "asleep", None);
    assert_eq!(w.act_err(ada, "inspect", json!({ "resident": bob })).code, ErrorCode::NotHere);
}
