//! Story 3: an election leads to a law, and the law changes the tax that
//! the simulation withholds on a wage.
//!
//! The story is the founding of the republic (`unsup_scenarios::republic`)
//! and one law. Twelve newcomers earn the fees of the candidates with
//! public works, vote, and get a mayor and a council. The council passes a
//! bill that raises the income tax of the city. The mayor signs it. The
//! next wage in the city pays the new rate.

use serde_json::json;
use unsup_kernel::CityId;
use unsup_scenarios::republic::{ISSUED, found};
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic};

const CITY: CityId = CityId(0);

/// What the founding left behind: the elections, the offices and the
/// money.
fn check_founding(g: &Game, republic: &Republic) {
    let Republic { mayor, rival, council, president, banker, officer, judge, .. } = *republic;

    // Four candidates paid the fee of 5,000 to the city, one to the nation.
    let registered = g.events_of("politics.candidate_registered");
    assert_eq!(registered.len(), 5);
    assert!(registered.iter().all(|event| event.data["fee"] == 5_000));

    // The count of the city elections at 20:00 of day 3.
    let decided = g.events_of("politics.election_decided");
    assert_eq!(decided.len(), 4, "two city elections and two national elections");
    let (mayor_count, council_count) = (&decided[0].data, &decided[1].data);
    assert_eq!(decided[0].frame, Game::frame_at(3, 20, 0));
    assert_eq!(
        (mayor_count["office"].as_str(), mayor_count["winner"].as_u64()),
        (Some("mayor"), Some(mayor.0))
    );
    assert_eq!(
        mayor_count["tally"],
        json!([
            { "candidate": mayor, "votes": 9, "won": true },
            { "candidate": rival, "votes": 3, "won": false },
        ])
    );
    assert_eq!(
        (mayor_count["turnout"].as_u64(), mayor_count["margin"].as_u64()),
        (Some(12), Some(6))
    );
    assert_eq!(council_count["office"], "council");
    assert_eq!(council_count["winners"], json!(council));
    assert_eq!(
        council_count["tally"],
        json!([
            { "candidate": council[0], "votes": 7, "won": true },
            { "candidate": council[1], "votes": 5, "won": true },
        ])
    );
    // The nation voted on day 5. Nobody ran for the legislature.
    let (president_count, legislature_count) = (&decided[2].data, &decided[3].data);
    assert_eq!(decided[2].frame, Game::frame_at(5, 20, 0));
    assert_eq!(president_count["winners"], json!([president]));
    assert_eq!(president_count["tally"][0]["votes"], 12);
    assert_eq!(
        (legislature_count["office"].as_str(), legislature_count["winner"].as_u64()),
        (Some("legislature"), None)
    );

    // Each official sees its office. The loser has none.
    let office = |r| g.section(r, "politics").unwrap_or_default()["offices"].clone();
    assert_eq!(office(mayor), json!([{ "office": "mayor", "city": 0 }]));
    assert_eq!(office(council[0]), json!([{ "office": "council", "city": 0 }]));
    assert_eq!(office(officer), json!([{ "office": "police_officer", "city": 0 }]));
    assert_eq!(office(judge), json!([{ "office": "judge", "city": 0 }]));
    assert_eq!(office(president), json!([{ "office": "president" }]));
    assert_eq!(office(banker), json!([{ "office": "central_banker" }]));
    assert_eq!(office(rival), json!(null));

    // The central banker issued the money of the year. The nation got the
    // fee of the president and paid one day of salaries: 12,000 to the
    // president and 8,000 to the central banker.
    let issued = g.last_event("finance.money_issued");
    assert_eq!(issued.data, json!({ "resident": banker, "amount": ISSUED }));
    assert_eq!(g.national_treasury(), ISSUED + 5_000 - 12_000 - 8_000);
    // The city could pay its five officials on one day only: 25,000.
    let paid: Vec<(u64, u64)> = g
        .events_of("politics.salaries_paid")
        .iter()
        .filter(|event| event.data["gov"] == "city")
        .map(|event| {
            (event.data["paid"].as_u64().unwrap_or(0), event.data["unpaid"].as_u64().unwrap_or(0))
        })
        .collect();
    assert_eq!(paid, [(5, 0), (2, 3), (0, 5)]);
    assert_eq!(g.city_treasury(CITY), 800);

    // Six days in the world: everybody is alive, fed and rested.
    for resident in republic.everyone() {
        assert!(g.is_alive(resident));
        assert_eq!(g.attr(resident, "need.health"), 1_000);
        assert!(g.attr(resident, "need.food") >= 300, "food of {resident}");
        assert!(g.attr(resident, "need.energy") >= 800, "energy of {resident}");
    }
}

#[test]
fn an_election_leads_to_a_law_and_the_next_wage_pays_the_new_tax() {
    assert_deterministic(&Setup::town(1), |g| {
        let republic = found(g);
        check_founding(g, &republic);
        let Republic { mayor, rival, council, .. } = republic;

        // The wage of today: 2,800 for a shift, 10 percent income tax.
        assert_eq!(g.param_in("economy.income_tax_bp", CITY), 1_000);
        let hall = g.site("city_hall");
        g.walk(rival, &hall);
        let before = g.act(rival, "work", json!({}));
        assert_eq!(
            (before["wage"].as_i64(), before["tax"].as_i64(), before["net"].as_i64()),
            (Some(2_800), Some(280), Some(2_520))
        );

        // A member of the council proposes 25 percent for this city.
        let bill = g.act(
            council[0],
            "propose_law",
            json!({ "param": "economy.income_tax_bp", "value": 2_500, "city": 0, "reason": "The city cannot pay its officials." }),
        );
        assert_eq!((bill["gov"].as_str(), bill["stage"].as_str()), (Some("city"), Some("vote")));
        let id = bill["bill"].as_u64().expect("a bill id");
        // The other member sees the bill that waits for its vote.
        let waiting = g.section(council[1], "politics").expect("a politics section");
        assert_eq!(waiting["bills"][0]["bill"], id);
        assert_eq!(
            (waiting["bills"][0]["now"].as_i64(), waiting["bills"][0]["use"].as_str()),
            (Some(1_000), Some("vote_law"))
        );
        // Somebody that is not a member cannot vote.
        assert_eq!(
            g.refused(rival, "vote_law", json!({ "bill": id, "vote": "no" })),
            unsup_kernel::ErrorCode::NotAllowed
        );

        // Both members vote yes: the bill passes and waits for the mayor.
        let first = g.act(council[0], "vote_law", json!({ "bill": id, "vote": "yes" }));
        assert_eq!((first["yes"].as_u64(), first["stage"].as_str()), (Some(1), Some("vote")));
        let second = g.act(council[1], "vote_law", json!({ "bill": id, "vote": "yes" }));
        assert_eq!((second["yes"].as_u64(), second["stage"].as_str()), (Some(2), Some("sign")));
        // The law is not in force before the signature.
        assert_eq!(g.param_in("economy.income_tax_bp", CITY), 1_000);
        let signed = g.act(mayor, "sign_law", json!({ "bill": id }));
        assert_eq!(signed["stage"], "enacted");
        let enacted = g.last_event("politics.law_enacted");
        assert_eq!(enacted.data["param"], "economy.income_tax_bp");
        assert_eq!(
            (enacted.data["old"].as_i64(), enacted.data["value"].as_i64()),
            (Some(1_000), Some(2_500))
        );
        assert_eq!(
            (enacted.data["how"].as_str(), enacted.data["signed_by"].as_u64()),
            (Some("signed"), Some(mayor.0))
        );
        // The city has the new rate. The nation keeps the old one.
        assert_eq!(g.param_in("economy.income_tax_bp", CITY), 2_500);
        assert_eq!(g.param("economy.income_tax_bp"), 1_000);

        // The next shift in the city: the simulation withholds 25 percent
        // and the city treasury gets it.
        let treasury = g.city_treasury(CITY);
        let cash = g.cash(rival);
        let after = g.act(rival, "work", json!({}));
        assert_eq!(
            (after["wage"].as_i64(), after["tax"].as_i64(), after["net"].as_i64()),
            (Some(2_800), Some(700), Some(2_100))
        );
        assert_eq!(g.cash(rival), cash + 2_100);
        g.step(1);
        assert_eq!(g.city_treasury(CITY), treasury + 700);
        let shift = g.last_event("economy.shift_completed");
        assert_eq!(
            shift.data,
            json!({ "resident": rival, "site": hall.id, "wage": 2_800, "tax": 700, "skill": "labor" })
        );
    });
}
