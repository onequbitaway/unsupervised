//! Skills: the level formula, the attributes and the verb `train`.

mod common;

use common::{report, result, site, with_params, world, world_with};
use unsup_kernel::protocol::{ActionStatus, ErrorCode, Event};
use unsup_kernel::serde_json::json;
use unsup_kernel::{Holder, Money, Pos};
use unsup_life::names::SKILLS;
use unsup_life::{skill_attr, skill_level, xp_for_level};

#[test]
fn the_level_is_the_floor_of_the_root_of_a_hundredth_of_the_experience() {
    for (xp, level) in [
        (-5, 0),
        (0, 0),
        (99, 0),
        (100, 1),
        (399, 1),
        (400, 2),
        (899, 2),
        (900, 3),
        (9_999, 9),
        (10_000, 10),
        (1_000_000, 100),
    ] {
        assert_eq!(skill_level(xp), level, "{xp} experience");
    }
    for level in 0..50 {
        assert_eq!(skill_level(xp_for_level(level)), level);
        assert_eq!(skill_level(xp_for_level(level + 1) - 1), level);
    }
    assert_eq!(xp_for_level(3), 900);
}

#[test]
fn every_skill_of_the_game_design_is_an_attribute() {
    let mut w = world();
    let ada = w.spawn("Ada");
    assert_eq!(SKILLS.len(), 13);
    for skill in SKILLS {
        assert_eq!(w.attr(ada, &skill_attr(skill)), 0, "{skill}");
    }
    assert_eq!(skill_attr("farming"), "skill.farming");
    // The catalogue lists them, so that a routine can test them.
    let catalogue = w.world.catalogue();
    for skill in SKILLS {
        assert!(catalogue.attrs.iter().any(|attr| attr.name == skill_attr(skill)), "{skill}");
    }
}

#[test]
fn a_lesson_costs_a_fee_and_time_and_gives_experience() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.give_money(ada, Money(10_000));
    let (_, school) = site(&w, "school");
    w.set_pos(ada, school.rect.centre());

    // Two hours: 2,000 cents to the till of the school, 120 experience.
    let status = w.act_ok(ada, "train", json!({ "skill": "farming", "ticks": 120 }));
    assert!(matches!(status, ActionStatus::Started { ticks: 120, .. }), "{status:?}");
    assert_eq!(w.cash(ada), Money(8_000));
    assert_eq!(w.money(Holder::Site(school.id)), Money(2_000));
    w.step(119);
    assert_eq!(w.attr(ada, "skill.farming"), 0);
    let events: Vec<Event> = w.step(1).into_iter().flat_map(|frame| frame.events).collect();
    // Mood after two hours is 996: 99.8 percent of 120 points, rounded down.
    assert_eq!(w.attr(ada, "need.mood"), 996);
    assert_eq!(result(&w, ada), json!({ "skill": "farming", "xp": 119, "total": 119, "level": 1 }));
    assert_eq!(w.attr(ada, "skill.farming"), 119);
    let kinds: Vec<&str> = events
        .iter()
        .map(|event| event.kind.as_str())
        .filter(|kind| kind.starts_with("life.skill") || kind.starts_with("life.level"))
        .collect();
    assert_eq!(kinds, ["life.skill_trained", "life.level_gained"]);
    w.assert_conserved();

    // Without a length: the shortest lesson, one hour.
    let status = w.act_ok(ada, "train", json!({ "skill": "law" }));
    assert!(matches!(status, ActionStatus::Started { ticks: 60, .. }), "{status:?}");
    assert_eq!(w.cash(ada), Money(7_000));
}

#[test]
fn a_bad_mood_slows_learning() {
    let mut w = world_with(with_params(&[("life.mood_decay_per_hour", 0), ("life.reflexes", 0)]));
    let ada = w.spawn("Ada");
    w.give_money(ada, Money(10_000));
    let (_, school) = site(&w, "school");
    w.set_pos(ada, school.rect.centre());
    for (mood, xp) in [(1_000, 60), (500, 45), (0, 30)] {
        w.set_attr(ada, "need.mood", mood);
        w.set_attr(ada, "need.food", 1_000);
        w.set_attr(ada, "need.energy", 1_000);
        w.act_ok(ada, "train", json!({ "skill": "combat", "ticks": 60 }));
        w.set_attr(ada, "need.mood", mood);
        w.run_until_idle(ada, 100);
        assert_eq!(result(&w, ada)["xp"], xp, "mood {mood}");
    }
}

#[test]
fn train_refusals() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let (_, school) = site(&w, "school");

    w.set_pos(ada, Pos::new(0, 0));
    w.give_money(ada, Money(500));
    let error = w.act_err(ada, "train", json!({ "skill": "farming" }));
    assert_eq!(error.code, ErrorCode::NotHere);

    w.set_pos(ada, school.rect.centre());
    let error = w.act_err(ada, "train", json!({ "skill": "juggling" }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
    assert_eq!(error.details["skills"].as_array().map(Vec::len), Some(13));
    for ticks in [0, 59, 481] {
        let error = w.act_err(ada, "train", json!({ "skill": "farming", "ticks": ticks }));
        assert_eq!(error.code, ErrorCode::InvalidArgs);
    }
    // One hour costs 1,000 cents; Ada has 500.
    let error = w.act_err(ada, "train", json!({ "skill": "farming" }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(w.cash(ada), Money(500));
    assert_eq!(w.money(Holder::Site(school.id)), Money::ZERO);
    assert_eq!(w.activity(ada), None);
}

#[test]
fn a_lesson_that_is_cut_short_gives_nothing_and_the_fee_is_gone() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.give_money(ada, Money(1_000));
    let (_, school) = site(&w, "school");
    w.set_pos(ada, school.rect.centre());
    w.act_ok(ada, "train", json!({ "skill": "farming" }));
    w.step(30);
    w.act_ok(ada, "cancel", json!({}));
    assert_eq!(report(&w, ada).error.expect("an error").code, ErrorCode::Interrupted);
    assert_eq!(w.attr(ada, "skill.farming"), 0);
    assert_eq!(w.cash(ada), Money::ZERO);
    assert_eq!(w.money(Holder::Site(school.id)), Money(1_000));
}

#[test]
fn a_student_that_something_moves_out_of_the_school_learns_nothing() {
    let mut w = world();
    let ada = w.spawn("Ada");
    w.give_money(ada, Money(1_000));
    let (_, school) = site(&w, "school");
    w.set_pos(ada, school.rect.centre());
    w.act_ok(ada, "train", json!({ "skill": "farming" }));
    w.step(30);
    // Something takes the student out and does not stop the lesson.
    w.set_pos(ada, Pos::new(0, 0));
    w.run_until_idle(ada, 60);
    let failed = report(&w, ada).error.expect("an error");
    assert_eq!(failed.code, ErrorCode::Conflict);
    assert_eq!(w.attr(ada, "skill.farming"), 0);
}
