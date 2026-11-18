//! Laws: a bill from the proposal to a parameter that the simulation
//! obeys, the veto and the override, the quorum and the enactment without
//! a signature.

mod common;

use common::{
    CITY, CITY_B, SQUARE_B, error_of, event, events, fail, notices, ok, plugins, published,
    quiet_config, rich, rich_at, seat, section, step_to, treasury, with_two_cities, zone_param,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ErrorCode, OrgId, ResidentId};
use unsup_politics::roles;

/// The parameter that most bills of these tests change. The society module
/// owns it: the fee of `org_create`, 1,000 cents in the data file.
const FEE_PARAM: &str = "society.org_create_fee";

/// A city with a mayor and a council.
struct Hall {
    w: TestWorld<'static>,
    mayor: ResidentId,
    council: Vec<ResidentId>,
    outsider: ResidentId,
}

/// Makes a world whose first city has a mayor and a council of `members`
/// members. Each resident has cash.
fn hall_in(config: unsup_kernel::WorldConfig, members: usize) -> Hall {
    let mut w = TestWorld::with_config(&plugins(), config);
    let mayor = rich(&mut w, "Mayor", 10_000);
    let council: Vec<ResidentId> =
        (0..members).map(|i| rich(&mut w, &format!("Member {i}"), 10_000)).collect();
    let outsider = rich(&mut w, "Outsider", 10_000);
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    seat(&mut w, CITY, roles::COUNCILLOR, &council);
    Hall { w, mayor, council, outsider }
}

fn hall(members: usize) -> Hall {
    hall_in(quiet_config(), members)
}

/// Proposes a city bill and returns its id.
fn propose(w: &mut TestWorld<'_>, member: ResidentId, param: &str, value: i64) -> u64 {
    let result = ok(w, member, "propose_law", json!({ "param": param, "value": value, "city": 0 }));
    result["bill"].as_u64().expect("the result has the id of the bill")
}

/// Casts the vote of a member and returns the result.
fn cast(w: &mut TestWorld<'_>, member: ResidentId, bill: u64, vote: &str) -> Value {
    ok(w, member, "vote_law", json!({ "bill": bill, "vote": vote }))
}

/// Returns the value of a parameter in the first city, as the global realm
/// has it.
fn city_value(w: &TestWorld<'_>, name: &str) -> i64 {
    let param = w.world.blueprint().param_id(name).expect("the parameter exists");
    w.global().param_in(param, CityId(0))
}

#[test]
fn a_bill_becomes_a_law_that_the_simulation_obeys() {
    let Hall { mut w, mayor, council, outsider } = hall_in(with_two_cities(quiet_config()), 3);
    let visitor = rich_at(&mut w, "Visitor", 10_000, SQUARE_B);
    let frame = w.world.frame();

    // The proposal.
    let ack = w.act(
        council[0],
        "propose_law",
        json!({ "param": FEE_PARAM, "value": 2_500, "city": 0, "reason": "The city needs money." }),
    );
    let frames = w.step(2);
    let report = w.last_report(council[0]).expect("the proposal reports");
    assert_eq!(report.action_id, ack.action_id);
    let bill = report.result["bill"].as_u64().expect("the id of the bill");
    assert_eq!(
        report.result,
        json!({
            "gov": "city", "city": 0, "bill": bill, "param": FEE_PARAM, "value": 2_500,
            "stage": "vote", "deadline": frame + 1 + 720,
        })
    );
    let proposed = event(&frames, "politics.bill_proposed");
    assert_eq!(proposed.actor, Some(council[0]));
    assert_eq!(proposed.data["current"], 1_000);
    assert_eq!(proposed.data["sponsor"], json!(council[0]));
    assert_eq!(
        proposed.data["reason"],
        json!({ "untrusted": true, "from": council[0], "text": "The city needs money." })
    );
    // The other members learn of the bill, and their observation asks for
    // their vote.
    assert!(notices(&w, council[0], "politics.bill_proposed").is_empty());
    for &member in &council[1..] {
        assert_eq!(notices(&w, member, "politics.bill_proposed").len(), 1);
    }
    let waiting = &section(&w, council[1])["bills"];
    assert_eq!(waiting.as_array().map(Vec::len), Some(1));
    assert_eq!(waiting[0]["bill"], bill);
    assert_eq!(waiting[0]["param"], FEE_PARAM);
    assert_eq!((&waiting[0]["value"], &waiting[0]["now"]), (&json!(2_500), &json!(1_000)));
    assert_eq!((&waiting[0]["stage"], &waiting[0]["use"]), (&json!("vote"), &json!("vote_law")));
    assert_eq!(waiting[0]["city"], 0);
    assert_eq!(section(&w, mayor)["bills"], Value::Null, "nothing waits for the mayor yet");
    assert_eq!(section(&w, outsider)["bills"], Value::Null);

    // The vote: two in favour, one against. The last vote ends the stage.
    let first = cast(&mut w, council[0], bill, "yes");
    assert_eq!(first, json!({ "bill": bill, "vote": "yes", "yes": 1, "no": 0, "stage": "vote" }));
    assert_eq!(section(&w, council[0])["bills"], Value::Null, "the member voted");
    cast(&mut w, council[1], bill, "yes");
    w.act_ok(council[2], "vote_law", json!({ "bill": bill, "vote": "no" }));
    let frames = w.step(2);
    let last = w.last_report(council[2]).expect("the vote reports").result;
    assert_eq!(last, json!({ "bill": bill, "vote": "no", "yes": 2, "no": 1, "stage": "sign" }));
    let voted = event(&frames, "politics.bill_voted");
    assert_eq!(voted.data["member"], json!(council[2]));
    assert_eq!((&voted.data["vote"], &voted.data["bill"]), (&json!("no"), &json!(bill)));
    let passed = event(&frames, "politics.bill_passed");
    assert_eq!((&passed.data["yes"], &passed.data["no"]), (&json!(2), &json!(1)));
    assert_eq!(passed.data["members"], 3);
    assert_eq!(notices(&w, mayor, "politics.bill_passed").len(), 1);
    let waiting = &section(&w, mayor)["bills"];
    assert_eq!(waiting[0]["bill"], bill);
    assert_eq!(waiting[0]["use"], "sign_law or veto_law");
    // A bill is not a law yet.
    assert_eq!(city_value(&w, FEE_PARAM), 1_000);

    // The signature. The global realm has the new value at once.
    w.act_ok(mayor, "sign_law", json!({ "bill": bill }));
    let frames = w.step(1);
    let enacted = event(&frames, "politics.law_enacted");
    assert_eq!(enacted.actor, Some(mayor));
    assert_eq!(
        enacted.data,
        json!({
            "gov": "city", "city": 0, "bill": bill, "param": FEE_PARAM, "value": 2_500,
            "old": 1_000, "how": "signed", "sponsor": council[0], "signed_by": mayor,
        })
    );
    assert_eq!(city_value(&w, FEE_PARAM), 2_500);
    assert_eq!(w.param(FEE_PARAM), 1_000, "the national value did not change");
    // The zones have it after the next frame: those of the first city, and
    // not the zone of the second city.
    for zone in 0..5 {
        assert_eq!(zone_param(&w, zone, FEE_PARAM), 1_000, "zone {zone} has not heard yet");
    }
    w.step(1);
    for zone in 0..4 {
        assert_eq!(zone_param(&w, zone, FEE_PARAM), 2_500, "zone {zone} of the first city");
    }
    assert_eq!(zone_param(&w, 4, FEE_PARAM), 1_000, "the zone of the second city");
    assert_eq!(
        w.last_report(mayor).expect("a report").result,
        json!({ "bill": bill, "stage": "enacted" })
    );
    assert_eq!(notices(&w, council[0], "politics.law_enacted").len(), 1);

    // The simulation obeys: a new organization costs the new fee in the
    // first city and the old fee in the second city.
    let before = (treasury(&w, CITY), treasury(&w, CITY_B));
    let club = ok(&mut w, outsider, "org_create", json!({ "name": "Chess Club" }));
    assert_eq!(club["fee"], 2_500);
    assert_eq!(w.cash(outsider).cents(), 10_000 - 2_500);
    let guild = ok(&mut w, visitor, "org_create", json!({ "name": "Guild" }));
    assert_eq!(guild["fee"], 1_000);
    assert_eq!((treasury(&w, CITY), treasury(&w, CITY_B)), (before.0 + 2_500, before.1 + 1_000));

    // The public register has the law.
    let frames = step_to(&mut w, 1_440);
    let register = published(&frames, "politics.bills_published", &json!(0)).expect("published");
    assert_eq!(register["open"], json!([]));
    assert_eq!(register["closed"][0]["bill"], bill);
    assert_eq!(register["closed"][0]["stage"], "enacted");
    assert_eq!(register["closed"][0]["how"], "signed");
    assert_eq!(register["laws"].as_array().map(Vec::len), Some(1));
    assert_eq!(register["laws"][0]["param"], FEE_PARAM);
    assert_eq!(
        (&register["laws"][0]["value"], &register["laws"][0]["old"]),
        (&json!(2_500), &json!(1_000))
    );
    assert_eq!(register["laws"][0]["bill"], bill);
    w.assert_conserved();
}

#[test]
fn a_national_law_changes_the_parameter_in_every_city() {
    let mut w = TestWorld::with_config(&plugins(), with_two_cities(quiet_config()));
    let president = rich(&mut w, "President", 0);
    let members: Vec<ResidentId> =
        (0..3).map(|i| rich(&mut w, &format!("Member {i}"), 0)).collect();
    seat(&mut w, OrgId::NATION, roles::PRESIDENT, &[president]);
    seat(&mut w, OrgId::NATION, roles::LEGISLATOR, &members);
    // A member of the legislature has no voice in a city.
    assert_eq!(
        fail(
            &mut w,
            members[0],
            "propose_law",
            json!({ "param": FEE_PARAM, "value": 7, "city": 0 })
        ),
        ErrorCode::NotAllowed
    );
    let result = ok(&mut w, members[0], "propose_law", json!({ "param": FEE_PARAM, "value": 400 }));
    assert_eq!(result["gov"], "nation");
    assert_eq!(result.get("city"), None);
    let bill = result["bill"].as_u64().expect("the id");
    cast(&mut w, members[0], bill, "yes");
    cast(&mut w, members[1], bill, "yes");
    assert_eq!(cast(&mut w, members[2], bill, "yes")["stage"], "sign");
    ok(&mut w, president, "sign_law", json!({ "bill": bill }));
    assert_eq!(w.param(FEE_PARAM), 400);
    for zone in 0..5 {
        assert_eq!(zone_param(&w, zone, FEE_PARAM), 400, "zone {zone}");
    }
    w.assert_conserved();
}

#[test]
fn each_refusal_of_the_law_verbs_has_its_code() {
    let Hall { mut w, mayor, council, outsider } = hall(3);
    let bill_args = |param: &str, value: i64| json!({ "param": param, "value": value, "city": 0 });

    // propose_law
    assert_eq!(
        fail(&mut w, outsider, "propose_law", bill_args(FEE_PARAM, 5)),
        ErrorCode::NotAllowed
    );
    assert_eq!(
        fail(&mut w, mayor, "propose_law", bill_args(FEE_PARAM, 5)),
        ErrorCode::NotAllowed,
        "the mayor signs, the council proposes"
    );
    assert_eq!(
        fail(&mut w, council[0], "propose_law", json!({ "param": FEE_PARAM, "value": 5 })),
        ErrorCode::NotAllowed,
        "a councillor cannot propose a national law"
    );
    assert_eq!(
        fail(
            &mut w,
            council[0],
            "propose_law",
            json!({ "param": FEE_PARAM, "value": 5, "city": 9 })
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, council[0], "propose_law", bill_args("no.such", 5)),
        ErrorCode::NotFound
    );
    assert_eq!(
        fail(&mut w, council[0], "propose_law", bill_args("politics.platform_max", 100)),
        ErrorCode::NotAllowed,
        "not a policy parameter"
    );
    // The value must be inside the bounds of the parameter. The bounds
    // themselves are allowed.
    for value in [-1, 100_000_001] {
        let error = error_of(&mut w, council[0], "propose_law", bill_args(FEE_PARAM, value));
        assert_eq!(error.code, ErrorCode::InvalidArgs, "value {value}");
        assert_eq!(error.details, json!({ "min": 0, "max": 100_000_000 }));
    }
    let mut blank = bill_args(FEE_PARAM, 5);
    blank["reason"] = json!("  ");
    assert_eq!(fail(&mut w, council[0], "propose_law", blank), ErrorCode::InvalidArgs);
    let bill = propose(&mut w, council[0], FEE_PARAM, 100_000_000);
    // One open bill for a parameter.
    let error = error_of(&mut w, council[1], "propose_law", bill_args(FEE_PARAM, 0));
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.details, json!({ "bill": bill }));
    // Five open bills for a government.
    for param in ["council_seats", "judges_max", "candidate_fee", "salary_mayor"] {
        propose(&mut w, council[1], &format!("politics.{param}"), 2);
    }
    assert_eq!(
        fail(&mut w, council[2], "propose_law", bill_args("politics.salary_judge", 2)),
        ErrorCode::Conflict
    );

    // vote_law
    let ballot = |bill: u64, vote: &str| json!({ "bill": bill, "vote": vote });
    assert_eq!(fail(&mut w, outsider, "vote_law", ballot(bill, "yes")), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, mayor, "vote_law", ballot(bill, "yes")), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, council[0], "vote_law", ballot(77, "yes")), ErrorCode::NotFound);
    let unknown = w.act_err(council[0], "vote_law", json!({ "bill": bill, "vote": "maybe" }));
    assert_eq!(unknown.code, ErrorCode::InvalidArgs);
    cast(&mut w, council[0], bill, "yes");
    assert_eq!(fail(&mut w, council[0], "vote_law", ballot(bill, "no")), ErrorCode::Conflict);

    // sign_law and veto_law come after the vote, from the executive.
    for verb in ["sign_law", "veto_law"] {
        assert_eq!(
            fail(&mut w, mayor, verb, json!({ "bill": bill })),
            ErrorCode::Conflict,
            "{verb}"
        );
        assert_eq!(fail(&mut w, mayor, verb, json!({ "bill": 77 })), ErrorCode::NotFound, "{verb}");
    }
    cast(&mut w, council[1], bill, "yes");
    assert_eq!(cast(&mut w, council[2], bill, "yes")["stage"], "sign");
    for verb in ["sign_law", "veto_law"] {
        for who in [council[0], outsider] {
            assert_eq!(fail(&mut w, who, verb, json!({ "bill": bill })), ErrorCode::NotAllowed);
        }
    }
    // The members cannot vote while the bill waits for the executive.
    assert_eq!(fail(&mut w, council[0], "vote_law", ballot(bill, "no")), ErrorCode::Conflict);
    // Nothing of this changed the parameter.
    assert_eq!(city_value(&w, FEE_PARAM), 1_000);
    ok(&mut w, mayor, "sign_law", json!({ "bill": bill }));
    assert_eq!(city_value(&w, FEE_PARAM), 100_000_000);
    // A closed bill takes no verb.
    assert_eq!(fail(&mut w, mayor, "veto_law", json!({ "bill": bill })), ErrorCode::NotFound);
    w.assert_conserved();
}

#[test]
fn a_veto_stops_a_bill_and_two_thirds_override_it() {
    let Hall { mut w, mayor, council, .. } = hall(3);

    // The members pass the bill, two to one. The mayor vetoes.
    let bill = propose(&mut w, council[0], FEE_PARAM, 3_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    cast(&mut w, council[2], bill, "no");
    let frame = w.world.frame();
    w.act_ok(mayor, "veto_law", json!({ "bill": bill }));
    let frames = w.step(2);
    let report = w.last_report(mayor).expect("the veto reports").result;
    assert_eq!(report, json!({ "bill": bill, "stage": "override", "deadline": frame + 1 + 360 }));
    let vetoed = event(&frames, "politics.law_vetoed");
    assert_eq!(vetoed.actor, Some(mayor));
    assert_eq!((&vetoed.data["bill"], &vetoed.data["by"]), (&json!(bill), &json!(mayor)));
    for &member in &council {
        assert_eq!(notices(&w, member, "politics.law_vetoed").len(), 1);
    }
    assert_eq!(city_value(&w, FEE_PARAM), 1_000, "a vetoed bill is not a law");
    // The override is a new vote: every member can vote again.
    let waiting = &section(&w, council[0])["bills"];
    assert_eq!((&waiting[0]["bill"], &waiting[0]["stage"]), (&json!(bill), &json!("override")));
    // The executive cannot act again.
    assert_eq!(fail(&mut w, mayor, "sign_law", json!({ "bill": bill })), ErrorCode::Conflict);

    // Two of three votes cast are two thirds: the bill is law.
    let first = cast(&mut w, council[0], bill, "yes");
    assert_eq!(
        first,
        json!({ "bill": bill, "vote": "yes", "yes": 1, "no": 0, "stage": "override" })
    );
    cast(&mut w, council[1], bill, "yes");
    w.act_ok(council[2], "vote_law", json!({ "bill": bill, "vote": "no" }));
    let frames = w.step(2);
    assert_eq!(w.last_report(council[2]).expect("a report").result["stage"], "enacted");
    let enacted = event(&frames, "politics.law_enacted");
    assert_eq!(enacted.data["how"], "override");
    assert_eq!(enacted.data["signed_by"], Value::Null);
    assert_eq!(enacted.drama, 75);
    assert_eq!(city_value(&w, FEE_PARAM), 3_000);

    // One of three is not two thirds: the veto stands.
    let bill = propose(&mut w, council[0], FEE_PARAM, 9_000);
    for &member in &council {
        cast(&mut w, member, bill, "yes");
    }
    ok(&mut w, mayor, "veto_law", json!({ "bill": bill }));
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "no");
    w.act_ok(council[2], "vote_law", json!({ "bill": bill, "vote": "no" }));
    let frames = w.step(2);
    assert_eq!(w.last_report(council[2]).expect("a report").result["stage"], "rejected");
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "vetoed");
    assert_eq!((&rejected.data["yes"], &rejected.data["no"]), (&json!(1), &json!(2)));
    assert_eq!(notices(&w, council[0], "politics.bill_rejected").len(), 1);
    assert_eq!(city_value(&w, FEE_PARAM), 3_000);

    // An override that too few members vote on fails at its deadline.
    let bill = propose(&mut w, council[0], FEE_PARAM, 9_000);
    for &member in &council {
        cast(&mut w, member, bill, "yes");
    }
    let veto = ok(&mut w, mayor, "veto_law", json!({ "bill": bill }));
    let deadline = veto["deadline"].as_u64().expect("a frame");
    cast(&mut w, council[0], bill, "yes");
    let frames = step_to(&mut w, deadline - 1);
    assert!(events(&frames, "politics.bill_rejected").is_empty(), "the stage is still open");
    let frames = w.step(1);
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "vetoed");
    assert_eq!((&rejected.data["yes"], &rejected.data["members"]), (&json!(1), &json!(3)));
    assert_eq!(city_value(&w, FEE_PARAM), 3_000);
    w.assert_conserved();
}

#[test]
fn a_majority_that_is_not_two_thirds_does_not_override() {
    let Hall { mut w, mayor, council, .. } = hall(5);
    for (yes, stage, value_after) in [(3, "rejected", 1_000), (4, "enacted", 7_000)] {
        let bill = propose(&mut w, council[0], FEE_PARAM, 7_000);
        for &member in &council {
            cast(&mut w, member, bill, "yes");
        }
        ok(&mut w, mayor, "veto_law", json!({ "bill": bill }));
        // Three of five are a majority and less than two thirds. Four of
        // five are more than two thirds.
        let mut last = Value::Null;
        for (i, &member) in council.iter().enumerate() {
            last = cast(&mut w, member, bill, if i < yes { "yes" } else { "no" });
        }
        assert_eq!(last["stage"], stage, "{yes} votes in favour");
        assert_eq!((&last["yes"], &last["no"]), (&json!(yes), &json!(5 - yes)));
        assert_eq!(city_value(&w, FEE_PARAM), value_after, "{yes} votes in favour");
    }
    w.assert_conserved();
}

#[test]
fn a_decision_needs_a_quorum_and_a_majority_of_the_votes_cast() {
    // A council of five: the quorum is three votes.
    let Hall { mut w, mayor, council, .. } = hall(5);

    // Two votes are not a quorum, also when both are in favour.
    let bill = propose(&mut w, council[0], FEE_PARAM, 2_000);
    let deadline = w.last_report(council[0]).expect("a report").result["deadline"].as_u64();
    let deadline = deadline.expect("a frame");
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    let frames = step_to(&mut w, deadline - 1);
    assert!(events(&frames, "politics.bill_rejected").is_empty(), "the vote is still open");
    let frames = w.step(1);
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "no_quorum");
    assert_eq!((&rejected.data["yes"], &rejected.data["no"]), (&json!(2), &json!(0)));
    assert_eq!(rejected.data["members"], 5);
    assert_eq!(city_value(&w, FEE_PARAM), 1_000);

    // Three votes are a quorum. Two in favour and one against pass at the
    // deadline, and the bill goes to the mayor.
    let bill = propose(&mut w, council[0], FEE_PARAM, 2_000);
    let deadline = w.last_report(council[0]).expect("a report").result["deadline"].as_u64();
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    cast(&mut w, council[2], bill, "no");
    assert_eq!(section(&w, mayor)["bills"], Value::Null, "the vote is still open");
    let frames = step_to(&mut w, deadline.expect("a frame"));
    let passed = event(&frames, "politics.bill_passed");
    assert_eq!((&passed.data["yes"], &passed.data["no"]), (&json!(2), &json!(1)));
    ok(&mut w, mayor, "sign_law", json!({ "bill": bill }));
    assert_eq!(city_value(&w, FEE_PARAM), 2_000);

    // With a quorum, a tie fails: the votes in favour must be more.
    let bill = propose(&mut w, council[0], FEE_PARAM, 5_000);
    let deadline = w.last_report(council[0]).expect("a report").result["deadline"].as_u64();
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    cast(&mut w, council[2], bill, "no");
    cast(&mut w, council[3], bill, "no");
    let frames = step_to(&mut w, deadline.expect("a frame"));
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "voted_down");
    assert_eq!((&rejected.data["yes"], &rejected.data["no"]), (&json!(2), &json!(2)));
    assert_eq!(city_value(&w, FEE_PARAM), 2_000);
    w.assert_conserved();
}

#[test]
fn only_the_votes_of_the_members_at_the_count_count() {
    let Hall { mut w, council, .. } = hall(3);
    let new: Vec<ResidentId> = (0..2).map(|i| rich(&mut w, &format!("New {i}"), 0)).collect();
    let bill = propose(&mut w, council[0], FEE_PARAM, 2_000);
    let deadline = w.last_report(council[0]).expect("a report").result["deadline"].as_u64();
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    // Two members lose their seats before the count.
    seat(&mut w, CITY, roles::COUNCILLOR, &[council[2], new[0], new[1]]);
    let frames = step_to(&mut w, deadline.expect("a frame"));
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "no_quorum");
    assert_eq!((&rejected.data["yes"], &rejected.data["members"]), (&json!(0), &json!(3)));
    assert_eq!(city_value(&w, FEE_PARAM), 1_000);
    w.assert_conserved();
}

#[test]
fn a_bill_becomes_law_without_a_signature() {
    let Hall { mut w, mayor, council, .. } = hall(3);

    // The mayor does nothing: the bill is law at the deadline.
    let bill = propose(&mut w, council[0], FEE_PARAM, 2_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    w.act_ok(council[2], "vote_law", json!({ "bill": bill, "vote": "yes" }));
    w.step(1);
    let passed_at = w.world.frame();
    let frames = step_to(&mut w, passed_at + 359);
    assert!(events(&frames, "politics.law_enacted").is_empty(), "the mayor still has time");
    assert_eq!(city_value(&w, FEE_PARAM), 1_000);
    let frames = w.step(1);
    let enacted = event(&frames, "politics.law_enacted");
    assert_eq!(enacted.data["how"], "deadline");
    assert_eq!(enacted.data["bill"], bill);
    assert_eq!(enacted.actor, None);
    assert_eq!(city_value(&w, FEE_PARAM), 2_000);

    // The mayor leaves the office while a bill waits: the bill is law.
    let bill = propose(&mut w, council[0], FEE_PARAM, 4_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    assert_eq!(cast(&mut w, council[2], bill, "yes")["stage"], "sign");
    w.act_ok(mayor, "org_leave", json!({ "org": CITY }));
    let frames = w.step(2);
    let enacted = event(&frames, "politics.law_enacted");
    assert_eq!((&enacted.data["how"], &enacted.data["bill"]), (&json!("vacant"), &json!(bill)));
    assert_eq!(city_value(&w, FEE_PARAM), 4_000);

    // The office is empty when the members pass a bill: it is law at once.
    let bill = propose(&mut w, council[0], FEE_PARAM, 6_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    w.act_ok(council[2], "vote_law", json!({ "bill": bill, "vote": "no" }));
    let frames = w.step(2);
    assert_eq!(w.last_report(council[2]).expect("a report").result["stage"], "enacted");
    assert!(events(&frames, "politics.bill_passed").is_empty(), "nobody is there to sign");
    assert_eq!(event(&frames, "politics.law_enacted").data["how"], "vacant");
    assert_eq!(city_value(&w, FEE_PARAM), 6_000);
    w.assert_conserved();
}
