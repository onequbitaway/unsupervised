//! Elections: the calendar, the candidates, the campaign, the vote, the
//! count and the offices of the winners.

mod common;

use common::{
    CITY, FEE, at, can, election_of, event, events, fail, holders, notices, ok, register, rich,
    rich_at, run_for, section, step_to, treasury, vote, world,
};
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, OrgId, Pos};
use unsup_politics::roles;

/// The frame at which the polls of the first city election open.
const POLLS_OPEN: u64 = 2 * 1_440 + 360;
/// The frame at which they close.
const POLLS_CLOSE: u64 = 2 * 1_440 + 1_200;

#[test]
fn a_full_election_from_registration_to_the_office() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 20_000);
    let near = rich_at(&mut w, "Near", 3_000, Pos::new(28, 20));
    let far = rich_at(&mut w, "Far", 0, Pos::new(60, 60));

    // Frame 1: the registration of the first city election opens.
    let frames = w.step(1);
    let called = events(&frames, "politics.election_called");
    assert_eq!(called.len(), 2, "a mayor and a council");
    assert_eq!(called[0].data["office"], "mayor");
    assert_eq!(called[0].data["city"], 0);
    assert_eq!(called[0].data["kind"], "regular");
    assert_eq!(called[0].data["seats"], 1);
    assert_eq!(called[0].data["vote_day"], json!({ "year": 1, "day": 3 }));
    assert_eq!(called[1].data["office"], "council");
    assert_eq!(called[1].data["seats"], 5);
    let mayor = election_of(&w, ann, "mayor");
    assert_eq!(called[0].data["election"], mayor);

    // Registration: the fee goes to the city treasury.
    let result = register(&mut w, ann, mayor, "Lower taxes.");
    assert_eq!(result["fee"], FEE);
    assert_eq!(result["office"], "mayor");
    assert_eq!(result["candidates"], 1);
    assert_eq!(w.cash(ann).cents(), 20_000 - FEE);
    assert_eq!(treasury(&w, CITY), FEE);
    let ack = w.act_ok(
        bob,
        "register_candidate",
        json!({ "election": mayor, "platform": "More parks." }),
    );
    assert_eq!(ack, unsup_kernel::protocol::ActionStatus::Pending);
    let frames = w.step(2);
    let registered = event(&frames, "politics.candidate_registered");
    assert_eq!(registered.actor, Some(bob));
    assert_eq!(registered.data["candidate"], json!(bob));
    assert_eq!(registered.data["candidates"], 2);
    assert_eq!(
        registered.data["platform"],
        json!({ "untrusted": true, "from": bob, "text": "More parks." })
    );
    assert_eq!(treasury(&w, CITY), 2 * FEE);

    // A donation goes to the campaign fund, not to the candidate.
    let gift =
        ok(&mut w, near, "donate", json!({ "election": mayor, "candidate": ann, "amount": 1_000 }));
    assert_eq!(
        gift,
        json!({ "election": mayor, "candidate": ann, "amount": 1_000, "fund": 1_000 })
    );
    assert_eq!(w.cash(near).cents(), 2_000);
    assert_eq!(w.cash(ann).cents(), 20_000 - FEE);
    assert_eq!(
        section(&w, ann)["candidacies"],
        json!([
            { "city": 0, "election": mayor, "office": "mayor", "phase": "registration", "fund": 1_000 }
        ])
    );

    // Day 2: the campaign. A rally takes 30 game minutes and money of the
    // fund. 600 cents reach 6 + 6 tiles: `near` is 8 tiles away.
    step_to(&mut w, at(2, 0));
    assert_eq!(section(&w, ann)["elections"][0]["phase"], "campaign");
    let ack = w.act_ok(ann, "campaign", json!({ "election": mayor, "spend": 600 }));
    assert_eq!(ack, unsup_kernel::protocol::ActionStatus::Pending);
    let frames = w.step(29);
    assert!(events(&frames, "politics.rally_held").is_empty(), "the rally takes time");
    assert_eq!(w.activity(ann), Some("campaign"));
    let frames = w.step(3);
    let rally = event(&frames, "politics.rally_held");
    assert_eq!(rally.actor, Some(ann));
    assert_eq!(rally.pos, Some(Pos::new(20, 20)));
    assert_eq!(rally.data["heard"], 2, "Bob and Near");
    assert_eq!(rally.data["radius"], 12);
    assert_eq!(rally.data["spend"], 600);
    let report = w.last_report(ann).expect("the rally reports");
    assert_eq!(report.result, json!({ "election": mayor, "spend": 600, "radius": 12, "heard": 2 }));
    let heard = notices(&w, near, "politics.rally_held");
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].from, Some(ann));
    assert_eq!(
        heard[0].data["platform"],
        json!({ "untrusted": true, "from": ann, "text": "Lower taxes." })
    );
    assert!(notices(&w, far, "politics.rally_held").is_empty(), "too far away");
    // The cost went from the fund to the city of the rally.
    assert_eq!(treasury(&w, CITY), 2 * FEE + 600);
    assert_eq!(section(&w, ann)["candidacies"][0]["fund"], 400);

    // Day 3, 06:00: the polls open.
    let frames = step_to(&mut w, POLLS_OPEN);
    let opened = events(&frames, "politics.polls_opened");
    assert_eq!(opened.len(), 2);
    assert_eq!(opened[0].data["election"], mayor);
    assert_eq!(opened[0].data["candidates"], 2);
    let cast = vote(&mut w, near, mayor, ann);
    assert_eq!(
        cast,
        json!({ "election": mayor, "office": "mayor", "candidate": ann, "turnout": 1 })
    );
    vote(&mut w, far, mayor, ann);
    vote(&mut w, bob, mayor, bob);
    assert_eq!(section(&w, near)["elections"][0]["voted"], true);
    assert!(!can(&w, ann, CITY, "gov.sign"), "nobody holds the office before the count");

    // Day 3, 20:00: the count. The winner takes office at once.
    let frames = step_to(&mut w, POLLS_CLOSE);
    let decided = events(&frames, "politics.election_decided");
    assert_eq!(decided.len(), 2);
    let result = &decided[0].data;
    assert_eq!(decided[0].actor, Some(ann));
    assert!(decided[0].drama >= 71, "an election result is a headline");
    assert_eq!(result["election"], mayor);
    assert_eq!(result["office"], "mayor");
    assert_eq!(result["city"], 0);
    assert_eq!(result["winner"], json!(ann));
    assert_eq!(result["winners"], json!([ann]));
    assert_eq!(result["previous"], json!([]));
    assert_eq!(
        result["tally"],
        json!([
            { "candidate": ann, "votes": 2, "won": true },
            { "candidate": bob, "votes": 1, "won": false },
        ])
    );
    assert_eq!(result["turnout"], 3);
    assert_eq!(result["tie"], false);
    assert_eq!(result["margin"], 1);
    assert_eq!(result["funds_to_treasury"], 400);
    assert_eq!(holders(&w, CITY, roles::MAYOR), [ann]);
    // The zone of the winner has the office one frame later.
    w.step(1);
    for perm in ["gov.sign", "gov.appoint", "treasury.spend"] {
        assert!(can(&w, ann, CITY, perm), "{perm}");
        assert!(!can(&w, bob, CITY, perm), "{perm}");
    }
    assert!(!can(&w, ann, CITY, "gov.vote"), "the mayor is not a member of the council");
    assert!(!can(&w, ann, OrgId::NATION, "gov.sign"), "a city office gives nothing in the nation");
    assert_eq!(notices(&w, ann, "politics.election_won").len(), 1);
    assert_eq!(notices(&w, bob, "politics.election_lost").len(), 1);
    assert_eq!(section(&w, ann)["offices"], json!([{ "city": 0, "office": "mayor" }]));
    // The rest of the fund went to the treasury, not to the candidate.
    assert_eq!(treasury(&w, CITY), 2 * FEE + 1_000);
    assert_eq!(w.cash(ann).cents(), 20_000 - FEE);
    // The next term is on the calendar.
    assert_eq!(section(&w, ann)["next_vote"]["city"], json!({ "year": 1, "day": 17 }));
    w.assert_conserved();
}

#[test]
fn each_refusal_of_the_election_verbs_has_its_code() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let poor = rich(&mut w, "Poor", FEE - 1);
    let cat = rich(&mut w, "Cat", 20_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    let council = election_of(&w, ann, "council");
    let platform = json!("A platform.");

    // register_candidate
    assert_eq!(
        fail(&mut w, ann, "register_candidate", json!({ "election": mayor, "platform": " " })),
        ErrorCode::InvalidArgs
    );
    let long = "x".repeat(281);
    assert_eq!(
        fail(&mut w, ann, "register_candidate", json!({ "election": mayor, "platform": long })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        fail(
            &mut w,
            poor,
            "register_candidate",
            json!({ "election": mayor, "platform": platform })
        ),
        ErrorCode::Insufficient
    );
    assert_eq!(
        fail(&mut w, ann, "register_candidate", json!({ "election": 77, "platform": platform })),
        ErrorCode::NotFound
    );
    register(&mut w, ann, mayor, "A platform.");
    assert_eq!(
        fail(&mut w, ann, "register_candidate", json!({ "election": mayor, "platform": platform })),
        ErrorCode::Conflict
    );
    // One office of a government at a time.
    assert_eq!(
        fail(
            &mut w,
            ann,
            "register_candidate",
            json!({ "election": council, "platform": platform })
        ),
        ErrorCode::Conflict
    );
    // A refused registration keeps no money.
    assert_eq!(w.cash(ann).cents(), 20_000 - FEE);
    assert_eq!(treasury(&w, CITY), FEE);

    // donate
    let gift = |amount: i64, to| json!({ "election": mayor, "candidate": to, "amount": amount });
    assert_eq!(fail(&mut w, cat, "donate", gift(0, ann)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, cat, "donate", gift(20_001, ann)), ErrorCode::Insufficient);
    assert_eq!(fail(&mut w, cat, "donate", gift(100, cat)), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, cat, "donate", json!({ "election": 77, "candidate": ann, "amount": 100 })),
        ErrorCode::NotFound
    );
    assert_eq!(w.cash(cat).cents(), 20_000, "a refused donation comes back");
    ok(&mut w, cat, "donate", gift(500, ann));

    // campaign and vote come too early in the registration phase.
    let rally = |spend: i64| json!({ "election": mayor, "spend": spend });
    assert_eq!(fail(&mut w, ann, "campaign", rally(200)), ErrorCode::Conflict);
    assert_eq!(
        fail(&mut w, cat, "vote", json!({ "election": mayor, "candidate": ann })),
        ErrorCode::NotAllowed,
        "a resident that lived less than a day cannot vote"
    );

    // The campaign phase.
    step_to(&mut w, at(2, 0));
    assert_eq!(
        fail(&mut w, cat, "register_candidate", json!({ "election": mayor, "platform": platform })),
        ErrorCode::Conflict
    );
    assert_eq!(fail(&mut w, ann, "campaign", rally(99)), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, ann, "campaign", rally(501)), ErrorCode::Insufficient);
    assert_eq!(fail(&mut w, cat, "campaign", rally(200)), ErrorCode::NotAllowed);
    assert_eq!(
        fail(&mut w, ann, "campaign", json!({ "election": 77, "spend": 200 })),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, cat, "vote", json!({ "election": mayor, "candidate": ann })),
        ErrorCode::Conflict,
        "the polls are not open"
    );
    // A refused rally spends nothing.
    assert_eq!(section(&w, ann)["candidacies"][0]["fund"], 500);

    // The voting day.
    step_to(&mut w, POLLS_OPEN);
    assert_eq!(fail(&mut w, cat, "donate", gift(100, ann)), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, ann, "campaign", rally(200)), ErrorCode::Conflict);
    assert_eq!(
        fail(&mut w, cat, "vote", json!({ "election": mayor, "candidate": cat })),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, cat, "vote", json!({ "election": 77, "candidate": ann })),
        ErrorCode::NotFound
    );
    vote(&mut w, cat, mayor, ann);
    // One vote for each resident and office.
    assert_eq!(
        fail(&mut w, cat, "vote", json!({ "election": mayor, "candidate": ann })),
        ErrorCode::Conflict
    );
    let frames = step_to(&mut w, POLLS_CLOSE);
    assert_eq!(events(&frames, "politics.election_decided")[0].data["tally"][0]["votes"], 1);
    // After the count the election is gone.
    assert_eq!(
        fail(&mut w, poor, "vote", json!({ "election": mayor, "candidate": ann })),
        ErrorCode::NotFound
    );
    w.assert_conserved();
}

#[test]
fn a_candidate_that_stops_a_rally_gets_no_event_and_no_money_back() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let near = rich(&mut w, "Near", 0);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "A platform.");
    ok(&mut w, ann, "donate", json!({ "election": mayor, "candidate": ann, "amount": 1_000 }));
    step_to(&mut w, at(2, 0));
    w.act_ok(ann, "campaign", json!({ "election": mayor, "spend": 300 }));
    w.step(5);
    w.act_ok(ann, "cancel", json!({}));
    let frames = w.step(40);
    assert!(events(&frames, "politics.rally_held").is_empty());
    assert!(notices(&w, near, "politics.rally_held").is_empty());
    assert_eq!(section(&w, ann)["candidacies"][0]["fund"], 700);
    assert_eq!(treasury(&w, CITY), FEE + 300);
    // The next rally works.
    let result = run_for(&mut w, ann, "campaign", json!({ "election": mayor, "spend": 100 }), 33);
    assert_eq!(result.map(|r| r["heard"].clone()), Ok(json!(1)));
    assert_eq!(w.money(Holder::Org(CITY)).cents(), FEE + 400);
    w.assert_conserved();
}

#[test]
fn a_request_that_left_is_still_handled_after_the_action_stops() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    // The request leaves for the global realm when the action starts. The
    // resident stops the action in the same frame.
    let ack = w.act(ann, "register_candidate", json!({ "election": mayor, "platform": "Ann." }));
    w.act_ok(ann, "cancel", json!({}));
    w.step(2);
    let report = w
        .reports(ann)
        .into_iter()
        .find(|report| report.action_id == ack.action_id)
        .expect("the action reports");
    assert_eq!(report.error.map(|error| error.code), Some(ErrorCode::Interrupted));
    // The resident learns that the request goes on, and the observation
    // has the outcome: it is a candidate and the fee is paid.
    let told = notices(&w, ann, "politics.request_still_handled");
    assert_eq!(told.len(), 1);
    assert_eq!(
        told[0].data,
        json!({ "verb": "register_candidate", "action_id": ack.action_id, "reason": "cancelled" })
    );
    assert_eq!(section(&w, ann)["candidacies"][0]["election"], mayor);
    assert_eq!(w.cash(ann).cents(), 20_000 - FEE);
    assert_eq!(treasury(&w, CITY), FEE);
    // A verb of another module that stops gives no such notice.
    w.act_ok(ann, "wait", json!({ "ticks": 10 }));
    w.act_ok(ann, "cancel", json!({}));
    w.step(1);
    assert_eq!(notices(&w, ann, "politics.request_still_handled").len(), 1);
    w.assert_conserved();
}

#[test]
fn an_election_takes_a_limited_number_of_candidates() {
    let data = common::overrides(&[("politics.candidates_max", 1)]);
    let config = unsup_kernel::WorldConfig { data: vec![data], ..common::default_config(0) };
    let mut w = unsup_kernel::testing::TestWorld::with_config(&common::plugins(), config);
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 20_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Ann.");
    assert_eq!(
        fail(&mut w, bob, "register_candidate", json!({ "election": mayor, "platform": "Bob." })),
        ErrorCode::Conflict
    );
    // The refused candidate has its fee back, and the other election of
    // the city still takes it.
    assert_eq!(w.cash(bob).cents(), 20_000);
    assert_eq!(treasury(&w, CITY), FEE);
    let council = election_of(&w, bob, "council");
    register(&mut w, bob, council, "Bob.");
    assert_eq!(treasury(&w, CITY), 2 * FEE);
    w.assert_conserved();
}
