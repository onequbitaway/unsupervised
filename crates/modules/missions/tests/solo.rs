//! A solo mission from its publication to the automatic payment.

mod common;

use common::{bake, bread_mission, escrow, events, notice, ok, one_event, rich, world};
use serde_json::json;
use unsup_kernel::Money;
use unsup_missions::Missions;

#[test]
fn a_solo_mission_pays_by_itself_when_the_objective_is_met() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);

    // Ann publishes: 3 loaves for 1,200 cents. The reward leaves her cash
    // and is locked in the escrow of the mission in the global realm.
    let ack = w.act(ann, "mission_post", bread_mission(1_200, 3));
    assert_eq!(w.cash(ann), Money(3_800), "the reward left the cash at once");
    let frames = w.step(2);
    let report = w.last_report(ann).expect("the answer came");
    assert_eq!(report.action_id, ack.action_id);
    let mission = report.result["mission"].as_u64().expect("the id");
    assert_eq!(report.result, json!({ "mission": mission, "locked": 1_200, "deadline": 601 }));
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(3_800), 1_200));
    assert_eq!(
        one_event(&frames, "missions.mission_posted"),
        json!({
            "mission": mission,
            "title": { "untrusted": true, "from": ann, "text": "Bake bread" },
            "reward": 1_200,
            "takers": 1,
            "mode": "solo",
            "deadline": 601,
            "objectives": 1,
            "poster": ann,
        })
    );
    w.assert_conserved();

    // Bob takes it.
    let accepted = ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    assert_eq!(accepted["status"], json!("accepted"));
    assert_eq!(accepted["deadline"], json!(601));
    assert_eq!(w.global().res::<Missions>().accepted_by(bob), [mission]);

    // Two loaves: nothing is paid yet. Another good and the bread of
    // another resident do not count.
    bake(&mut w, bob);
    bake(&mut w, bob);
    w.act_ok(bob, "produce_test", json!({ "item": "cake", "qty": 1 }));
    bake(&mut w, ann);
    w.step(2);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(0), 1_200));
    let error = common::run_full(&mut w, bob, "mission_claim", json!({ "mission": mission }))
        .expect_err("the objectives are not met");
    assert_eq!(error.details, json!({ "progress": [[2, 3]] }));

    // The third loaf: the engine pays in the frame of the event, and the
    // cash arrives one frame later.
    bake(&mut w, bob);
    let frames = w.step(2);
    assert_eq!((w.cash(bob), w.cash(ann), escrow(&w, mission)), (Money(1_200), Money(3_800), 0));
    assert_eq!(
        one_event(&frames, "missions.mission_completed"),
        json!({
            "mission": mission,
            "resident": bob,
            "paid": 1_200,
            "shares": [{ "resident": bob, "amount": 1_200 }],
        })
    );
    // The mission had one place, so it is closed. Nothing is left to give
    // back.
    assert_eq!(
        one_event(&frames, "missions.mission_closed"),
        json!({ "mission": mission, "reason": "completed", "completed": 1, "refunded": 0, "destroyed": 0 })
    );
    assert_eq!(events(&frames, "missions.mission_completed")[0].actor, Some(bob));
    assert!(!w.global().res::<Missions>().is_open(mission));
    assert_eq!(w.global().res::<Missions>().accepted_by(bob), [] as [u64; 0]);

    // Both get the news. A mission of a resident gives 3 points.
    assert_eq!(
        notice(&w, bob, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 1_200, "points": 3 })
    );
    assert_eq!(notice(&w, ann, "missions.mission_paid").data["paid"], json!(1_200));
    assert_eq!(
        notice(&w, ann, "missions.mission_closed").data,
        json!({ "mission": mission, "reason": "completed", "completed": 1, "refunded": 0 })
    );

    // More bread changes nothing.
    bake(&mut w, bob);
    w.step(2);
    assert_eq!(w.cash(bob), Money(1_200));
    let totals = w.assert_conserved();
    assert_eq!((totals.money.minted, totals.money.held), (5_000, 5_000));
}
