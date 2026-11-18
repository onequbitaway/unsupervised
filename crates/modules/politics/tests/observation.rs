//! The observation section `politics`.

mod common;

use common::{
    CITY, SQUARE_B, at, default_config, election_of, ok, overrides, plugins, register, rich,
    rich_at, section, step_to, vote, with_two_cities, world,
};
use serde_json::{Value, json};
use unsup_kernel::WorldConfig;
use unsup_kernel::protocol::{Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;

#[test]
fn a_resident_sees_the_calendar_and_the_open_elections_of_its_city() {
    let mut w = TestWorld::with_config(&plugins(), with_two_cities(default_config(0)));
    let ann = rich(&mut w, "Ann", 0);
    let ben = rich_at(&mut w, "Ben", 0, SQUARE_B);
    // The global realm learns of a new resident in the next frame. Until
    // then it does not know the city, and the section has the calendar of
    // the nation only.
    assert_eq!(section(&w, ann), json!({ "next_vote": { "nation": { "year": 1, "day": 5 } } }));
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    let council = election_of(&w, ann, "council");
    assert_eq!(
        section(&w, ann),
        json!({
            "next_vote": {
                "city": { "year": 1, "day": 3 },
                "nation": { "year": 1, "day": 5 },
            },
            "elections": [
                { "city": 0, "election": mayor, "office": "mayor", "phase": "registration",
                  "minutes_left": 1_439, "candidates": 0 },
                { "city": 0, "election": council, "office": "council", "phase": "registration",
                  "minutes_left": 1_439, "candidates": 0 },
            ],
        })
    );
    // A resident of the other city sees the elections of that city.
    let there = section(&w, ben);
    assert_eq!(there["elections"][0]["city"], 1);
    assert_ne!(there["elections"][0]["election"], json!(mayor));
    assert_eq!(there["elections"].as_array().map(Vec::len), Some(2));
    // The section follows the resident.
    w.set_pos(ben, common::SQUARE);
    w.step(1);
    assert_eq!(section(&w, ben)["elections"][0]["election"], mayor);
    w.assert_conserved();
}

#[test]
fn the_brief_section_is_small_and_the_full_section_has_the_ballot() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 10_000);
    let bob = rich(&mut w, "Bob", 10_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "Lower taxes.");
    register(&mut w, bob, mayor, "More parks.");
    // Day 3: the city votes, and the registration of the nation is open.
    step_to(&mut w, at(3, 360));
    vote(&mut w, bob, mayor, ann);
    let brief = section(&w, bob);
    let open = brief["elections"].as_array().expect("a list");
    let offices: Vec<&Value> = open.iter().map(|entry| &entry["office"]).collect();
    assert_eq!(offices, ["mayor", "council", "president", "legislature"]);
    assert_eq!(open[0]["phase"], "voting");
    assert_eq!(open[0]["voted"], true);
    assert_eq!(open[0]["candidates"], 2);
    assert_eq!(open[0]["minutes_left"], 840 - 2, "the polls close at 20:00");
    assert_eq!(open[1]["voted"], false);
    assert_eq!(open[2].get("voted"), None, "the nation does not vote yet");
    assert_eq!(open[0].get("ballot"), None, "the brief section has no ballot");
    assert_eq!(brief["candidacies"][0]["election"], mayor);
    // The brief section stays near 150 tokens, also on a busy day.
    let size = brief.to_string().len();
    assert!(size < 700, "the brief section has {size} bytes: {brief}");

    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let full = w.observe_with(bob, &opts).sections["politics"].clone();
    assert_eq!(
        full["elections"][0]["ballot"],
        json!([
            { "candidate": ann, "votes": 1,
              "platform": { "untrusted": true, "from": ann, "text": "Lower taxes." } },
            { "candidate": bob, "votes": 0,
              "platform": { "untrusted": true, "from": bob, "text": "More parks." } },
        ])
    );
    assert_eq!(full["elections"][1]["ballot"], json!([]));
    // The `sections` option leaves the section out.
    let other = ObserveOpts { sections: Some(vec!["self".into()]), ..ObserveOpts::default() };
    assert!(!w.observe_with(bob, &other).sections.contains_key("politics"));
    w.assert_conserved();
}

#[test]
fn the_lists_of_the_brief_section_have_a_limit() {
    let data = overrides(&[("politics.brief_list_max", 2)]);
    let config = WorldConfig { data: vec![data], ..default_config(0) };
    let mut w = TestWorld::with_config(&plugins(), config);
    let ann = rich(&mut w, "Ann", 0);
    step_to(&mut w, at(3, 1));
    let brief = section(&w, ann);
    assert_eq!(brief["elections"].as_array().map(Vec::len), Some(2));
    assert_eq!(brief["more_elections"], 2);
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let full = w.observe_with(ann, &opts).sections["politics"].clone();
    assert_eq!(full["elections"].as_array().map(Vec::len), Some(4));
    assert_eq!(full.get("more_elections"), None);
    w.assert_conserved();
}

#[test]
fn an_observation_does_not_change_the_world() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 10_000);
    w.step(1);
    let mayor = election_of(&w, ann, "mayor");
    register(&mut w, ann, mayor, "A platform.");
    ok(&mut w, ann, "donate", json!({ "election": mayor, "candidate": ann, "amount": 100 }));
    let before = w.hash();
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let first = w.observe_with(ann, &opts).sections["politics"].clone();
    let second = w.observe_with(ann, &opts).sections["politics"].clone();
    assert_eq!(first, second);
    assert_eq!(first["candidacies"][0]["fund"], 100);
    assert_eq!(w.hash(), before);
    assert_eq!(common::treasury(&w, CITY), common::FEE);
    w.assert_conserved();
}
