//! Public state for viewers: the `_published` events.

mod common;

use common::{CITY, FEE, at, election_of, events, published, register, rich, step_to, world};
use serde_json::{Value, json};
use unsup_kernel::protocol::FrameOutput;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Event, ResidentId};

/// Returns the publications of one election in a list of frames.
fn of_election(frames: &[FrameOutput], id: u64) -> Vec<&Event> {
    events(frames, "politics.election_published")
        .into_iter()
        .filter(|e| e.data["key"] == json!(id))
        .collect()
}

/// Casts a vote and returns the frames of the next six game minutes, in
/// which the count is published.
fn vote_and_watch(
    w: &mut TestWorld<'_>,
    voter: ResidentId,
    election: u64,
    candidate: ResidentId,
) -> Vec<FrameOutput> {
    w.act_ok(voter, "vote", json!({ "election": election, "candidate": candidate }));
    w.step(6)
}

#[test]
fn the_first_frame_publishes_every_government() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 0);
    let frames = w.step(1);
    let governments = events(&frames, "politics.government_published");
    assert_eq!(governments.len(), 2, "the nation and one city");
    let nation =
        published(&frames, "politics.government_published", &json!("nation")).expect("the nation");
    assert_eq!(nation["gov"], "nation");
    assert_eq!(nation["org"], 1);
    assert_eq!(nation["next_vote"], json!({ "year": 1, "day": 5 }));
    assert_eq!(nation["elections"], json!([]));
    let offices: Vec<(&Value, &Value, &Value)> = nation["offices"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|o| (&o["office"], &o["seats"], &o["salary"]))
        .collect();
    assert_eq!(
        offices,
        [
            (&json!("president"), &json!(1), &json!(12_000)),
            (&json!("legislature"), &json!(7), &json!(4_000)),
            (&json!("central_banker"), &json!(1), &json!(8_000)),
        ]
    );
    assert_eq!(nation["offices"][2]["org"], 2, "the post is a role of the central bank");
    assert_eq!(nation["offices"][2]["elected"], false);

    let city = published(&frames, "politics.government_published", &json!(0)).expect("the city");
    assert_eq!(
        (&city["gov"], &city["city"], &city["org"]),
        (&json!("city"), &json!(0), &json!(100))
    );
    assert_eq!(city["next_vote"], json!({ "year": 1, "day": 3 }));
    assert_eq!(city["treasury"], 0);
    let names: Vec<&Value> =
        city["offices"].as_array().expect("a list").iter().map(|o| &o["office"]).collect();
    assert_eq!(names, ["mayor", "council", "police_chief", "police_officer", "judge"]);
    assert_eq!(city["offices"][0]["holders"], json!([]));
    assert_eq!(city["offices"][0]["role"], "mayor");
    assert_eq!(city["offices"][1]["role"], "councillor");
    assert_eq!(city["offices"][1]["seats"], 5);
    // The elections of the first frame are in the state of the city.
    let mayor = election_of(&w, ann, "mayor");
    assert_eq!(city["elections"][0]["election"], mayor);
    assert_eq!(city["elections"][0]["phase"], "registration");
    assert_eq!(city["recalls"], json!([]));
    // The registers of bills start empty.
    let bills = published(&frames, "politics.bills_published", &json!(0)).expect("the register");
    assert_eq!(
        (&bills["open"], &bills["closed"], &bills["laws"]),
        (&json!([]), &json!([]), &json!([]))
    );
    assert!(published(&frames, "politics.bills_published", &json!("nation")).is_some());
    // Each open election has its own series.
    let series = of_election(&frames, mayor);
    assert_eq!(series.len(), 1);
    assert_eq!(series[0].data["phase"], "registration");
    assert_eq!(series[0].data["candidates"], json!([]));
    assert_eq!(series[0].data["vote_day"], json!({ "year": 1, "day": 3 }));
    assert_eq!(series[0].data["polls_open"], at(3, 360));
    assert_eq!(series[0].data["polls_close"], at(3, 1_200));
    // Published state is not news.
    assert!(governments.iter().all(|e| e.drama <= 10));
    w.assert_conserved();
}

#[test]
fn an_election_is_published_with_live_counts_and_the_result() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 20_000);
    let cat = rich(&mut w, "Cat", 0);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Lower taxes.");
    register(&mut w, bob, mayor, "More parks.");
    // A series is published at most one time in five game minutes.
    let frames = w.step(10);
    let series = of_election(&frames, mayor);
    assert_eq!(series.len(), 1, "two registrations, one publication");
    let candidates = series[0].data["candidates"].as_array().expect("a list");
    assert_eq!(candidates.len(), 2);
    assert_eq!(
        candidates[0],
        json!({
            "candidate": ann, "name": "Ann", "raised": 0, "spent": 0, "fund": 0, "votes": 0,
            "platform": { "untrusted": true, "from": ann, "text": "Lower taxes." },
        })
    );
    // Nothing changes: nothing is published.
    let frames = w.step(100);
    assert!(of_election(&frames, mayor).is_empty());

    // The phases.
    let frames = step_to(&mut w, at(2, 6));
    let series = of_election(&frames, mayor);
    assert_eq!(series.last().expect("published").data["phase"], "campaign");
    let frames = step_to(&mut w, at(3, 366));
    let series = of_election(&frames, mayor);
    assert_eq!(series.last().expect("published").data["phase"], "voting");

    // The voting day: the counts are live, and a new leader is news.
    let frames = vote_and_watch(&mut w, bob, mayor, bob);
    let live = of_election(&frames, mayor);
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].data["turnout"], 1);
    assert_eq!(live[0].data["candidates"][0]["candidate"], json!(bob), "the leader is first");
    assert_eq!(live[0].data["candidates"][0]["votes"], 1);
    assert_eq!(live[0].data["candidates"][1]["votes"], 0);
    let lead = events(&frames, "politics.lead_changed");
    assert_eq!(lead.len(), 1);
    assert_eq!(lead[0].actor, Some(bob));
    assert_eq!((&lead[0].data["leader"], &lead[0].data["votes"]), (&json!(bob), &json!(1)));
    assert_eq!((&lead[0].data["previous"], &lead[0].data["second"]), (&Value::Null, &json!(0)));
    assert!((41..=70).contains(&lead[0].drama), "a lead change is dramatic");

    // A tie has no leader. Then the other candidate leads.
    let frames = vote_and_watch(&mut w, ann, mayor, ann);
    assert!(events(&frames, "politics.lead_changed").is_empty(), "one vote each: nobody leads");
    assert_eq!(of_election(&frames, mayor)[0].data["turnout"], 2);
    let frames = vote_and_watch(&mut w, cat, mayor, ann);
    let lead = events(&frames, "politics.lead_changed");
    assert_eq!(lead.len(), 1);
    assert_eq!((&lead[0].data["leader"], &lead[0].data["previous"]), (&json!(ann), &json!(bob)));
    assert_eq!((&lead[0].data["votes"], &lead[0].data["second"]), (&json!(2), &json!(1)));

    // The count publishes the result in the same frame.
    let frames = step_to(&mut w, at(3, 1_200));
    let decided = std::slice::from_ref(frames.last().expect("a frame"));
    let last = of_election(decided, mayor);
    assert_eq!(last.len(), 1);
    assert_eq!(last[0].data["phase"], "decided");
    assert_eq!(last[0].data["winners"], json!([ann]));
    assert_eq!(last[0].data["tie"], false);
    assert_eq!(last[0].data["candidates"][0]["votes"], 2);
    assert_eq!(events(decided, "politics.election_decided").len(), 2);
    // The government of the city has its mayor, with the name, in the
    // same frame.
    let city = published(decided, "politics.government_published", &json!(0)).expect("the city");
    assert_eq!(city["offices"][0]["holders"], json!([{ "resident": ann, "name": "Ann" }]));
    assert_eq!(city["treasury"], 2 * FEE);
    assert_eq!(city["next_vote"], json!({ "year": 1, "day": 17 }));
    assert_eq!(city["elections"], json!([]), "the elections of the city are over");
    assert_eq!(common::treasury(&w, CITY), 2 * FEE);
    w.assert_conserved();
}

#[test]
fn every_series_is_published_again_each_day() {
    let mut w = world();
    rich(&mut w, "Ann", 0);
    step_to(&mut w, at(2, 0) - 1);
    let frames = w.step(1);
    assert_eq!(events(&frames, "politics.government_published").len(), 2);
    assert_eq!(events(&frames, "politics.bills_published").len(), 2);
    assert_eq!(events(&frames, "politics.election_published").len(), 2);
    // In the middle of a quiet day nothing is published.
    step_to(&mut w, at(2, 600));
    let quiet = step_to(&mut w, at(2, 900));
    assert_eq!(quiet.len(), 300);
    assert!(events(&quiet, "politics.government_published").is_empty());
    assert!(events(&quiet, "politics.election_published").is_empty());
    assert!(events(&quiet, "politics.bills_published").is_empty());
    w.assert_conserved();
}
