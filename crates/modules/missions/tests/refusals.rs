//! The refusals of each verb. A refused request changes nothing and keeps
//! no money.

mod common;

use common::{
    bake, bread_mission, escrow, fail, fail_message, join, ok, one_event, post, rich, run_full,
    world,
};
use serde_json::{Value, json};
use unsup_kernel::{ErrorCode, Money};
use unsup_missions::Missions;

#[test]
fn mission_post_refusals() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 1_000);

    // The poster locks the reward for every taker: 3 * 400 cents.
    let mut definition = bread_mission(400, 1);
    definition["takers"] = json!(3);
    let error = w.act_err(ann, "mission_post", definition.clone());
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(
        error.message,
        "The mission locks 1200 cents: the reward for each of its takers. You have 1000."
    );
    assert_eq!(w.cash(ann), Money(1_000));

    // The limit of open missions of one poster. The global realm refuses,
    // and the cash that left with the request comes back.
    w.set_param("missions.posted_max", 1);
    let first = post(&mut w, ann, bread_mission(400, 1));
    let ack = w.act(ann, "mission_post", bread_mission(400, 1));
    assert_eq!(w.cash(ann), Money(200), "the cash is on its way");
    w.step(2);
    let report = w.last_report(ann).expect("the answer came");
    assert_eq!(report.action_id, ack.action_id);
    let error = report.error.expect("the request was refused");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.message, "You have too many open missions. The limit is 1.");
    assert_eq!((w.cash(ann), escrow(&w, first)), (Money(600), 400));
    assert_eq!(w.global().res::<Missions>().open_count(), 1);

    // The limit of open missions of the world.
    let bob = rich(&mut w, "Bob", 2, 1_000);
    w.set_param("missions.open_max", 1);
    assert_eq!(
        fail_message(&mut w, bob, "mission_post", bread_mission(400, 1)),
        "The world has 1 open missions already. Try again later."
    );
    assert_eq!(w.cash(bob), Money(1_000));

    // A resident in jail cannot publish.
    w.set_param("missions.open_max", 10);
    w.act_ok(bob, "jail_me", Value::Null);
    assert_eq!(w.act_err(bob, "mission_post", bread_mission(400, 1)).code, ErrorCode::Blocked);
    w.assert_conserved();
}

#[test]
fn mission_accept_refusals() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    join(&mut w, bob, 5_000);
    let solo = post(&mut w, ann, bread_mission(400, 1));

    assert_eq!(
        fail(&mut w, bob, "mission_accept", json!({ "mission": 12_345 })),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": 12_345 })),
        "No open mission has the id 12345."
    );
    assert_eq!(
        fail_message(&mut w, ann, "mission_accept", json!({ "mission": solo })),
        "You cannot accept your own mission."
    );
    assert_eq!(
        fail(&mut w, ann, "mission_accept", json!({ "mission": solo })),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": solo, "org": 5_000 })),
        "This is a solo mission. Do not give `org`."
    );
    assert_eq!(
        w.act_err(bob, "mission_accept", json!({ "mission": solo, "team": 5_000 })).code,
        ErrorCode::InvalidArgs,
        "an unknown argument"
    );

    // One place: the second taker finds it full, and the first cannot take
    // it twice.
    ok(&mut w, bob, "mission_accept", json!({ "mission": solo }));
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": solo })),
        "You work on this mission already."
    );
    assert_eq!(
        fail(&mut w, bob, "mission_accept", json!({ "mission": solo })),
        ErrorCode::Conflict
    );
    assert_eq!(
        fail_message(&mut w, cid, "mission_accept", json!({ "mission": solo })),
        "The mission has no free place."
    );
    assert_eq!(
        fail(&mut w, cid, "mission_accept", json!({ "mission": solo })),
        ErrorCode::Conflict
    );

    // A taker that completed a mission cannot take it again.
    let mut definition = bread_mission(400, 1);
    definition["takers"] = json!(2);
    let twice = post(&mut w, ann, definition);
    ok(&mut w, cid, "mission_accept", json!({ "mission": twice }));
    bake(&mut w, cid);
    w.step(2);
    assert_eq!(w.cash(cid), Money(400));
    assert_eq!(
        fail_message(&mut w, cid, "mission_accept", json!({ "mission": twice })),
        "You completed this mission already."
    );

    // The limit of missions that one resident works on.
    w.set_param("missions.accepted_max", 1);
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": twice })),
        "You work on too many missions. The limit is 1."
    );

    // A resident in jail cannot accept.
    w.act_ok(cid, "jail_me", Value::Null);
    assert_eq!(
        w.act_err(cid, "mission_accept", json!({ "mission": solo })).code,
        ErrorCode::Blocked
    );
    w.assert_conserved();
}

#[test]
fn a_mission_cannot_be_accepted_in_the_frame_of_its_deadline() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let mut definition = bread_mission(400, 1);
    definition["deadline_in"] = json!(10);
    let mission = post(&mut w, ann, definition);
    // The request reaches the global realm in frame 11, the deadline.
    w.step((10 - w.world.frame()) as u32);
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": mission })),
        "The mission ends in this frame."
    );
    assert_eq!(w.cash(ann), Money(10_000), "the reward came back");
    w.assert_conserved();
}

#[test]
fn mission_abandon_refusals_and_effect() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    // Two loaves, one place.
    let mission = post(&mut w, ann, bread_mission(400, 2));

    assert_eq!(fail(&mut w, bob, "mission_abandon", json!({ "mission": 99 })), ErrorCode::NotFound);
    assert_eq!(
        fail_message(&mut w, bob, "mission_abandon", json!({ "mission": mission })),
        "You do not work on this mission."
    );
    assert_eq!(
        fail(&mut w, bob, "mission_abandon", json!({ "mission": mission })),
        ErrorCode::Conflict
    );

    // Bob bakes one of two loaves and gives up. The place is free again.
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, bob);
    w.step(2);
    let ack = w.act(bob, "mission_abandon", json!({ "mission": mission }));
    let frames = w.step(2);
    let report = w.last_report(bob).expect("the answer came");
    assert_eq!((report.action_id, report.ok), (ack.action_id, true));
    assert_eq!(report.result, json!({ "mission": mission, "status": "abandoned" }));
    assert_eq!(
        one_event(&frames, "missions.mission_abandoned"),
        json!({ "mission": mission, "resident": bob })
    );
    assert_eq!(w.global().res::<Missions>().accepted_by(bob), [] as [u64; 0]);
    assert_eq!(
        fail(&mut w, bob, "mission_abandon", json!({ "mission": mission })),
        ErrorCode::Conflict
    );

    // Cid takes the place and leaves it again. Bob comes back, and starts
    // from zero: one more loaf is not enough.
    ok(&mut w, cid, "mission_accept", json!({ "mission": mission }));
    assert_eq!(
        fail_message(&mut w, bob, "mission_accept", json!({ "mission": mission })),
        "The mission has no free place."
    );
    ok(&mut w, cid, "mission_abandon", json!({ "mission": mission }));
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    bake(&mut w, bob);
    w.step(2);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(0), 400));
    let error = run_full(&mut w, bob, "mission_claim", json!({ "mission": mission }))
        .expect_err("one loaf is missing");
    assert_eq!(error.details, json!({ "progress": [[1, 2]] }));
    w.assert_conserved();
}

#[test]
fn mission_claim_refusals_and_effect() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    let mut definition = bread_mission(400, 1);
    definition["takers"] = json!(3);
    let mission = post(&mut w, ann, definition);
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(8_800), 1_200));

    assert_eq!(fail(&mut w, bob, "mission_claim", json!({ "mission": 99 })), ErrorCode::NotFound);
    // A resident with no part in the mission.
    assert_eq!(
        fail_message(&mut w, cid, "mission_claim", json!({ "mission": mission })),
        "You did not accept this mission and you did not publish it."
    );
    assert_eq!(
        fail(&mut w, cid, "mission_claim", json!({ "mission": mission })),
        ErrorCode::NotAllowed
    );

    // A taker whose objectives are not met.
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    let error = run_full(&mut w, bob, "mission_claim", json!({ "mission": mission }))
        .expect_err("no bread yet");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        error.message,
        "The objectives are not met yet. The engine pays by itself when they are."
    );
    assert_eq!(error.details, json!({ "progress": [[0, 1]] }));

    // The poster cannot take the reward back while a taker works on the
    // mission.
    let error = run_full(&mut w, ann, "mission_claim", json!({ "mission": mission }))
        .expect_err("Bob works on it");
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(
        error.message,
        "1 takers work on the mission. The unpaid reward comes back at the deadline."
    );
    assert_eq!(error.details, json!({ "active": 1, "deadline": 601 }));
    assert_eq!(escrow(&w, mission), 1_200);

    // Bob completes it. The engine paid already, so his claim moves nothing
    // and tells him what he got.
    bake(&mut w, bob);
    w.step(2);
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(400), 800));
    let paid = ok(&mut w, bob, "mission_claim", json!({ "mission": mission }));
    assert_eq!(paid["status"], json!("paid"));
    assert_eq!(paid["paid"], json!(400));
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(400), 800));

    // Nobody works on the mission now: the poster takes back the reward of
    // the two places that are left, and the mission closes.
    let ack = w.act(ann, "mission_claim", json!({ "mission": mission }));
    let frames = w.step(2);
    let report = w.last_report(ann).expect("the answer came");
    assert_eq!(report.action_id, ack.action_id);
    assert_eq!(
        report.result,
        json!({ "mission": mission, "status": "withdrawn", "refunded": 800 })
    );
    assert_eq!(
        one_event(&frames, "missions.mission_closed"),
        json!({ "mission": mission, "reason": "withdrawn", "completed": 1, "refunded": 800, "destroyed": 0 })
    );
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(9_600), 0));
    assert_eq!(
        fail(&mut w, ann, "mission_claim", json!({ "mission": mission })),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, bob, "mission_claim", json!({ "mission": mission })),
        ErrorCode::NotFound
    );
    let totals = w.assert_conserved();
    assert_eq!(totals.money.held, 10_000);
}

#[test]
fn mission_list_and_leaderboard_refusals() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    assert_eq!(fail(&mut w, ann, "mission_list", json!({ "mission": 99 })), ErrorCode::NotFound);
    for args in [
        json!({ "scope": "everything" }),
        json!({ "mode": "duo" }),
        json!({ "limit": -1 }),
        json!({ "filter": "open" }),
    ] {
        assert_eq!(w.act_err(ann, "mission_list", args).code, ErrorCode::InvalidArgs);
    }
    for args in [
        json!({}),
        json!({ "board": "fame" }),
        json!({ "board": "wealth", "by": "city" }),
        json!({ "board": "wealth", "top": 5 }),
    ] {
        assert_eq!(w.act_err(ann, "leaderboard", args).code, ErrorCode::InvalidArgs);
    }
    assert_eq!(
        fail_message(&mut w, ann, "leaderboard", json!({ "board": "wealth", "season": 9 })),
        "The hall of fame has no season 9."
    );
    assert_eq!(
        fail(&mut w, ann, "leaderboard", json!({ "board": "wealth", "season": 9 })),
        ErrorCode::NotFound
    );

    // The two verbs that only read work in jail. They are free: a resident
    // that waits can use them.
    w.act_ok(ann, "jail_me", Value::Null);
    assert_eq!(ok(&mut w, ann, "mission_list", json!({})), json!({ "missions": [] }));
    assert_eq!(
        ok(&mut w, ann, "leaderboard", json!({ "board": "wealth" }))["board"],
        json!("wealth")
    );
    let catalogue = w.world.catalogue();
    let free: Vec<(&str, bool)> = catalogue
        .verbs
        .iter()
        .filter(|verb| verb.module == "missions")
        .map(|verb| (verb.verb.as_str(), verb.free))
        .collect();
    assert_eq!(
        free,
        [
            ("leaderboard", true),
            ("mission_abandon", false),
            ("mission_accept", false),
            ("mission_claim", false),
            ("mission_list", true),
            ("mission_post", false),
        ]
    );
}

#[test]
fn a_request_that_is_cancelled_on_its_way_is_still_handled() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 1_000);
    // The request leaves with the cash when the action starts. Ann cancels
    // the action before the answer comes.
    let ack = w.act(ann, "mission_post", bread_mission(400, 1));
    w.act_ok(ann, "cancel", Value::Null);
    w.step(2);
    let report = w.last_report(ann).expect("the action ended");
    assert_eq!(report.action_id, ack.action_id);
    assert_eq!(report.error.map(|error| error.code), Some(ErrorCode::Interrupted));
    // The global realm published the mission all the same, and Ann learns
    // that she must not send the request again.
    assert_eq!(
        common::notice(&w, ann, "missions.request_still_handled").data,
        json!({ "verb": "mission_post", "action_id": ack.action_id, "reason": "cancelled" })
    );
    let posted = ok(&mut w, ann, "mission_list", json!({ "scope": "posted" }));
    let mission = posted["missions"][0]["mission"].as_u64().expect("the mission exists");
    assert_eq!((w.cash(ann), escrow(&w, mission)), (Money(600), 400));
    // A verb that does not wait for the global realm gives no such notice.
    w.act_ok(ann, "wait", json!({ "ticks": 5 }));
    w.act_ok(ann, "cancel", Value::Null);
    assert_eq!(common::count_notices(&w, ann, "missions.request_still_handled"), 1);
    w.assert_conserved();
}
