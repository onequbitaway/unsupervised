//! The condition `wanted` and the warrants of a resident.
//!
//! One function of the module sets the condition: it gives `wanted` to a
//! resident while a warrant names it and it is not under arrest. So an
//! arrest that the condition allows always has a case behind it, and a
//! resident cannot make another resident wanted. This test follows one
//! thief through every step that changes a warrant and checks that rule in
//! each frame.

mod common;

use common::*;
use serde_json::json;
use unsup_crime::Record;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// What the world says about a resident: the condition `wanted`, the
/// number of warrants in its record, and the condition `detained`.
fn state(w: &TestWorld<'_>, r: ResidentId) -> (bool, usize, bool) {
    let warrants = w.with_resident(r, |realm, idx| realm.col::<Record>()[idx].warrants().len());
    (w.has_condition(r, "wanted"), warrants, w.has_condition(r, "detained"))
}

/// Checks the rule: a resident is wanted exactly while a warrant names it
/// and it is not under arrest.
fn check(w: &TestWorld<'_>, r: ResidentId, when: &str) {
    let (wanted, warrants, detained) = state(w, r);
    assert_eq!(
        wanted,
        warrants > 0 && !detained,
        "{when}, frame {}: wanted {wanted}, {warrants} warrants, detained {detained}",
        w.world.frame()
    );
}

/// Runs frames one at a time and checks the rule after each.
fn frames(w: &mut TestWorld<'_>, r: ResidentId, count: u32, when: &str) {
    for _ in 0..count {
        w.step(1);
        check(w, r, when);
    }
}

#[test]
fn a_resident_is_wanted_only_while_a_warrant_names_it() {
    let rules = [ALWAYS, WATCH_SEES_ALL, ("crime.case_ttl_ticks", 300)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    assert_eq!(state(&w, cy), (false, 0, false), "nobody is wanted before a crime");
    // A victim is never wanted, whatever happens to the thief.
    let victim_is_clean = |w: &TestWorld<'_>| assert_eq!(state(w, bo), (false, 0, false));

    // The watch sees the theft: a warrant, and with it the condition.
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    frames(&mut w, cy, 1, "after the theft");
    assert_eq!(state(&w, cy), (true, 1, false));
    victim_is_clean(&w);

    // An arrest serves the warrant: the condition ends with it.
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    check(&w, cy, "after the arrest");
    assert_eq!(state(&w, cy), (false, 0, true));

    // The officer lets the suspect go without a charge. The case is open
    // again, and its warrant comes back from the registry.
    let beside = w.pos(cy).offset(1, 0);
    w.set_pos(eve, beside);
    ok(&mut w, eve, "release", json!({ "target": cy }));
    check(&w, cy, "after the release");
    frames(&mut w, cy, 3, "after the release");
    assert_eq!(state(&w, cy), (true, 1, false));

    // A new arrest and a charge: the warrant is gone for good.
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    frames(&mut w, cy, 2, "after the charge");
    assert_eq!(state(&w, cy), (false, 0, true));

    // An acquittal frees the resident, and nothing names it any more.
    ok(&mut w, finn, "rule_case", json!({ "case": case, "guilty": false }));
    frames(&mut w, cy, 3, "after the acquittal");
    assert_eq!(state(&w, cy), (false, 0, false));

    // A second theft: a new warrant. Nobody arrests the thief, and the
    // case goes cold. The warrant ends, and the condition ends with it.
    let beside = w.pos(bo).offset(1, 0);
    w.set_pos(cy, beside);
    ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    frames(&mut w, cy, 1, "after the second theft");
    assert_eq!(state(&w, cy), (true, 1, false));
    frames(&mut w, cy, 305, "while the second case goes cold");
    assert_eq!(state(&w, cy), (false, 0, false));
    victim_is_clean(&w);
    w.assert_conserved();
}
