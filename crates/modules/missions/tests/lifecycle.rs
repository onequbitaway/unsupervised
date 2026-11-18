//! How the engine counts, and what happens when a taker leaves the world.

mod common;

use common::{
    bake, bread_mission, escrow, events, fail_message, join, notice, ok, one_event, post, rich,
    run_full, world,
};
use serde_json::{Value, json};
use unsup_kernel::Money;
use unsup_missions::{Boards, Missions};

#[test]
fn a_solo_taker_that_leaves_the_world_gives_its_place_back() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let mission = post(&mut w, ann, bread_mission(500, 2));
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, bob);
    w.step(1);
    assert_eq!(
        fail_message(&mut w, cid, "mission_accept", json!({ "mission": mission })),
        "The mission has no free place."
    );

    w.act_ok(bob, "die_test", json!({}));
    w.step(2);
    assert_eq!(w.global().res::<Missions>().accepted_by(bob), [] as [u64; 0]);
    // Cid takes the place and starts from zero.
    ok(&mut w, cid, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, cid);
    w.step(2);
    assert_eq!((w.cash(cid), escrow(&w, mission)), (Money(0), 500));
    bake(&mut w, cid);
    w.step(2);
    assert_eq!((w.cash(cid), escrow(&w, mission)), (Money(500), 0));
    w.assert_conserved();
}

#[test]
fn a_team_goes_on_without_a_member_that_left_the_world() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    join(&mut w, bob, 5_000);
    join(&mut w, cid, 5_000);
    let mission = post(
        &mut w,
        ann,
        json!({
            "title": "Bread",
            "objectives": [{ "event": "economy.goods_produced", "count": 4, "actor": "team" }],
            "mode": "team",
            "reward": 1_000,
            "deadline_in": 600,
        }),
    );
    for member in [bob, cid] {
        ok(&mut w, member, "mission_accept", json!({ "mission": mission, "org": 5_000 }));
    }
    // Bob speaks for the team and adds 3 of the 4 events. Then Bob dies.
    for _ in 0..3 {
        bake(&mut w, bob);
    }
    w.act_ok(bob, "die_test", json!({}));
    w.step(2);
    let read = ok(&mut w, cid, "mission_list", json!({ "mission": mission }));
    assert_eq!(
        read["mission"]["you"],
        json!({ "status": "accepted", "org": 5_000, "assignee": cid, "members": 1, "progress": [[3, 4]] })
    );

    // Cid adds the last event and gets the whole reward: only members that
    // are still in the team share it.
    bake(&mut w, cid);
    let frames = w.step(2);
    assert_eq!(
        one_event(&frames, "missions.mission_completed")["shares"],
        json!([{ "resident": cid, "amount": 1_000 }])
    );
    assert_eq!((w.cash(cid), escrow(&w, mission)), (Money(1_000), 0));
    w.assert_conserved();
}

#[test]
fn the_data_fields_of_an_objective_must_be_equal() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    // Two events with exactly 5 loaves at site 7.
    let mission = post(
        &mut w,
        ann,
        json!({
            "title": "Big batches",
            "objectives": [{
                "event": "economy.goods_produced",
                "fields": { "item": "bread", "qty": 5, "site": 7 },
                "count": 2,
            }],
            "reward": 600,
            "deadline_in": 600,
        }),
    );
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    for (item, qty) in [("bread", 4), ("cake", 5), ("bread", 5), ("Bread", 5), ("bread", 50)] {
        w.act_ok(bob, "produce_test", json!({ "item": item, "qty": qty }));
    }
    // Another kind of event, and an event that no mission can count.
    w.act_ok(bob, "shift_test", Value::Null);
    w.act_ok(bob, "say_test", Value::Null);
    w.step(2);
    let error = run_full(&mut w, bob, "mission_claim", json!({ "mission": mission }))
        .expect_err("one batch counted");
    assert_eq!(error.details, json!({ "progress": [[1, 2]] }));
    w.act_ok(bob, "produce_test", json!({ "item": "bread", "qty": 5 }));
    w.step(2);
    assert_eq!(w.cash(bob), Money(600));
    w.assert_conserved();
}

#[test]
fn one_event_counts_for_every_objective_and_mission_that_it_matches() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    // Two objectives that the same event matches: any good, and bread.
    let both = post(
        &mut w,
        ann,
        json!({
            "title": "Goods and bread",
            "objectives": [
                { "event": "economy.goods_produced", "count": 2 },
                { "event": "economy.goods_produced", "fields": { "item": "bread" } },
            ],
            "reward": 300,
            "deadline_in": 600,
        }),
    );
    let other = post(&mut w, ann, bread_mission(200, 1));
    ok(&mut w, bob, "mission_accept", json!({ "mission": both }));
    ok(&mut w, bob, "mission_accept", json!({ "mission": other }));

    // One loaf: it completes the second mission, and it counts for both
    // objectives of the first one.
    bake(&mut w, bob);
    w.step(2);
    assert_eq!(w.cash(bob), Money(200));
    let error = run_full(&mut w, bob, "mission_claim", json!({ "mission": both }))
        .expect_err("one good is missing");
    assert_eq!(error.details, json!({ "progress": [[1, 2], [1, 1]] }));
    w.act_ok(bob, "produce_test", json!({ "item": "cake", "qty": 1 }));
    w.step(2);
    assert_eq!(w.cash(bob), Money(500));
    w.assert_conserved();
}

#[test]
fn a_mission_can_count_the_events_of_this_module() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    // A mission about missions: complete two other missions.
    let meta = post(
        &mut w,
        ann,
        json!({
            "title": "Complete two missions",
            "objectives": [{ "event": "missions.mission_completed", "count": 2 }],
            "reward": 1_000,
            "deadline_in": 600,
        }),
    );
    let first = post(&mut w, ann, bread_mission(200, 1));
    let second = post(
        &mut w,
        ann,
        json!({ "title": "A shift", "objectives": [{ "event": "economy.shift_completed" }], "reward": 300, "deadline_in": 600 }),
    );
    for mission in [meta, first, second] {
        ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    }

    // Both small missions end in the same frame. Each completion is an
    // event, and the engine counts it in the same frame too.
    bake(&mut w, bob);
    w.act_ok(bob, "shift_test", Value::Null);
    let frames = w.step(2);
    let completed: Vec<u64> = events(&frames, "missions.mission_completed")
        .iter()
        .map(|event| event.data["mission"].as_u64().expect("an id"))
        .collect();
    assert_eq!(completed, [first, second, meta]);
    assert_eq!(w.cash(bob), Money(1_500));
    assert_eq!(w.global().res::<Missions>().open_count(), 0);
    w.assert_conserved();
}

#[test]
fn points_from_missions_of_residents_have_a_limit_for_each_day() {
    let mut w = world();
    w.set_param("missions.posted_max", 10);
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let complete = |w: &mut unsup_kernel::testing::TestWorld<'_>| {
        let mission = post(w, ann, bread_mission(100, 1));
        ok(w, bob, "mission_accept", json!({ "mission": mission }));
        bake(w, bob);
        w.step(2);
        notice(w, bob, "missions.mission_completed").data["points"].as_i64().expect("points")
    };

    // 3 points for each mission, and at most 6 in one game day.
    assert_eq!((complete(&mut w), complete(&mut w), complete(&mut w)), (3, 3, 0));
    assert_eq!(w.global().res::<Boards>().points(bob), 6);
    // The reward is paid all the same.
    assert_eq!(w.cash(bob), Money(300));

    // The next game day has a new limit.
    w.step((1_440 - w.world.frame()) as u32);
    assert_eq!(complete(&mut w), 3);
    assert_eq!(w.global().res::<Boards>().points(bob), 9);

    // The numbers are parameters.
    w.set_param("missions.points_resident", 5);
    w.set_param("missions.points_per_day", 9);
    assert_eq!((complete(&mut w), complete(&mut w)), (5, 1));
    assert_eq!(w.global().res::<Boards>().points(bob), 15);
    w.assert_conserved();
}
