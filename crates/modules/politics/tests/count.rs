//! The count: ties, empty offices, several seats, the end of a term and a
//! candidate that leaves the world.

mod common;

use common::{
    CITY, FEE, at, can, default_config, election_of, events, holders, notices, ok, overrides,
    plugins, register, rich, section, step_to, treasury, vote, world,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ResidentId, WorldConfig};
use unsup_politics::roles;

/// The frame at which the polls of the first city election open.
const POLLS_OPEN: u64 = at_const(3, 360);
/// The frame at which they close.
const POLLS_CLOSE: u64 = at_const(3, 1_200);

/// Returns the frame of a minute of a day, for a constant.
const fn at_const(day: u64, minute: u64) -> u64 {
    (day - 1) * 1_440 + minute
}

/// Runs an election for mayor with two candidates that each get their own
/// vote. Returns the two candidates, the data of the result event, its
/// drama and the state hash after the count.
///
/// The calendar is short, so that the test can run many worlds: the
/// registration is day 1, and the polls are open in the first ten minutes
/// of day 2.
fn tied_election(seed: u64) -> (ResidentId, ResidentId, Value, u8, String) {
    let data = overrides(&[
        ("politics.city_first_vote_day", 2),
        ("politics.campaign_days", 0),
        ("politics.polls_open_minute", 0),
        ("politics.polls_close_minute", 10),
        ("politics.voter_min_age", 0),
    ]);
    let config = WorldConfig { data: vec![data], ..default_config(seed) };
    let mut w = TestWorld::with_config(&plugins(), config);
    let ann = rich(&mut w, "Ann", 10_000);
    let bob = rich(&mut w, "Bob", 10_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Ann first.");
    register(&mut w, bob, mayor, "Bob first.");
    step_to(&mut w, at(2, 0));
    vote(&mut w, ann, mayor, ann);
    vote(&mut w, bob, mayor, bob);
    let frames = step_to(&mut w, at(2, 10));
    let decided = events(&frames, "politics.election_decided")[0];
    assert_eq!(decided.data["election"], mayor);
    assert_eq!(holders(&w, CITY, roles::MAYOR).len(), 1, "one seat, one mayor");
    w.assert_conserved();
    (ann, bob, decided.data.clone(), decided.drama, w.hash())
}

#[test]
fn a_tie_is_broken_by_a_seeded_draw() {
    let mut ann_wins = 0;
    let mut bob_wins = 0;
    for seed in 0..16 {
        let (ann, bob, result, drama, hash) = tied_election(seed);
        assert_eq!(result["tie"], true, "seed {seed}");
        assert_eq!(result["margin"], 0, "seed {seed}");
        assert_eq!(result["turnout"], 2);
        assert_eq!(drama, 95, "a tie is a close election");
        let winners = result["winners"].as_array().expect("a list");
        assert_eq!(winners.len(), 1, "seed {seed}");
        let winner = &winners[0];
        assert_eq!(result["winner"], *winner);
        if *winner == json!(ann) {
            ann_wins += 1;
        } else {
            assert_eq!(*winner, json!(bob), "seed {seed}");
            bob_wins += 1;
        }
        let won = |who| {
            result["tally"].as_array().expect("a list").iter().any(|row| {
                row["candidate"] == json!(who) && row["won"] == true && row["votes"] == 1
            })
        };
        assert_eq!(won(ann), *winner == json!(ann));
        assert_eq!(won(bob), *winner == json!(bob));
        // The same seed gives the same winner and the same world.
        if seed < 4 {
            let (_, _, again, _, hash_again) = tied_election(seed);
            assert_eq!(again["winner"], *winner, "seed {seed}");
            assert_eq!(hash_again, hash, "seed {seed}");
        }
    }
    // The draw is not a rule that favours one candidate: over the seeds
    // each of the two wins some times.
    assert_eq!(ann_wins + bob_wins, 16);
    assert!(ann_wins >= 3, "Ann won {ann_wins} of 16 draws");
    assert!(bob_wins >= 3, "Bob won {bob_wins} of 16 draws");
}

#[test]
fn an_office_with_no_candidate_stays_empty_and_nothing_breaks() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 0);
    w.step(1);
    let council = election_of(&w, ann, "council");
    // Ann runs for the council. Nobody runs for mayor, and nobody votes.
    register(&mut w, ann, council, "A seat for Ann.");
    let frames = step_to(&mut w, POLLS_CLOSE);
    let decided = events(&frames, "politics.election_decided");
    assert_eq!(decided.len(), 2);
    let mayor = &decided[0];
    assert_eq!(mayor.data["office"], "mayor");
    assert_eq!(mayor.data["winner"], Value::Null);
    assert_eq!(mayor.data["winners"], json!([]));
    assert_eq!(mayor.data["tally"], json!([]));
    assert_eq!(mayor.data["turnout"], 0);
    assert_eq!(mayor.data["margin"], Value::Null);
    assert_eq!(mayor.actor, None);
    assert_eq!(mayor.drama, 30, "an empty result is not a headline");
    // A candidate without a vote does not win.
    let seats = &decided[1];
    assert_eq!(seats.data["office"], "council");
    assert_eq!(seats.data["winners"], json!([]));
    assert_eq!(seats.data["tally"], json!([{ "candidate": ann, "votes": 0, "won": false }]));
    assert!(holders(&w, CITY, roles::MAYOR).is_empty());
    assert!(holders(&w, CITY, roles::COUNCILLOR).is_empty());

    // The world goes on. The next term has its election, and a candidate
    // with a vote wins it.
    w.step(1);
    assert_eq!(notices(&w, ann, "politics.election_lost").len(), 1);
    assert!(!can(&w, ann, CITY, "gov.vote"));
    assert_eq!(section(&w, ann)["next_vote"]["city"], json!({ "year": 1, "day": 17 }));
    // No city election is open now. The two that the resident sees are
    // the first elections of the nation, which have no `city`.
    let open: Vec<Value> = section(&w, ann)["elections"].as_array().cloned().unwrap_or_default();
    let offices: Vec<&Value> = open.iter().map(|entry| &entry["office"]).collect();
    assert_eq!(offices, ["president", "legislature"]);
    assert!(open.iter().all(|entry| entry.get("city").is_none()), "{open:?}");
    let frames = step_to(&mut w, at(15, 1));
    let called = events(&frames, "politics.election_called");
    // The nation had its own first election on day 5 in between.
    let city: Vec<_> = called.iter().filter(|e| e.data["gov"] == "city").collect();
    assert_eq!(city.len(), 2);
    assert_eq!(city[0].data["vote_day"], json!({ "year": 1, "day": 17 }));
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Second try.");
    step_to(&mut w, at(17, 360));
    vote(&mut w, bob, mayor, ann);
    let frames = step_to(&mut w, at(17, 1_200));
    let decided = events(&frames, "politics.election_decided");
    assert_eq!(decided[0].data["winners"], json!([ann]));
    assert_eq!(decided[0].data["margin"], Value::Null, "nobody lost");
    assert_eq!(decided[0].drama, 85);
    w.step(1);
    assert!(can(&w, ann, CITY, "gov.sign"));
    w.assert_conserved();
}

/// A world with a council of two seats and a term of three days.
fn short_terms() -> TestWorld<'static> {
    let data = overrides(&[("politics.council_seats", 2), ("politics.city_term_days", 3)]);
    TestWorld::with_config(&plugins(), WorldConfig { data: vec![data], ..WorldConfig::default() })
}

#[test]
fn the_council_seats_go_to_the_candidates_with_the_most_votes() {
    let mut w = short_terms();
    let names = ["Ann", "Bob", "Cat", "Dan", "Eve", "Fay"];
    let r: Vec<ResidentId> = names.iter().map(|name| rich(&mut w, name, 10_000)).collect();
    w.step(1);
    let council = election_of(&w, r[0], "council");
    for &candidate in &r[..3] {
        register(&mut w, candidate, council, "A seat.");
    }
    step_to(&mut w, POLLS_OPEN);
    // Ann gets three votes, Bob two, Cat one.
    for (voter, choice) in [(0, 0), (3, 0), (4, 0), (1, 1), (5, 1), (2, 2)] {
        vote(&mut w, r[voter], council, r[choice]);
    }
    let frames = step_to(&mut w, POLLS_CLOSE);
    let decided = events(&frames, "politics.election_decided")
        .into_iter()
        .find(|e| e.data["office"] == "council")
        .expect("the council election");
    assert_eq!(decided.data["seats"], 2);
    assert_eq!(decided.data["winners"], json!([r[0], r[1]]));
    assert_eq!(decided.data["winner"], json!(r[0]));
    assert_eq!(decided.data["tie"], false);
    assert_eq!(decided.data["margin"], 1, "Bob had one vote more than Cat");
    assert_eq!(decided.drama, 95, "a margin of one vote is close");
    assert_eq!(holders(&w, CITY, roles::COUNCILLOR), [r[0], r[1]]);
    w.step(1);
    for member in [r[0], r[1]] {
        assert!(can(&w, member, CITY, "gov.propose"));
        assert!(can(&w, member, CITY, "gov.vote"));
        assert!(!can(&w, member, CITY, "gov.sign"));
    }
    assert!(!can(&w, r[2], CITY, "gov.vote"));
    w.assert_conserved();
}

#[test]
fn the_winner_of_the_next_term_replaces_the_holder() {
    let mut w = short_terms();
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 20_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Ann.");
    step_to(&mut w, POLLS_OPEN);
    vote(&mut w, ann, mayor, ann);
    step_to(&mut w, POLLS_CLOSE + 1);
    assert!(can(&w, ann, CITY, "gov.sign"));

    // The term has three days: the next voting day is day 6. Its
    // registration opens on day 4.
    assert_eq!(section(&w, ann)["next_vote"]["city"], json!({ "year": 1, "day": 6 }));
    step_to(&mut w, at(4, 1));
    let second = election_of(&w, bob, "mayor");
    assert_ne!(second, mayor);
    register(&mut w, bob, second, "Bob.");
    step_to(&mut w, at(6, 360));
    vote(&mut w, bob, second, bob);
    assert!(can(&w, ann, CITY, "gov.sign"), "Ann holds the office until the count");
    let frames = step_to(&mut w, at(6, 1_200));
    let decided = events(&frames, "politics.election_decided")[0];
    assert_eq!(decided.data["winners"], json!([bob]));
    assert_eq!(decided.data["previous"], json!([ann]));
    w.step(1);
    assert!(can(&w, bob, CITY, "gov.sign"));
    assert!(!can(&w, ann, CITY, "gov.sign"), "the holder before lost the office");
    assert_eq!(holders(&w, CITY, roles::MAYOR), [bob]);
    assert_eq!(notices(&w, ann, "society.org_removed").len(), 1);

    // A term without a candidate empties the office.
    let frames = step_to(&mut w, at(9, 1_200));
    let decided = events(&frames, "politics.election_decided")[0];
    assert_eq!(decided.data["winners"], json!([]));
    assert_eq!(decided.data["previous"], json!([bob]));
    w.step(1);
    assert!(!can(&w, bob, CITY, "gov.sign"));
    assert!(holders(&w, CITY, roles::MAYOR).is_empty());
    w.assert_conserved();
}

#[test]
fn a_candidate_that_leaves_the_world_leaves_the_ballot() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 20_000);
    let bob = rich(&mut w, "Bob", 20_000);
    let cat = rich(&mut w, "Cat", 2_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Ann.");
    register(&mut w, bob, mayor, "Bob.");
    ok(&mut w, cat, "donate", json!({ "election": mayor, "candidate": ann, "amount": 1_500 }));
    step_to(&mut w, POLLS_OPEN);
    vote(&mut w, cat, mayor, ann);
    assert_eq!(treasury(&w, CITY), 2 * FEE);

    w.despawn(ann);
    w.step(1);
    // The fund of the candidate went to the treasury of the city.
    assert_eq!(treasury(&w, CITY), 2 * FEE + 1_500);
    // The voter of the candidate can vote again.
    assert_eq!(section(&w, cat)["elections"][0]["voted"], false);
    assert_eq!(section(&w, cat)["elections"][0]["candidates"], 1);
    vote(&mut w, cat, mayor, bob);
    let frames = step_to(&mut w, POLLS_CLOSE);
    let decided = events(&frames, "politics.election_decided")[0];
    assert_eq!(decided.data["tally"], json!([{ "candidate": bob, "votes": 1, "won": true }]));
    assert_eq!(decided.data["turnout"], 1);
    assert_eq!(holders(&w, CITY, roles::MAYOR), [bob]);
    w.assert_conserved();
}
