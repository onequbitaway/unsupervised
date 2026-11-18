//! Recall: a petition, its signatures and the early election.

mod common;

use common::{
    CITY, DAY, SQUARE_B, at, can, election_of, error_of, event, events, fail, holders, notices, ok,
    plugins, quiet_config, register, rich, rich_at, seat, section, step_to, vote, with_two_cities,
    world,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, OrgId, ResidentId};
use unsup_politics::roles;

/// Makes a world with a mayor and four more residents in the first city.
fn city() -> (TestWorld<'static>, ResidentId, Vec<ResidentId>) {
    let mut w = TestWorld::with_config(&plugins(), quiet_config());
    let mayor = rich(&mut w, "Mayor", 0);
    let r: Vec<ResidentId> =
        (0..4).map(|i| rich(&mut w, &format!("Resident {i}"), 10_000)).collect();
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    (w, mayor, r)
}

#[test]
fn enough_signatures_trigger_an_early_election() {
    let (mut w, mayor, r) = city();
    let regular = section(&w, r[0])["next_vote"]["city"].clone();
    assert_eq!(regular, json!({ "year": 11, "day": 20 }), "day 300 of the world");

    // Five residents are in the city. A tenth of them is less than the
    // least number of signatures, so the petition needs three.
    let frame = w.world.frame();
    w.act_ok(r[0], "start_recall", json!({ "office": "mayor" }));
    let frames = w.step(2);
    let started = w.last_report(r[0]).expect("the petition reports").result;
    let recall = started["recall"].as_u64().expect("the id of the petition");
    assert_eq!(
        started,
        json!({
            "gov": "city", "city": 0, "recall": recall, "office": "mayor", "signatures": 1,
            "needed": 3, "deadline": frame + 1 + 2 * DAY, "succeeded": false, "election": null,
        })
    );
    let event_started = event(&frames, "politics.recall_started");
    assert_eq!(event_started.actor, Some(r[0]));
    assert_eq!(event_started.data["starter"], json!(r[0]));
    assert_eq!(event_started.data["holders"], json!([mayor]));
    assert_eq!(
        (&event_started.data["signatures"], &event_started.data["needed"]),
        (&json!(1), &json!(3))
    );
    assert_eq!(notices(&w, mayor, "politics.recall_started").len(), 1);
    let open = &section(&w, r[1])["recalls"];
    assert_eq!(open.as_array().map(Vec::len), Some(1));
    assert_eq!((&open[0]["recall"], &open[0]["office"]), (&json!(recall), &json!("mayor")));
    assert_eq!((&open[0]["signatures"], &open[0]["needed"]), (&json!(1), &json!(3)));
    assert_eq!((&open[0]["signed"], &open[0]["city"]), (&json!(false), &json!(0)));
    assert_eq!(section(&w, r[0])["recalls"][0]["signed"], true);

    // The second signature is not enough.
    let second = ok(&mut w, r[1], "sign_recall", json!({ "recall": recall }));
    assert_eq!(second["signatures"], 2);
    assert_eq!((&second["succeeded"], &second["election"]), (&json!(false), &Value::Null));

    // The third signature opens the early election at once.
    w.act_ok(r[2], "sign_recall", json!({ "recall": recall }));
    let frames = w.step(2);
    let third = w.last_report(r[2]).expect("the signature reports").result;
    assert_eq!((&third["signatures"], &third["succeeded"]), (&json!(3), &json!(true)));
    let early = third["election"].as_u64().expect("the id of the early election");
    let succeeded = event(&frames, "politics.recall_succeeded");
    assert_eq!(succeeded.drama, 70);
    assert_eq!(
        (&succeeded.data["recall"], &succeeded.data["election"]),
        (&json!(recall), &json!(early))
    );
    // Today is day 1. One day of registration and one day of campaign
    // follow the rest of today: the voting day is day 4.
    assert_eq!(succeeded.data["vote_day"], json!({ "year": 1, "day": 4 }));
    let called = event(&frames, "politics.election_called");
    assert_eq!((&called.data["election"], &called.data["kind"]), (&json!(early), &json!("recall")));
    assert_eq!((&called.data["office"], &called.data["city"]), (&json!("mayor"), &json!(0)));
    assert_eq!(notices(&w, mayor, "politics.recall_succeeded").len(), 1);
    assert_eq!(section(&w, r[3])["recalls"], Value::Null, "the petition is closed");
    assert_eq!(election_of(&w, r[3], "mayor"), early);
    // Only the office of the petition has an election.
    let offices: Vec<Value> = section(&w, r[3])["elections"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["office"].clone())
        .collect();
    assert_eq!(offices, ["mayor"]);

    // The mayor stays in office until the count.
    assert!(can(&w, mayor, CITY, "gov.sign"));
    register(&mut w, r[0], early, "A new start.");
    step_to(&mut w, at(4, 360));
    assert!(can(&w, mayor, CITY, "gov.sign"));
    vote(&mut w, r[1], early, r[0]);
    let frames = step_to(&mut w, at(4, 1_200) + 1);
    let decided = event(&frames, "politics.election_decided");
    assert_eq!(decided.data["kind"], "recall");
    assert_eq!(decided.data["winners"], json!([r[0]]));
    assert_eq!(decided.data["previous"], json!([mayor]));
    assert!(can(&w, r[0], CITY, "gov.sign"));
    assert!(!can(&w, mayor, CITY, "gov.sign"));
    assert_eq!(holders(&w, CITY, roles::MAYOR), [r[0]]);
    // The regular calendar did not move.
    assert_eq!(section(&w, r[0])["next_vote"]["city"], regular);
    w.assert_conserved();
}

#[test]
fn a_petition_without_enough_signatures_ends() {
    let (mut w, mayor, r) = city();
    let started = ok(&mut w, r[0], "start_recall", json!({ "office": "mayor" }));
    let recall = started["recall"].as_u64().expect("the id");
    let deadline = started["deadline"].as_u64().expect("a frame");
    ok(&mut w, r[1], "sign_recall", json!({ "recall": recall }));
    let frames = step_to(&mut w, deadline - 1);
    assert!(events(&frames, "politics.recall_failed").is_empty(), "the petition is still open");
    let frames = w.step(1);
    let failed = event(&frames, "politics.recall_failed");
    assert_eq!((&failed.data["recall"], &failed.data["signatures"]), (&json!(recall), &json!(2)));
    assert_eq!((&failed.data["needed"], &failed.data["starter"]), (&json!(3), &json!(r[0])));
    assert!(events(&frames, "politics.election_called").is_empty());
    assert_eq!(fail(&mut w, r[2], "sign_recall", json!({ "recall": recall })), ErrorCode::NotFound);
    assert!(can(&w, mayor, CITY, "gov.sign"));
    assert_eq!(section(&w, r[2])["recalls"], Value::Null);
    // A new petition can start.
    let again = ok(&mut w, r[2], "start_recall", json!({ "office": "mayor" }));
    assert_ne!(again["recall"], json!(recall));
    w.assert_conserved();
}

#[test]
fn each_refusal_of_the_recall_verbs_has_its_code() {
    let mut w = TestWorld::with_config(&plugins(), with_two_cities(quiet_config()));
    let ann = rich(&mut w, "Ann", 0);
    let bob = rich(&mut w, "Bob", 0);
    let far = rich_at(&mut w, "Far", 0, SQUARE_B);
    w.step(1);
    let unknown = w.act_err(ann, "start_recall", json!({ "office": "king" }));
    assert_eq!(unknown.code, ErrorCode::InvalidArgs);
    let started = ok(&mut w, ann, "start_recall", json!({ "office": "council" }));
    let recall = started["recall"].as_u64().expect("the id");
    // One open petition for an office.
    let error = error_of(&mut w, bob, "start_recall", json!({ "office": "council" }));
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.details, json!({ "recall": recall }));
    // The same office of another city is another office.
    let other = ok(&mut w, far, "start_recall", json!({ "office": "council" }));
    assert_eq!(other["city"], 1);
    // One resident lives in the second city, so the least number counts.
    assert_eq!(other["needed"], 3);

    // sign_recall
    assert_eq!(fail(&mut w, bob, "sign_recall", json!({ "recall": 77 })), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, far, "sign_recall", json!({ "recall": recall })),
        ErrorCode::NotHere,
        "only residents of the city sign"
    );
    assert_eq!(fail(&mut w, ann, "sign_recall", json!({ "recall": recall })), ErrorCode::Conflict);
    ok(&mut w, bob, "sign_recall", json!({ "recall": recall }));
    assert_eq!(fail(&mut w, bob, "sign_recall", json!({ "recall": recall })), ErrorCode::Conflict);

    // A national office: residents of every city sign.
    let national = ok(&mut w, ann, "start_recall", json!({ "office": "president" }));
    assert_eq!(national["gov"], "nation");
    let national = national["recall"].as_u64().expect("the id");
    ok(&mut w, far, "sign_recall", json!({ "recall": national }));
    let third = ok(&mut w, bob, "sign_recall", json!({ "recall": national }));
    assert_eq!(third["succeeded"], true);
    // While the early election is open, no new petition starts.
    assert_eq!(
        fail(&mut w, far, "start_recall", json!({ "office": "president" })),
        ErrorCode::Conflict
    );
    assert!(holders(&w, OrgId::NATION, roles::PRESIDENT).is_empty());
    w.assert_conserved();
}

#[test]
fn a_recall_needs_a_resident_of_a_day_and_time_before_the_regular_election() {
    // The calendar of the data file: residents vote and sign after one
    // day, and the first city election is on day 3.
    let mut w = world();
    let ann = rich(&mut w, "Ann", 0);
    w.step(1);
    let young = w.act_err(ann, "start_recall", json!({ "office": "mayor" }));
    assert_eq!(young.code, ErrorCode::NotAllowed);
    assert_eq!(young.details, json!({ "needed": 1_440, "lived": 1 }));
    step_to(&mut w, at(2, 0));
    // The regular election for the office is open.
    assert_eq!(
        fail(&mut w, ann, "start_recall", json!({ "office": "mayor" })),
        ErrorCode::Conflict
    );

    // After the first election the next voting day is day 17. A petition
    // has two days, and its early election needs three more: the last day
    // to start one is day 11.
    step_to(&mut w, at(11, 0));
    ok(&mut w, ann, "start_recall", json!({ "office": "mayor" }));
    step_to(&mut w, at(12, 0));
    let near = error_of(&mut w, ann, "start_recall", json!({ "office": "council" }));
    assert_eq!(near.code, ErrorCode::Conflict);
    assert_eq!(near.details, json!({ "regular_vote_day": { "year": 1, "day": 17 } }));
    // A resident that just arrived cannot sign or start.
    let new = rich(&mut w, "New", 0);
    let refused = w.act_err(new, "sign_recall", json!({ "recall": 1 }));
    assert_eq!(refused.code, ErrorCode::NotAllowed);
    w.assert_conserved();
}
