//! The electorate of a bill: the members that vote on it, and that the
//! quorum counts, are the members of the moment when the stage began. A
//! member that leaves afterwards does not lower the quorum and does not
//! take its vote away.

mod common;

use common::{
    CITY, error_of, event, events, notices, ok, plugins, quiet_config, rich, seat, section, step_to,
};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ErrorCode, ResidentId};
use unsup_politics::roles;

/// The parameter that the bills of these tests change: the fee of
/// `org_create` of the society module, 1,000 cents in the data file.
const FEE_PARAM: &str = "society.org_create_fee";

/// A city with a mayor, a council and two residents without an office.
struct Hall {
    w: TestWorld<'static>,
    mayor: ResidentId,
    council: Vec<ResidentId>,
    new: Vec<ResidentId>,
}

fn hall(members: usize) -> Hall {
    let mut w = TestWorld::with_config(&plugins(), quiet_config());
    let mayor = rich(&mut w, "Mayor", 10_000);
    let council: Vec<ResidentId> =
        (0..members).map(|i| rich(&mut w, &format!("Member {i}"), 10_000)).collect();
    let new: Vec<ResidentId> = (0..2).map(|i| rich(&mut w, &format!("New {i}"), 0)).collect();
    seat(&mut w, CITY, roles::MAYOR, &[mayor]);
    seat(&mut w, CITY, roles::COUNCILLOR, &council);
    Hall { w, mayor, council, new }
}

/// Proposes a city bill and returns its id and the frame of its deadline.
fn propose(w: &mut TestWorld<'_>, member: ResidentId, value: i64) -> (u64, u64) {
    let result =
        ok(w, member, "propose_law", json!({ "param": FEE_PARAM, "value": value, "city": 0 }));
    let bill = result["bill"].as_u64().expect("the result has the id of the bill");
    (bill, result["deadline"].as_u64().expect("the result has the deadline"))
}

fn cast(w: &mut TestWorld<'_>, member: ResidentId, bill: u64, vote: &str) -> Value {
    ok(w, member, "vote_law", json!({ "bill": bill, "vote": vote }))
}

/// Returns the fee in the first city, as the global realm has it.
fn fee(w: &TestWorld<'_>) -> i64 {
    let param = w.world.blueprint().param_id(FEE_PARAM).expect("the parameter exists");
    w.global().param_in(param, CityId(0))
}

#[test]
fn the_vote_of_a_member_that_left_still_counts() {
    let Hall { mut w, mayor, council, new } = hall(3);
    let (bill, deadline) = propose(&mut w, council[0], 2_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    // The two members that voted lose their seats before the count.
    seat(&mut w, CITY, roles::COUNCILLOR, &[council[2], new[0], new[1]]);

    // Their votes stay. Two of the three members of the proposal voted:
    // that is a quorum, and the bill passes at the deadline.
    let frames = step_to(&mut w, deadline);
    assert!(events(&frames, "politics.bill_rejected").is_empty());
    let passed = event(&frames, "politics.bill_passed");
    assert_eq!(
        (&passed.data["yes"], &passed.data["no"], &passed.data["members"]),
        (&json!(2), &json!(0), &json!(3))
    );
    ok(&mut w, mayor, "sign_law", json!({ "bill": bill }));
    assert_eq!(fee(&w), 2_000);
    w.assert_conserved();
}

#[test]
fn a_member_that_leaves_does_not_lower_the_quorum() {
    // A council of five: the quorum is three votes.
    let Hall { mut w, council, .. } = hall(5);
    let (_, deadline) = propose(&mut w, council[0], 2_000);
    let bill = section(&w, council[1])["bills"][0]["bill"].as_u64().expect("the bill waits");
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    // Three members leave the council. With a quorum of the members that
    // are left, two votes would be enough.
    seat(&mut w, CITY, roles::COUNCILLOR, &council[..2]);

    let frames = step_to(&mut w, deadline);
    let rejected = event(&frames, "politics.bill_rejected");
    assert_eq!(rejected.data["why"], "no_quorum");
    assert_eq!(
        (&rejected.data["yes"], &rejected.data["no"], &rejected.data["members"]),
        (&json!(2), &json!(0), &json!(5))
    );
    assert_eq!(fee(&w), 1_000);
    w.assert_conserved();
}

#[test]
fn a_member_that_came_later_does_not_vote_on_the_bill() {
    let Hall { mut w, council, new, .. } = hall(3);
    let (bill, _) = propose(&mut w, council[0], 2_000);
    // A fourth member gets a seat after the proposal.
    let mut seats = council.clone();
    seats.push(new[0]);
    seat(&mut w, CITY, roles::COUNCILLOR, &seats);

    // The bill does not wait for the new member, and its vote is refused.
    assert_eq!(section(&w, new[0])["bills"], Value::Null);
    let error = error_of(&mut w, new[0], "vote_law", json!({ "bill": bill, "vote": "no" }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert_eq!(
        error.message,
        "You were not a member when this stage of the bill began. Only the members of that \
         moment vote in it."
    );

    // The stage ends when the three members of the proposal voted.
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    let last = cast(&mut w, council[2], bill, "no");
    assert_eq!(last, json!({ "bill": bill, "vote": "no", "yes": 2, "no": 1, "stage": "sign" }));
    w.assert_conserved();
}

#[test]
fn the_stage_ends_when_every_member_that_can_still_vote_voted() {
    let Hall { mut w, council, .. } = hall(3);
    let (bill, _) = propose(&mut w, council[0], 2_000);
    // One member leaves without a vote. It cannot vote any more.
    seat(&mut w, CITY, roles::COUNCILLOR, &council[..2]);
    assert_eq!(
        error_of(&mut w, council[2], "vote_law", json!({ "bill": bill, "vote": "no" })).code,
        ErrorCode::NotAllowed
    );

    // The stage does not wait for it. Two of three votes are a quorum.
    cast(&mut w, council[0], bill, "yes");
    w.act_ok(council[1], "vote_law", json!({ "bill": bill, "vote": "yes" }));
    let frames = w.step(2);
    let last = w.last_report(council[1]).expect("the vote reports").result;
    assert_eq!(last, json!({ "bill": bill, "vote": "yes", "yes": 2, "no": 0, "stage": "sign" }));
    let passed = event(&frames, "politics.bill_passed");
    assert_eq!(passed.data["members"], 3);
    w.assert_conserved();
}

#[test]
fn the_override_has_the_electorate_of_the_moment_of_the_veto() {
    let Hall { mut w, mayor, council, new } = hall(3);
    let (bill, _) = propose(&mut w, council[0], 2_000);
    for &member in &council {
        cast(&mut w, member, bill, "yes");
    }
    // Between the first vote and the veto, one member gives its seat to a
    // new member.
    seat(&mut w, CITY, roles::COUNCILLOR, &[council[0], council[1], new[0]]);
    let veto = ok(&mut w, mayor, "veto_law", json!({ "bill": bill }));
    let deadline = veto["deadline"].as_u64().expect("a frame");

    // The members of the moment of the veto vote on the override: the new
    // member does, the member that left does not.
    assert_eq!(notices(&w, new[0], "politics.law_vetoed").len(), 1);
    assert_eq!(section(&w, new[0])["bills"][0]["bill"], bill);
    assert_eq!(
        error_of(&mut w, council[2], "vote_law", json!({ "bill": bill, "vote": "no" })).code,
        ErrorCode::NotAllowed
    );
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, new[0], bill, "yes");
    // Both leave after their votes. The votes stay, and the quorum is
    // still two of the three members of the veto.
    seat(&mut w, CITY, roles::COUNCILLOR, &[council[1]]);
    let frames = step_to(&mut w, deadline);
    assert!(events(&frames, "politics.bill_rejected").is_empty());
    let enacted = event(&frames, "politics.law_enacted");
    assert_eq!((&enacted.data["how"], &enacted.data["bill"]), (&json!("override"), &json!(bill)));
    assert_eq!(fee(&w), 2_000);
    w.assert_conserved();
}

#[test]
fn a_snapshot_keeps_the_electorate() {
    let Hall { mut w, council, new, .. } = hall(3);
    let (bill, deadline) = propose(&mut w, council[0], 2_000);
    cast(&mut w, council[0], bill, "yes");
    cast(&mut w, council[1], bill, "yes");
    seat(&mut w, CITY, roles::COUNCILLOR, &[council[2], new[0], new[1]]);
    w.snapshot_restore();
    let frames = step_to(&mut w, deadline);
    let passed = event(&frames, "politics.bill_passed");
    assert_eq!((&passed.data["yes"], &passed.data["members"]), (&json!(2), &json!(3)));
}
