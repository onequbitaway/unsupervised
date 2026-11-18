//! A team mission: the members of an organization work together and share
//! the reward.

mod common;

use common::{
    bake, escrow, events, fail, fail_message, join, notice, ok, one_event, post, rich, world,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Money, ResidentId};
use unsup_missions::Missions;

/// The organization of the bakers.
const BAKERS: u64 = 5_000;
/// Another organization.
const MILLERS: u64 = 6_000;

/// A team mission: 4 loaves from any member, and one shift of the member
/// that speaks for the team.
fn team_mission(reward: i64, takers: u32) -> Value {
    json!({
        "title": "Bread for the fair",
        "objectives": [
            { "event": "economy.goods_produced", "fields": { "item": "bread" }, "count": 4, "actor": "team" },
            { "event": "economy.shift_completed", "count": 1 },
        ],
        "mode": "team",
        "reward": reward,
        "deadline_in": 600,
        "takers": takers,
    })
}

fn accept(w: &mut TestWorld<'_>, r: ResidentId, mission: u64, org: u64) -> Value {
    ok(w, r, "mission_accept", json!({ "mission": mission, "org": org }))
}

#[test]
fn a_team_shares_the_reward_by_the_events_of_each_member() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let dan = rich(&mut w, "Dan", 4, 0);
    let eve = rich(&mut w, "Eve", 5, 0);
    for member in [bob, cid, dan] {
        join(&mut w, member, BAKERS);
    }
    join(&mut w, eve, MILLERS);

    // Two teams can take it, so Ann locks the reward two times.
    let mission = post(&mut w, ann, team_mission(1_001, 2));
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(7_998), 2_002));

    // Bob accepts for the bakers and speaks for the team. Cid and Dan join
    // the same team; that takes no second place.
    let first = accept(&mut w, bob, mission, BAKERS);
    assert_eq!((&first["org"], &first["members"]), (&json!(BAKERS), &json!(1)));
    assert_eq!(accept(&mut w, cid, mission, BAKERS)["members"], json!(2));
    assert_eq!(accept(&mut w, dan, mission, BAKERS)["members"], json!(3));
    let listed = ok(&mut w, cid, "mission_list", json!({ "mission": mission }));
    assert_eq!(listed["mission"]["taken"], json!(1));
    assert_eq!(
        listed["mission"]["you"],
        json!({ "status": "accepted", "org": BAKERS, "assignee": bob, "members": 3, "progress": [[0, 4], [0, 1]] })
    );

    // Only a member can accept for an organization. The zone knows the
    // memberships, so the answer comes at once.
    let error = w.act_err(eve, "mission_accept", json!({ "mission": mission, "org": BAKERS }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(error.message, format!("You are not a member of organization {BAKERS}."));

    // Cid bakes 3 loaves and Bob bakes 1. The shift of Cid does not count:
    // that objective is for the member that speaks for the team.
    for _ in 0..3 {
        bake(&mut w, cid);
    }
    bake(&mut w, bob);
    w.act_ok(cid, "shift_test", Value::Null);
    w.step(2);
    assert_eq!((w.cash(bob), w.cash(cid), escrow(&w, mission)), (Money(0), Money(0), 2_002));
    let error = common::run_full(&mut w, cid, "mission_claim", json!({ "mission": mission }))
        .expect_err("one objective is open");
    assert_eq!(error.details, json!({ "progress": [[4, 4], [0, 1]] }));

    // A loaf above the count adds nothing to the share of Bob. Then the
    // shift of Bob completes the mission.
    bake(&mut w, bob);
    w.act_ok(bob, "shift_test", Value::Null);
    let frames = w.step(2);

    // Bob added 2 events and Cid 3. 1,001 * 2 / 5 = 400 and 1,001 * 3 / 5 =
    // 600. The cent that is left goes to the member with the most events.
    // Dan added nothing and gets nothing.
    assert_eq!((w.cash(bob), w.cash(cid), w.cash(dan)), (Money(400), Money(601), Money(0)));
    assert_eq!(escrow(&w, mission), 1_001, "the reward of the second place stays locked");
    assert_eq!(
        one_event(&frames, "missions.mission_completed"),
        json!({
            "mission": mission,
            "resident": bob,
            "org": BAKERS,
            "paid": 1_001,
            "shares": [{ "resident": bob, "amount": 400 }, { "resident": cid, "amount": 601 }],
        })
    );
    assert_eq!(
        notice(&w, cid, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 601, "points": 3 })
    );
    assert_eq!(
        notice(&w, dan, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 0, "points": 0 })
    );
    // One place is left, so the mission stays open, and a claim tells each
    // member what it got.
    assert!(events(&frames, "missions.mission_closed").is_empty());
    assert_eq!(
        ok(&mut w, cid, "mission_claim", json!({ "mission": mission })),
        json!({ "mission": mission, "status": "paid", "paid": 601, "frame": 15 })
    );
    assert_eq!(ok(&mut w, dan, "mission_claim", json!({ "mission": mission }))["paid"], json!(0));

    // A team cannot take the mission again. Another organization can.
    assert_eq!(
        fail_message(&mut w, dan, "mission_accept", json!({ "mission": mission, "org": BAKERS })),
        "Your organization completed this mission already."
    );
    assert_eq!(accept(&mut w, eve, mission, MILLERS)["members"], json!(1));
    assert_eq!(w.global().res::<Missions>().accepted_by(eve), [mission]);
    w.assert_conserved();
}

#[test]
fn a_team_mission_can_name_its_organization() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let eve = rich(&mut w, "Eve", 5, 0);
    join(&mut w, bob, BAKERS);
    join(&mut w, eve, MILLERS);
    let mut definition = team_mission(500, 1);
    definition["org"] = json!(BAKERS);
    let mission = post(&mut w, ann, definition);

    assert_eq!(
        fail_message(&mut w, eve, "mission_accept", json!({ "mission": mission, "org": MILLERS })),
        format!("Only organization {BAKERS} can take this mission.")
    );
    assert_eq!(
        fail(&mut w, bob, "mission_accept", json!({ "mission": mission })),
        ErrorCode::InvalidArgs,
        "a team mission needs the organization"
    );
    assert_eq!(accept(&mut w, bob, mission, BAKERS)["org"], json!(BAKERS));
    w.assert_conserved();
}

#[test]
fn a_member_that_leaves_gets_no_share_and_the_team_goes_on() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let dan = rich(&mut w, "Dan", 4, 0);
    for member in [bob, cid, dan] {
        join(&mut w, member, BAKERS);
    }
    let mission = post(&mut w, ann, team_mission(900, 1));
    for member in [bob, cid, dan] {
        accept(&mut w, member, mission, BAKERS);
    }

    // Bob speaks for the team and bakes 2 loaves. Then he leaves the
    // organization: the society module tells that with an event.
    bake(&mut w, bob);
    bake(&mut w, bob);
    bake(&mut w, cid);
    w.act_ok(bob, "leave_test", json!({ "org": BAKERS }));
    w.step(2);
    assert_eq!(w.global().res::<Missions>().accepted_by(bob), [] as [u64; 0]);
    assert_eq!(
        notice(&w, bob, "missions.mission_ended").data,
        json!({ "mission": mission, "reason": "left_organization" })
    );
    // The progress of the team stays. Cid has the lowest id of the members
    // that are left, so Cid speaks for the team now.
    let listed = ok(&mut w, dan, "mission_list", json!({ "mission": mission }));
    assert_eq!(
        listed["mission"]["you"],
        json!({ "status": "accepted", "org": BAKERS, "assignee": cid, "members": 2, "progress": [[3, 4], [0, 1]] })
    );

    // Dan gives the mission up with the verb. Cid ends the work alone: one
    // more loaf and its own shift.
    let frames = {
        ok(&mut w, dan, "mission_abandon", json!({ "mission": mission }));
        bake(&mut w, cid);
        w.act_ok(cid, "shift_test", Value::Null);
        w.step(2)
    };
    // Only the events of members that stayed count for the shares: Cid has
    // all 3 of them and gets the whole reward.
    assert_eq!(
        one_event(&frames, "missions.mission_completed")["shares"],
        json!([{ "resident": cid, "amount": 900 }])
    );
    assert_eq!((w.cash(bob), w.cash(cid), w.cash(dan)), (Money(0), Money(900), Money(0)));
    assert_eq!(escrow(&w, mission), 0);
    w.assert_conserved();
}

#[test]
fn the_last_member_that_leaves_gives_the_place_back() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let eve = rich(&mut w, "Eve", 5, 0);
    join(&mut w, bob, BAKERS);
    join(&mut w, eve, MILLERS);
    let mission = post(&mut w, ann, team_mission(900, 1));
    accept(&mut w, bob, mission, BAKERS);
    bake(&mut w, bob);
    w.step(2);
    assert_eq!(
        fail_message(&mut w, eve, "mission_accept", json!({ "mission": mission, "org": MILLERS })),
        "The mission has no free place."
    );

    let frames = {
        let ack = w.act(bob, "mission_abandon", json!({ "mission": mission }));
        assert_eq!(ack.verb, "mission_abandon");
        w.step(2)
    };
    assert_eq!(
        one_event(&frames, "missions.mission_abandoned"),
        json!({ "mission": mission, "resident": bob, "org": BAKERS })
    );
    // The place is free, and a team that takes it starts from zero.
    accept(&mut w, eve, mission, MILLERS);
    let listed = ok(&mut w, eve, "mission_list", json!({ "mission": mission }));
    assert_eq!(listed["mission"]["you"]["progress"], json!([[0, 4], [0, 1]]));
    assert_eq!(escrow(&w, mission), 900);
    w.assert_conserved();
}

#[test]
fn a_team_has_a_limit_of_members() {
    let mut w = world();
    w.set_param("missions.team_members_max", 2);
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let dan = rich(&mut w, "Dan", 4, 0);
    for member in [bob, cid, dan] {
        join(&mut w, member, BAKERS);
    }
    let mission = post(&mut w, ann, team_mission(900, 1));
    accept(&mut w, bob, mission, BAKERS);
    accept(&mut w, cid, mission, BAKERS);
    assert_eq!(
        fail_message(&mut w, dan, "mission_accept", json!({ "mission": mission, "org": BAKERS })),
        "The team is full. The limit is 2 members."
    );
    assert_eq!(
        fail(&mut w, dan, "mission_accept", json!({ "mission": mission, "org": BAKERS })),
        ErrorCode::Conflict
    );
    // A member that leaves makes room.
    ok(&mut w, cid, "mission_abandon", json!({ "mission": mission }));
    assert_eq!(accept(&mut w, dan, mission, BAKERS)["members"], json!(2));
    w.assert_conserved();
}
