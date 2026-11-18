//! A recall, the campaign of the early election, laws and dismissals.

use serde_json::json;
use unsup_kernel::ResidentId;
use unsup_scenarios::republic::open_election;
use unsup_scenarios::{Game, Republic};

/// Three residents sign a petition against the mayor. The third
/// signature opens an early election, and the mayor runs again. Returns
/// the id of the election.
pub fn start_recall(g: &mut Game, cast: &Republic) -> u64 {
    let [ann, ben, cal, _] = cast.citizens;
    let started = g.act(ann, "start_recall", json!({ "office": "mayor" }));
    let petition = started["recall"].as_u64().expect("a petition id");
    assert_eq!(
        (
            started["signatures"].as_u64(),
            started["needed"].as_u64(),
            started["succeeded"].as_bool()
        ),
        (Some(1), Some(3), Some(false))
    );
    assert_eq!(g.act(ben, "sign_recall", json!({ "recall": petition }))["succeeded"], false);
    let third = g.act(cal, "sign_recall", json!({ "recall": petition }));
    assert_eq!(third["succeeded"], true);
    let election = third["election"].as_u64().expect("an early election");
    assert_eq!(open_election(g, cast.mayor, "mayor"), election);
    g.act(
        cast.mayor,
        "register_candidate",
        json!({ "election": election, "platform": "Four more days" }),
    );
    election
}

/// The council passes two bills. The mayor signs one and vetoes the
/// other.
pub fn laws(g: &mut Game, cast: &Republic) {
    let [cleo, cato] = cast.council;
    let pass = |g: &mut Game, param: &str, value: i64| {
        let bill = g.act(cleo, "propose_law", json!({ "param": param, "value": value, "city": 0 }));
        let id = bill["bill"].as_u64().expect("a bill id");
        g.act(cleo, "vote_law", json!({ "bill": id, "vote": "yes" }));
        assert_eq!(g.act(cato, "vote_law", json!({ "bill": id, "vote": "yes" }))["stage"], "sign");
        id
    };
    let fee = pass(g, "economy.permit_fee", 6_000);
    let tax = pass(g, "economy.sales_tax_bp", 600);
    assert_eq!(g.act(cast.mayor, "sign_law", json!({ "bill": fee }))["stage"], "enacted");
    assert_eq!(g.act(cast.mayor, "veto_law", json!({ "bill": tax }))["stage"], "override");
    assert_eq!(g.param_in("economy.permit_fee", crate::CITY), 6_000);
    assert_eq!(g.param_in("economy.sales_tax_bp", crate::CITY), 500);
}

/// The campaign day: a donation to the fund of the mayor and a rally.
pub fn campaign(g: &mut Game, cast: &Republic, election: u64) {
    let donor = cast.council[0];
    let given = g.act(
        donor,
        "donate",
        json!({ "election": election, "candidate": cast.mayor, "amount": 600 }),
    );
    assert_eq!(given["fund"], 600);
    let rally = g.act(cast.mayor, "campaign", json!({ "election": election, "spend": 600 }));
    assert_eq!((rally["spend"].as_i64(), rally["radius"].as_i64()), (Some(600), Some(12)));
}

/// The voting day: everybody votes for the mayor.
pub fn vote(g: &mut Game, cast: &Republic, election: u64, voters: &[ResidentId]) {
    let votes = g.each(voters, "vote", json!({ "election": election, "candidate": cast.mayor }));
    assert_eq!(votes.last().map(|vote| vote["office"].clone()), Some(json!("mayor")));
}

/// The count at 20:00 of the voting day: the mayor keeps the office.
pub fn count(g: &mut Game, cast: &Republic, votes: usize) {
    if g.frame() < Game::frame_at(9, 20, 2) {
        g.run_to(9, 20, 2);
    }
    let decided = g.last_event("politics.election_decided");
    assert_eq!(decided.frame, Game::frame_at(9, 20, 0));
    assert_eq!(
        (decided.data["kind"].as_str(), decided.data["winner"].as_u64()),
        (Some("recall"), Some(cast.mayor.0))
    );
    assert_eq!(decided.data["tally"][0]["votes"], votes);
}

/// The mayor takes the post of the judge away.
pub fn dismissals(g: &mut Game, cast: &Republic) {
    let gone =
        g.act(cast.mayor, "dismiss", json!({ "post": "judge", "resident": cast.judge, "city": 0 }));
    assert_eq!(gone["post"], "judge");
    g.step(1);
    assert_eq!(g.section(cast.judge, "politics").unwrap_or_default()["offices"], json!(null));
}
