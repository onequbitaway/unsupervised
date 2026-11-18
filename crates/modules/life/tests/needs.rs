//! The hourly pass over food, energy, health and mood, and the reflexes of
//! an idle resident.

mod common;

use common::{with_params, world, world_with};
use unsup_kernel::protocol::Notice;
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Pos, ResidentId, ZoneId};
use unsup_life::{NEED_EMPTY, NEED_LOW};

const HOUR: u32 = 60;

/// Returns food, energy, health and mood of a resident.
fn needs(w: &TestWorld<'_>, r: ResidentId) -> [i64; 4] {
    ["need.food", "need.energy", "need.health", "need.mood"].map(|name| w.attr(r, name))
}

fn set_needs(w: &mut TestWorld<'_>, r: ResidentId, values: [i64; 4]) {
    for (name, value) in ["need.food", "need.energy", "need.health", "need.mood"].iter().zip(values)
    {
        w.set_attr(r, name, value);
    }
}

/// Returns the notices of the life module about needs.
fn alerts(w: &TestWorld<'_>, r: ResidentId) -> Vec<(String, String, i64)> {
    w.notices(r)
        .into_iter()
        .filter(|notice: &Notice| notice.kind == NEED_LOW || notice.kind == NEED_EMPTY)
        .map(|notice| {
            (
                notice.kind,
                notice.data["need"].as_str().expect("a need").to_string(),
                notice.data["value"].as_i64().expect("a value"),
            )
        })
        .collect()
}

/// A world without the reflexes and without the routines of a newcomer, so
/// that only the rates of the pass show.
fn bare() -> TestWorld<'static> {
    world_with(with_params(&[("life.reflexes", 0), ("life.spawn_routines", 0)]))
}

/// A world whose residents start without routines, so that only the
/// reflexes act for an idle resident.
fn reflexes_only() -> TestWorld<'static> {
    world_with(with_params(&[("life.spawn_routines", 0)]))
}

#[test]
fn a_newcomer_has_full_needs_and_is_18() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(needs(&w, ada), [1_000; 4]);
    assert_eq!(w.attr(ada, "age.years"), 18);
    assert_eq!(w.cash(ada).cents(), 0);
    assert!(w.world.realm(unsup_kernel::RealmId::Zone(w.zone_of(ada))).is_some());
}

#[test]
fn needs_change_once_in_each_game_hour_at_the_documented_rates() {
    let mut w = bare();
    let ada = w.spawn("Ada");
    // Nothing changes between two passes.
    w.step(HOUR - 5);
    let early = needs(&w, ada);
    assert!(early == [1_000; 4] || early == [986, 960, 1_000, 998], "{early:?}");
    // One pass in each hour: food 14, energy 40, mood 2. Health stays full.
    w.step(5);
    assert_eq!(needs(&w, ada), [986, 960, 1_000, 998]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [972, 920, 1_000, 996]);
    w.step(8 * HOUR);
    assert_eq!(needs(&w, ada), [1_000 - 10 * 14, 1_000 - 10 * 40, 1_000, 1_000 - 10 * 2]);
    assert!(alerts(&w, ada).is_empty());
}

#[test]
fn the_rates_are_parameters() {
    let mut w = world_with(with_params(&[
        ("life.reflexes", 0),
        ("life.food_decay_per_hour", 100),
        ("life.energy_decay_per_hour", 7),
        ("life.mood_decay_per_hour", 0),
    ]));
    let ada = w.spawn("Ada");
    w.step(2 * HOUR);
    assert_eq!(needs(&w, ada), [800, 986, 1_000, 1_000]);
}

#[test]
fn low_food_and_low_energy_hurt_mood_and_stop_healing() {
    let mut w = bare();
    let ada = w.spawn("Ada");

    // Both fine, health below full: health comes back, 5 for each hour.
    set_needs(&mut w, ada, [600, 600, 500, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [586, 560, 505, 498]);

    // Food below 250: mood falls 10 more, health does not come back.
    set_needs(&mut w, ada, [260, 600, 500, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [246, 560, 500, 488]);
    assert_eq!(alerts(&w, ada), [(NEED_LOW.to_string(), "food".to_string(), 246)]);

    // Both below 250: mood falls 2 + 10 + 10.
    set_needs(&mut w, ada, [200, 200, 500, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [186, 160, 500, 478]);
    // No second notice for food: it was low before.
    assert_eq!(alerts(&w, ada).len(), 1);
}

#[test]
fn no_food_and_no_energy_cost_health() {
    let mut w = bare();
    let ada = w.spawn("Ada");
    // No food: 20 health for each hour.
    set_needs(&mut w, ada, [10, 600, 500, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [0, 560, 480, 488]);
    assert_eq!(alerts(&w, ada), [(NEED_EMPTY.to_string(), "food".to_string(), 0)]);
    // No energy: 10 health for each hour.
    set_needs(&mut w, ada, [600, 30, 500, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [586, 0, 490, 488]);
    // Both: 30 health for each hour. Health falls below the low mark.
    set_needs(&mut w, ada, [0, 0, 260, 500]);
    w.step(HOUR);
    assert_eq!(needs(&w, ada), [0, 0, 230, 478]);
    let last = alerts(&w, ada).pop().expect("a notice");
    assert_eq!(last, (NEED_LOW.to_string(), "health".to_string(), 230));
    assert!(w.is_alive(ada));
}

#[test]
fn a_sleeping_resident_keeps_its_energy_and_a_patient_keeps_all_needs() {
    let mut w = bare();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    set_needs(&mut w, ada, [600, 100, 500, 500]);
    set_needs(&mut w, bob, [100, 100, 500, 500]);
    w.set_condition(ada, "asleep", None);
    w.set_condition(bob, "hospitalized", None);
    w.step(HOUR);
    // Asleep: food and mood change, energy does not, and low energy does
    // not count against mood. Health does not come back with low energy.
    assert_eq!(needs(&w, ada), [586, 100, 500, 498]);
    // In hospital: nothing changes.
    assert_eq!(needs(&w, bob), [100, 100, 500, 500]);
}

#[test]
fn an_idle_resident_eats_what_it_carries_and_dozes() {
    let mut w = reflexes_only();
    let ada = w.spawn("Ada");
    w.give_item(ada, "bread", 2);
    w.give_item(ada, "stone", 1);
    // Food at 310 falls to 296: below 300, the resident eats one bread.
    // Energy at 230 falls to 190: below 200, the resident dozes for 60.
    set_needs(&mut w, ada, [310, 230, 1_000, 1_000]);
    w.step(HOUR);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 1);
    assert_eq!(w.qty(Holder::Resident(ada), "stone"), 1);
    // Mood: 2 for the hour and 10 for low energy (food at 296 is not below
    // the low mark of 250), and the meal gives 5. The reflexes come after
    // the pass.
    assert_eq!(needs(&w, ada), [296 + 300, 190 + 60, 1_000, 1_000 - 12 + 5]);
    w.assert_conserved();

    // A busy resident has no reflexes: its owner decides.
    set_needs(&mut w, ada, [310, 230, 1_000, 1_000]);
    w.act_ok(ada, "wait", json!({ "ticks": 2 * HOUR }));
    w.step(HOUR);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 1);
    assert_eq!(needs(&w, ada)[..2], [296, 190]);
}

#[test]
fn an_idle_resident_with_food_stays_alive_for_weeks() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.give_item(ada, "bread", 40);
    // Thirty game days. Ada has food; Bob has none.
    for _ in 0..30 {
        w.step(24 * HOUR);
    }
    assert!(w.is_alive(ada));
    assert!(!w.is_alive(bob));
    // Ada ate about 336 food points a day: a bit more than one bread.
    let left = w.qty(Holder::Resident(ada), "bread");
    assert!((3..=8).contains(&left), "{left} breads left");
    let [food, energy, health, _] = needs(&w, ada);
    assert!(food >= 286, "food {food}");
    assert!(energy >= 160, "energy {energy}");
    assert_eq!(health, 1_000);
    w.assert_conserved();
}

#[test]
fn without_reflexes_an_idle_resident_does_not_eat() {
    let mut w = bare();
    let ada = w.spawn("Ada");
    w.give_item(ada, "bread", 2);
    set_needs(&mut w, ada, [310, 230, 1_000, 1_000]);
    w.step(HOUR);
    assert_eq!(w.qty(Holder::Resident(ada), "bread"), 2);
    assert_eq!(needs(&w, ada)[..2], [296, 190]);
}

/// Runs frames until the world is at a frame number.
fn run_to(w: &mut TestWorld<'_>, frame: u64) {
    let now = w.world.frame();
    assert!(frame >= now, "frame {frame} is in the past of frame {now}");
    w.step((frame - now) as u32);
}

#[test]
fn a_resident_cannot_keep_away_from_the_hourly_pass_of_every_zone() {
    let mut w = bare();
    // Zone 0 has its pass in the frames 60, 120, ... and zone 1 in the
    // frames 59, 119, ... Ada is in zone 0 during the pass of zone 1 and in
    // zone 1 during the pass of zone 0. Bob stays in zone 0.
    let (in_zone_0, in_zone_1) = (Pos::new(63, 0), Pos::new(64, 0));
    let ada = w.spawn_at("Ada", in_zone_0);
    let bob = w.spawn_at("Bob", in_zone_0);
    assert_eq!((w.zone_of(ada), w.zone_of(bob)), (ZoneId(0), ZoneId(0)));
    for hour in 1..=10u64 {
        run_to(&mut w, 60 * hour - 1);
        w.set_pos(ada, in_zone_1);
        assert_eq!(w.zone_of(ada), ZoneId(1));
        run_to(&mut w, 60 * hour);
        w.set_pos(ada, in_zone_0);
        run_to(&mut w, 60 * hour + 1);
        // Both paid one hour for each hour.
        let paid = 1_000 - 14 * hour as i64;
        assert_eq!(w.attr(bob, "need.food"), paid, "Bob after {hour} hours");
        assert_eq!(w.attr(ada, "need.food"), paid, "Ada after {hour} hours");
        assert_eq!(w.attr(ada, "need.energy"), 1_000 - 40 * hour as i64);
    }
}

#[test]
fn a_resident_that_changes_zone_often_pays_one_hour_for_each_hour() {
    let mut w = bare();
    let corners = [Pos::new(63, 63), Pos::new(64, 63), Pos::new(64, 64), Pos::new(63, 64)];
    let ada = w.spawn_at("Ada", corners[0]);
    // A new zone every seven frames, for ten hours: 85 arrivals, and the
    // pass of one of the four zones every few frames.
    for hop in 1..=85usize {
        w.step(7);
        w.set_pos(ada, corners[hop % 4]);
    }
    w.step(5);
    assert_eq!(w.world.frame(), 600);
    assert_eq!(needs(&w, ada), [1_000 - 10 * 14, 1_000 - 10 * 40, 1_000, 1_000 - 10 * 2]);
}

#[test]
fn a_resident_that_owes_an_hour_pays_when_it_arrives_and_can_die_of_it() {
    let mut w = bare();
    let ada = w.spawn_at("Ada", Pos::new(63, 0));
    w.give_money(ada, unsup_kernel::Money(700));
    // Half an hour in zone 0, where the pass is 30 frames away.
    w.step(30);
    set_needs(&mut w, ada, [0, 600, 15, 500]);
    w.set_pos(ada, Pos::new(64, 0));
    // The hour was due: no food costs 20 health, and Ada has 15.
    let events: Vec<_> = w.step(1).into_iter().flat_map(|frame| frame.events).collect();
    assert!(!w.is_alive(ada));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["cause"], json!("starvation"));
    assert_eq!(w.money(common::treasury(0)), unsup_kernel::Money(700));
    w.assert_conserved();
}
