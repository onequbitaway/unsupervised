//! Determinism: the same seed and the same inputs give the same world,
//! also when the world is restored from a snapshot on the way.

mod common;

use common::{
    CITY, at, default_config, election_of, holders, ok, plugins, register, rich, rich_at, section,
    step_to, vote,
};
use serde_json::json;
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Pos, ResidentId};
use unsup_politics::roles;

/// The residents of the scenario, in the order in which they are made.
/// Resident ids count from 1 in every world of the same inputs.
const ANN: ResidentId = ResidentId(1);
const BOB: ResidentId = ResidentId(2);
const CAT: ResidentId = ResidentId(3);
const DAN: ResidentId = ResidentId(4);

/// What a scenario does at each of its stages: nothing, or a restore of
/// the world from its own snapshot.
type Checkpoint = fn(&mut TestWorld<'_>);

fn go_on(_: &mut TestWorld<'_>) {}

fn restore(w: &mut TestWorld<'_>) {
    w.snapshot_restore();
}

/// Day 1 and day 2: the candidates register, a donor gives and a rally
/// starts.
fn first_days(w: &mut TestWorld<'_>, checkpoint: Checkpoint) {
    // An empty world with the calendar of the first frame.
    checkpoint(w);
    assert_eq!(rich(w, "Ann", 30_000), ANN);
    assert_eq!(rich(w, "Bob", 30_000), BOB);
    assert_eq!(rich_at(w, "Cat", 30_000, Pos::new(24, 22)), CAT);
    assert_eq!(rich_at(w, "Dan", 5_000, Pos::new(70, 20)), DAN);
    w.step(1);
    let mayor = election_of(w, ANN, "mayor");
    let council = election_of(w, ANN, "council");
    register(w, ANN, mayor, "Ann for mayor.");
    register(w, BOB, mayor, "Bob for mayor.");
    register(w, CAT, council, "Cat for the council.");
    ok(w, DAN, "donate", json!({ "election": mayor, "candidate": ANN, "amount": 2_000 }));
    step_to(w, at(2, 10));
    w.act_ok(ANN, "campaign", json!({ "election": mayor, "spend": 900 }));
    w.step(12);
}

/// The rest of the rally, the vote with a tie for mayor, the count with
/// its draw, a law with a veto and an override, a post, a recall petition
/// and the salaries of two days.
fn later_days(w: &mut TestWorld<'_>, checkpoint: Checkpoint) {
    // Candidates, a fund and a rally that is not over.
    checkpoint(w);
    let mayor = election_of(w, ANN, "mayor");
    let council = election_of(w, ANN, "council");
    step_to(w, at(3, 360));
    vote(w, ANN, mayor, ANN);
    // Open polls with one vote.
    checkpoint(w);
    vote(w, BOB, mayor, BOB);
    vote(w, CAT, council, CAT);
    vote(w, DAN, council, CAT);
    step_to(w, at(3, 1_200) + 1);
    // The draw gave the office to one of the two. The council has Cat.
    checkpoint(w);
    let mayors = holders(w, CITY, roles::MAYOR);
    assert_eq!(mayors.len(), 1);
    let winner = mayors[0];
    assert!(winner == ANN || winner == BOB);
    assert_eq!(holders(w, CITY, roles::COUNCILLOR), [CAT]);
    let bill = ok(
        w,
        CAT,
        "propose_law",
        json!({ "param": "politics.salary_councillor", "value": 4_500, "city": 0 }),
    );
    let bill = bill["bill"].as_u64().expect("the id");
    ok(w, CAT, "vote_law", json!({ "bill": bill, "vote": "yes" }));
    ok(w, winner, "veto_law", json!({ "bill": bill }));
    // A vetoed bill in its override stage.
    checkpoint(w);
    ok(w, CAT, "vote_law", json!({ "bill": bill, "vote": "yes" }));
    ok(w, winner, "appoint", json!({ "post": "judge", "resident": DAN }));
    ok(w, DAN, "start_recall", json!({ "office": "mayor" }));
    // A law, a post and an open petition.
    checkpoint(w);
    step_to(w, at(5, 30));
    assert_eq!(section(w, CAT)["offices"], json!([{ "city": 0, "office": "council" }]));
    // The new salary of the law was paid two times.
    assert_eq!(w.cash(CAT).cents(), 30_000 - common::FEE + 2 * 4_500);
    w.assert_conserved();
    checkpoint(w);
}

#[test]
fn the_same_inputs_give_the_same_world_also_after_a_restore() {
    for seed in [0, 7] {
        let hash = assert_deterministic(
            &plugins(),
            &default_config(seed),
            |w| first_days(w, go_on),
            |w| later_days(w, go_on),
        );
        assert_eq!(hash.len(), 64);
    }
}

/// Runs the whole scenario and returns the state hash and the mayor.
fn run(seed: u64, checkpoint: Checkpoint) -> (String, ResidentId) {
    let mut w = TestWorld::with_config(&plugins(), default_config(seed));
    first_days(&mut w, checkpoint);
    later_days(&mut w, checkpoint);
    step_to(&mut w, at(6, 0));
    (w.hash(), holders(&w, CITY, roles::MAYOR)[0])
}

#[test]
fn another_seed_gives_another_world() {
    assert_ne!(run(0, go_on).0, run(7, go_on).0);
}

#[test]
fn a_restore_at_every_stage_changes_nothing() {
    for seed in [3, 4] {
        assert_eq!(run(seed, restore), run(seed, go_on), "seed {seed}");
    }
}
