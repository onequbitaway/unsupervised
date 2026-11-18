//! Several cities and the nation: each city has its own elections, and
//! the national election is open to every city.

mod common;

use common::{
    CITY, CITY_B, FEE, SQUARE_B, at, can, default_config, election_of, events, fail, holders,
    plugins, register, rich, rich_at, section, step_to, treasury, vote, with_two_cities,
};
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, OrgId};
use unsup_politics::roles;

fn two_cities() -> TestWorld<'static> {
    TestWorld::with_config(&plugins(), with_two_cities(default_config(0)))
}

#[test]
fn a_city_election_belongs_to_its_city() {
    let mut w = two_cities();
    let ann = rich(&mut w, "Ann", 20_000);
    let val = rich(&mut w, "Val", 0);
    let ben = rich_at(&mut w, "Ben", 20_000, SQUARE_B);
    let frames = w.step(1);
    let called = events(&frames, "politics.election_called");
    let places: Vec<_> = called.iter().map(|e| (&e.data["city"], &e.data["office"])).collect();
    assert_eq!(
        places,
        [
            (&json!(0), &json!("mayor")),
            (&json!(0), &json!("council")),
            (&json!(1), &json!("mayor")),
            (&json!(1), &json!("council"))
        ]
    );
    let mayor_a = election_of(&w, ann, "mayor");
    let mayor_b = election_of(&w, ben, "mayor");
    assert_ne!(mayor_a, mayor_b);

    // A candidate stands in the city of the office.
    assert_eq!(
        fail(&mut w, ann, "register_candidate", json!({ "election": mayor_b, "platform": "Ann." })),
        ErrorCode::NotHere
    );
    assert_eq!(w.cash(ann).cents(), 20_000, "the fee came back");
    register(&mut w, ann, mayor_a, "Ann.");
    register(&mut w, ben, mayor_b, "Ben.");
    assert_eq!((treasury(&w, CITY), treasury(&w, CITY_B)), (FEE, FEE));

    // The campaign is in the city too.
    w.act_ok(ann, "donate", json!({ "election": mayor_a, "candidate": ann, "amount": 500 }));
    step_to(&mut w, at(2, 0));
    w.set_pos(ann, SQUARE_B);
    assert_eq!(
        fail(&mut w, ann, "campaign", json!({ "election": mayor_a, "spend": 200 })),
        ErrorCode::NotHere
    );
    w.set_pos(ann, common::SQUARE);

    // A resident votes in the city in which it is, one time for each
    // office.
    step_to(&mut w, at(3, 360));
    assert_eq!(
        fail(&mut w, val, "vote", json!({ "election": mayor_b, "candidate": ben })),
        ErrorCode::NotHere
    );
    vote(&mut w, val, mayor_a, ann);
    w.set_pos(val, SQUARE_B);
    assert_eq!(
        fail(&mut w, val, "vote", json!({ "election": mayor_b, "candidate": ben })),
        ErrorCode::Conflict,
        "one vote for the office of mayor"
    );
    vote(&mut w, ben, mayor_b, ben);

    let frames = step_to(&mut w, at(3, 1_200) + 1);
    let decided = events(&frames, "politics.election_decided");
    assert_eq!(decided.len(), 4);
    assert_eq!(holders(&w, CITY, roles::MAYOR), [ann]);
    assert_eq!(holders(&w, CITY_B, roles::MAYOR), [ben]);
    assert!(can(&w, ann, CITY, "gov.sign") && !can(&w, ann, CITY_B, "gov.sign"));
    assert!(can(&w, ben, CITY_B, "gov.sign") && !can(&w, ben, CITY, "gov.sign"));
    // The power of an office names its city.
    assert!(can(&w, ben, CITY_B, "gov.sign@101"));
    w.assert_conserved();
}

#[test]
fn the_national_election_is_open_to_every_city() {
    let mut w = two_cities();
    let pat = rich(&mut w, "Pat", 20_000);
    let sam = rich_at(&mut w, "Sam", 20_000, SQUARE_B);
    assert_eq!(section(&w, pat)["next_vote"]["nation"], json!({ "year": 1, "day": 5 }));

    // The registration of the first national election opens on day 3.
    let frames = step_to(&mut w, at(3, 1));
    let called: Vec<_> = events(&frames, "politics.election_called")
        .into_iter()
        .filter(|e| e.data["gov"] == "nation")
        .collect();
    assert_eq!(called.len(), 2);
    assert_eq!(called[0].data["office"], "president");
    assert_eq!(called[0].data.get("city"), None);
    assert_eq!(called[0].data["vote_day"], json!({ "year": 1, "day": 5 }));
    assert_eq!(called[1].data["office"], "legislature");
    assert_eq!(called[1].data["seats"], 7);
    let president = election_of(&w, pat, "president");
    assert_eq!(election_of(&w, sam, "president"), president, "both cities see it");
    let legislature = election_of(&w, sam, "legislature");

    // The fee of a national office goes to the national treasury.
    register(&mut w, pat, president, "One nation.");
    register(&mut w, sam, legislature, "A voice for Brine.");
    assert_eq!(treasury(&w, OrgId::NATION), 2 * FEE);

    step_to(&mut w, at(5, 360));
    vote(&mut w, pat, president, pat);
    vote(&mut w, sam, president, pat);
    vote(&mut w, sam, legislature, sam);
    let frames = step_to(&mut w, at(5, 1_200) + 1);
    let decided = events(&frames, "politics.election_decided");
    assert_eq!(decided.len(), 2);
    assert_eq!(decided[0].data["gov"], "nation");
    assert_eq!(decided[0].data["winners"], json!([pat]));
    assert_eq!(decided[0].data["turnout"], 2);
    assert_eq!(decided[1].data["winners"], json!([sam]));
    for perm in ["gov.sign", "gov.appoint", "treasury.spend"] {
        assert!(can(&w, pat, OrgId::NATION, perm), "{perm}");
    }
    assert!(can(&w, sam, OrgId::NATION, "gov.propose") && can(&w, sam, OrgId::NATION, "gov.vote"));
    assert!(!can(&w, pat, CITY, "gov.sign"), "a national office gives nothing in a city");
    assert_eq!(section(&w, pat)["offices"], json!([{ "office": "president" }]));
    assert_eq!(section(&w, sam)["offices"], json!([{ "office": "legislature" }]));
    // The next national term: 28 days later.
    assert_eq!(section(&w, pat)["next_vote"]["nation"], json!({ "year": 2, "day": 5 }));
    w.assert_conserved();
}
