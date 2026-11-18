//! The deadline of a mission: the unpaid reward returns to the poster.

mod common;

use common::{
    CITY, bake, bread_mission, count_notices, escrow, events, notice, ok, one_event, post, rich,
    world,
};
use serde_json::json;
use unsup_kernel::{Holder, Money, UNCLAIMED};
use unsup_missions::Missions;

#[test]
fn the_unpaid_reward_returns_to_the_poster_at_the_deadline() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);

    // Three places of 500 cents. The mission lives 20 game minutes.
    let mut definition = bread_mission(500, 2);
    definition["takers"] = json!(3);
    definition["deadline_in"] = json!(20);
    let mission = post(&mut w, ann, definition);
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(3_500), 1_500));
    // The global realm took the request in frame 1.
    assert_eq!(w.world.frame(), 2);
    let deadline = 21;

    // Bob completes it. Cid takes it and bakes only one loaf.
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    ok(&mut w, cid, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, bob);
    bake(&mut w, bob);
    bake(&mut w, cid);
    w.step(2);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(500), 1_000));

    // The frame before the deadline: nothing happens yet.
    let before = w.step((deadline - 1 - w.world.frame()) as u32);
    assert_eq!(w.world.frame(), deadline - 1);
    assert!(events(&before, "missions.mission_closed").is_empty());
    assert!(w.global().res::<Missions>().is_open(mission));

    // The frame of the deadline: the mission closes, and the 1,000 cents
    // that nobody earned go back to Ann.
    let frames = w.step(1);
    assert_eq!(
        one_event(&frames, "missions.mission_closed"),
        json!({ "mission": mission, "reason": "expired", "completed": 1, "refunded": 1_000, "destroyed": 0 })
    );
    assert_eq!(escrow(&w, mission), 0);
    assert_eq!(w.cash(ann), Money(3_500), "the refund is on its way");
    w.step(1);
    assert_eq!(w.cash(ann), Money(4_500));
    assert!(!w.global().res::<Missions>().is_open(mission));
    assert_eq!(
        notice(&w, ann, "missions.mission_closed").data,
        json!({ "mission": mission, "reason": "expired", "completed": 1, "refunded": 1_000 })
    );
    // Cid gets the news. Bob was paid and gets none.
    assert_eq!(
        notice(&w, cid, "missions.mission_ended").data,
        json!({ "mission": mission, "reason": "expired" })
    );
    assert_eq!(count_notices(&w, bob, "missions.mission_ended"), 0);
    assert_eq!(w.global().res::<Missions>().accepted_by(cid), [] as [u64; 0]);

    // Bread after the deadline pays nothing.
    bake(&mut w, cid);
    w.step(2);
    assert_eq!(w.cash(cid), Money(0));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.minted, totals.money.burned), (5_000, 0));
}

#[test]
fn an_event_in_the_frame_of_the_deadline_counts() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let mut definition = bread_mission(700, 1);
    definition["deadline_in"] = json!(10);
    let mission = post(&mut w, ann, definition);
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    let deadline = 11;
    w.step((deadline - 1 - w.world.frame()) as u32);

    // Bob bakes between frame 10 and frame 11. The global realm gets the
    // event in frame 11, before it closes the mission.
    bake(&mut w, bob);
    let frames = w.step(2);
    assert_eq!(one_event(&frames, "missions.mission_completed")["paid"], json!(700));
    assert_eq!(
        one_event(&frames, "missions.mission_closed"),
        json!({ "mission": mission, "reason": "completed", "completed": 1, "refunded": 0, "destroyed": 0 })
    );
    assert_eq!((w.cash(bob), w.cash(ann)), (Money(700), Money(4_300)));
    w.assert_conserved();
}

#[test]
fn an_event_one_frame_after_the_deadline_does_not_count() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let mut definition = bread_mission(700, 1);
    definition["deadline_in"] = json!(10);
    let mission = post(&mut w, ann, definition);
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    w.step((11 - w.world.frame()) as u32);

    bake(&mut w, bob);
    let frames = w.step(2);
    assert!(events(&frames, "missions.mission_completed").is_empty());
    assert_eq!((w.cash(bob), w.cash(ann)), (Money(0), Money(5_000)));
    w.assert_conserved();
}

#[test]
fn the_refund_of_a_dead_poster_goes_to_its_heir() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let mut definition = bread_mission(800, 1);
    definition["deadline_in"] = json!(20);
    let mission = post(&mut w, ann, definition);

    // Ann dies. Her will names Bob: he gets her cash at once, and the
    // reward that is still locked at the deadline.
    w.act_ok(ann, "die_test", json!({ "heir": bob }));
    w.step(2);
    assert!(!w.is_alive(ann));
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(4_200), 800));
    w.step((22 - w.world.frame()) as u32);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(5_000), 0));
    assert_eq!(w.money(UNCLAIMED), Money(0));
    w.assert_conserved();
}

#[test]
fn the_refund_of_a_dead_poster_without_a_resident_heir_goes_to_the_named_holder() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let mut definition = bread_mission(800, 1);
    definition["deadline_in"] = json!(20);
    let mission = post(&mut w, ann, definition);
    // No will: the city treasury inherits.
    w.act_ok(ann, "die_test", json!({}));
    w.step((22 - w.world.frame()) as u32);
    assert_eq!((w.money(CITY), escrow(&w, mission)), (Money(5_000), 0));
    w.assert_conserved();
}

#[test]
fn the_refund_of_a_removed_poster_goes_to_the_unclaimed_account() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let mut definition = bread_mission(800, 1);
    definition["deadline_in"] = json!(20);
    let mission = post(&mut w, ann, definition);
    // An administrator removes Ann. No module names an heir.
    w.despawn(ann);
    w.step((22 - w.world.frame()) as u32);
    assert_eq!((w.money(UNCLAIMED), escrow(&w, mission)), (Money(5_000), 0));
    assert_eq!(w.money(Holder::Resident(ann)), Money(0));
    w.assert_conserved();
}
