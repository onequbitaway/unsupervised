//! Ageing and death of old age.

mod common;

use common::{add_params, world_with};
use unsup_kernel::protocol::Event;
use unsup_kernel::serde_json::json;
use unsup_kernel::{Pos, WorldConfig};

const DAY: u32 = 1_440;

/// A world where one year is one day, without needs that kill first.
fn short_years(params: &[(&str, i64)]) -> WorldConfig {
    let mut config = WorldConfig { days_per_year: 1, ..WorldConfig::default() };
    add_params(
        &mut config,
        &[
            ("life.food_decay_per_hour", 0),
            ("life.energy_decay_per_hour", 0),
            ("life.mood_decay_per_hour", 0),
        ],
    );
    add_params(&mut config, params);
    config
}

#[test]
fn age_grows_with_the_calendar() {
    let mut w = world_with(short_years(&[]));
    // Zone 0 looks at ages when its tick is a whole day.
    let ada = w.spawn_at("Ada", Pos::new(0, 0));
    assert_eq!(w.attr(ada, "age.years"), 18);
    w.step(DAY - 1);
    assert_eq!(w.attr(ada, "age.years"), 18);
    let events: Vec<Event> = w.step(1).into_iter().flat_map(|frame| frame.events).collect();
    assert_eq!(w.attr(ada, "age.years"), 19);
    let birthday = events.iter().find(|e| e.kind == "life.birthday_reached").expect("a birthday");
    assert_eq!(birthday.data, json!({ "resident": ada, "age": 19 }));
    w.step(3 * DAY);
    assert_eq!(w.attr(ada, "age.years"), 22);

    // A resident that is born later is younger.
    let bob = w.spawn_at("Bob", Pos::new(1, 0));
    w.step(2 * DAY);
    assert_eq!(w.attr(ada, "age.years"), 24);
    assert_eq!(w.attr(bob, "age.years"), 20);
    assert!(w.is_alive(ada));
}

#[test]
fn with_the_normal_calendar_a_year_is_28_days() {
    let mut config = WorldConfig::default();
    add_params(&mut config, &[("life.food_decay_per_hour", 0), ("life.energy_decay_per_hour", 0)]);
    let mut w = world_with(config);
    let ada = w.spawn_at("Ada", Pos::new(0, 0));
    w.step(27 * DAY);
    assert_eq!(w.attr(ada, "age.years"), 18);
    w.step(DAY);
    assert_eq!(w.attr(ada, "age.years"), 19);
}

#[test]
fn nobody_dies_of_old_age_before_the_age_of_the_data_file() {
    // The chance is certain from 21. Before that it is nothing.
    let mut w =
        world_with(short_years(&[("life.old_age_from", 21), ("life.old_age_death_bp", 10_000)]));
    let ada = w.spawn_at("Ada", Pos::new(0, 0));
    w.step(2 * DAY);
    assert_eq!(w.attr(ada, "age.years"), 20);
    assert!(w.is_alive(ada));
    let events: Vec<Event> = w.step(DAY).into_iter().flat_map(|frame| frame.events).collect();
    assert!(!w.is_alive(ada));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["cause"], json!("old_age"));
    assert_eq!(died.data["age"], json!(21));
    w.assert_conserved();
}

#[test]
fn the_chance_of_death_grows_with_each_year() {
    // No chance at 19, and 5,000 basis points more with each year: half at
    // 20 and certain at 21. With one day in a year, a resident gets one
    // roll at each age.
    let mut w = world_with(short_years(&[
        ("life.old_age_from", 19),
        ("life.old_age_death_bp", 0),
        ("life.old_age_death_rise_bp", 5_000),
    ]));
    let residents: Vec<_> =
        (0..200).map(|i| w.spawn_at(&format!("r{i}"), Pos::new(i % 60, 0))).collect();
    let alive = |w: &unsup_kernel::testing::TestWorld<'_>| {
        residents.iter().filter(|r| w.is_alive(**r)).count()
    };
    w.step(DAY);
    assert_eq!(alive(&w), 200, "age 19: no chance");
    w.step(DAY);
    let at_20 = alive(&w);
    assert!((70..=130).contains(&at_20), "age 20: {at_20} of 200 alive, about half expected");
    w.step(DAY);
    assert_eq!(alive(&w), 0, "age 21: certain");
}

#[test]
fn a_resident_cannot_keep_away_from_the_daily_pass_of_every_zone() {
    // Death is certain from the age of 19, and a year is one day.
    let mut w =
        world_with(short_years(&[("life.old_age_from", 19), ("life.old_age_death_bp", 10_000)]));
    // Zone 1 has its daily pass in frame 1,439 and zone 0 in frame 1,440.
    // Ada is in zone 0 during the first and in zone 1 during the second.
    let ada = w.spawn_at("Ada", Pos::new(63, 0));
    w.step(DAY - 1);
    w.set_pos(ada, Pos::new(64, 0));
    // Not a year old yet: the arrival changes nothing.
    assert_eq!(w.attr(ada, "age.years"), 18);
    w.step(1);
    assert!(w.is_alive(ada));
    // Back in zone 0 after its pass. The arrival gives the birthday and
    // the chance of the day.
    w.set_pos(ada, Pos::new(63, 0));
    let events: Vec<Event> = w.step(1).into_iter().flat_map(|frame| frame.events).collect();
    assert!(!w.is_alive(ada));
    let birthday = events.iter().find(|e| e.kind == "life.birthday_reached").expect("a birthday");
    assert_eq!(birthday.data, json!({ "resident": ada, "age": 19 }));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["cause"], json!("old_age"));
}

#[test]
fn a_resident_that_changes_zone_has_one_chance_of_death_in_each_day() {
    // Half of the residents of 19 die in each day.
    let mut w =
        world_with(short_years(&[("life.old_age_from", 19), ("life.old_age_death_bp", 5_000)]));
    let residents: Vec<_> =
        (0..200).map(|i| w.spawn_at(&format!("r{i}"), Pos::new(i % 60, 0))).collect();
    let alive = |w: &unsup_kernel::testing::TestWorld<'_>| -> Vec<_> {
        residents.iter().copied().filter(|r| w.is_alive(*r)).collect()
    };
    // The day of the birthday: the pass of zone 0 gives one chance.
    w.step(DAY);
    let after_pass = alive(&w).len();
    assert!((70..=130).contains(&after_pass), "{after_pass} of 200 alive, about half expected");
    // Later in the same day they change zone four times. Each arrival
    // would be one more chance, but the day has only one.
    for step in 1..=4 {
        w.step(10);
        for resident in alive(&w) {
            let pos = w.pos(resident);
            w.set_pos(resident, Pos::new(pos.x, if step % 2 == 1 { 64 } else { 0 }));
        }
        w.step(1);
    }
    assert_eq!(alive(&w).len(), after_pass);
}
