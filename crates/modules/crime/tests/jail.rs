//! Jail: the conditions block verbs, an inmate can still talk, and the
//! jail keeps an inmate alive for a long sentence.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// Cy steals from Bo, the watch sees it, Eve arrests Cy. Returns the
/// world, Bo, Cy, Eve and the case.
fn arrest(
    rules: &[(&'static str, i64)],
) -> (TestWorld<'static>, ResidentId, ResidentId, ResidentId, u64) {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND, rules]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    (w, bo, cy, eve, case)
}

#[test]
fn custody_blocks_most_verbs_and_an_inmate_can_still_talk_eat_and_deal_with_the_court() {
    let (mut w, bo, cy, eve, case) = arrest(&[("crime.trial_ticks", 30)]);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    w.give_item(cy, "bread", 1);
    w.set_attr(cy, "need.food", 500);
    w.set_attr(cy, "need.energy", 500);
    let cell = w.pos(cy).offset(1, 0);
    let other = resident_at(&mut w, "Other", 100, cell);

    for state in ["detained", "jailed"] {
        assert!(w.has_condition(cy, state), "the resident is not {state}");
        // Blocked: walking, crimes, the verbs of the police and the court.
        for (verb, args) in [
            ("move_to", json!({ "x": 100, "y": 20 })),
            ("pickpocket", json!({ "target": other })),
            ("rob", json!({ "target": other })),
            ("assault", json!({ "target": other })),
            ("burgle", json!({})),
            ("vandalize", json!({})),
            ("bribe", json!({ "to": other, "amount": 1 })),
            ("fence", json!({ "item": "bread" })),
            ("patrol", json!({})),
            ("jury_vote", json!({ "case": case, "guilty": false })),
            ("post_bounty", json!({ "target": other, "amount": 500 })),
            ("give", json!({ "to": other, "money": 1 })),
        ] {
            let error = w.act_err(cy, verb, args);
            assert_eq!(error.code, ErrorCode::Blocked, "`{verb}` while {state}");
            assert_eq!(error.details["condition"], json!(state));
        }
        // Allowed: speech, sleep, the docket.
        let said = ok(&mut w, cy, "say", json!({ "text": "I did nothing." }));
        assert_eq!(said["heard_by"], json!(1));
        ok(&mut w, cy, "message", json!({ "to": bo, "text": "Get me a lawyer." }));
        assert_eq!(ok(&mut w, cy, "case_view", json!({ "case": case }))["case"], json!(case));
        assert_eq!(ok(&mut w, cy, "case_list", json!({ "status": "trial" }))["city"], json!(0));
        w.act_ok(cy, "sleep", json!({ "ticks": 30 }));
        w.run_until_idle(cy, 40);
        // The trial ends by the evidence while the resident sleeps: jail.
        if state == "detained" {
            w.step(5);
            assert!(!w.has_condition(cy, "detained"));
        }
    }
    // An inmate eats what it has and pays what it owes.
    ok(&mut w, cy, "eat", json!({}));
    assert_eq!(w.qty(unsup_kernel::Holder::Resident(cy), "bread"), 0);
    assert_eq!(law(&w, cy).expect("in jail")["debts"]["owed"], json!(3_000));
    // A gift reaches an inmate. Other residents cannot hurt it.
    ok(&mut w, other, "give", json!({ "to": cy, "money": 100 }));
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 100, "owed": 2_900 }));
    assert_eq!(fail(&mut w, other, "assault", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, other, "rob", json!({ "target": cy })), ErrorCode::NotAllowed);
    w.assert_conserved();
}

#[test]
fn an_inmate_survives_a_long_sentence_and_is_free_at_the_right_tick() {
    // Ten game days for a theft. The reflexes of the life module are off,
    // so only the jail keeps the inmate alive. A free resident without
    // food dies in about five days.
    let rules = [
        ("crime.theft.jail_ticks", 14_400),
        ("crime.trial_ticks", 30),
        ("crime.fine_due_ticks", 100_000),
        ("life.reflexes", 0),
    ];
    let (mut w, _bo, cy, eve, case) = arrest(&rules);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let events = step_events(&mut w, 35);
    assert_eq!(of_kind(&events, "crime.verdict_given")[0].data["jail_ticks"], json!(14_400));
    let verdict = notice(&w, cy, "crime.verdict").expect("the convict gets the verdict");
    let until = jailed_until(&w, cy).expect("the convict is in jail");
    assert_eq!(until, verdict.tick.0 + 14_400);
    assert_eq!(law(&w, cy).expect("in jail")["custody"]["until"], json!(until));

    // Day after day the inmate has food, energy and health.
    let mut lowest = (1_000, 1_000, 1_000);
    while tick_of(&w, cy) + 720 < until {
        w.step(720);
        assert!(w.is_alive(cy), "the inmate died at tick {}", tick_of(&w, cy));
        assert!(w.has_condition(cy, "jailed"));
        lowest = (
            lowest.0.min(w.attr(cy, "need.food")),
            lowest.1.min(w.attr(cy, "need.energy")),
            lowest.2.min(w.attr(cy, "need.health")),
        );
    }
    // A meal comes below 500 food, rest comes below 250 energy.
    assert!(lowest.0 >= 480, "food fell to {}", lowest.0);
    assert!(lowest.1 >= 200, "energy fell to {}", lowest.1);
    assert_eq!(lowest.2, 1_000);

    // The last tick in jail, and the first tick outside.
    let left = until - tick_of(&w, cy);
    let events = step_events(&mut w, u32::try_from(left - 1).expect("less than a day"));
    assert!(w.has_condition(cy, "jailed"));
    assert_eq!(of_kind(&events, "crime.sentence_served").len(), 0);
    let events = step_events(&mut w, 1);
    assert_eq!(tick_of(&w, cy), until);
    assert!(!w.has_condition(cy, "jailed"));
    assert_eq!(of_kind(&events, "crime.sentence_served").len(), 1);
    assert_eq!(
        notice(&w, cy, "crime.released").expect("the inmate is told").data,
        json!({ "reason": "sentence_served", "free": true })
    );
    // It can walk out.
    w.act_ok(cy, "move_to", json!({ "x": 100, "y": 16 }));
    w.assert_conserved();
}

#[test]
fn a_free_resident_without_food_dies_in_the_same_time() {
    // The control for the test above: the same ten days outside a jail.
    let mut w = town(&[("life.reflexes", 0)]);
    let cy = resident_at(&mut w, "Cy", 0, STREET);
    w.step(14_400);
    assert!(!w.is_alive(cy));
    w.assert_conserved();
}

#[test]
fn a_suspect_that_waits_for_its_trial_is_fed_too() {
    // Hunger and tiredness are off here, so that only the jail changes the
    // needs and the numbers are exact.
    let rules = [
        ("crime.hold_ticks", 9_000),
        ("life.reflexes", 0),
        ("life.food_decay_per_hour", 0),
        ("life.energy_decay_per_hour", 0),
    ];
    let (mut w, _bo, cy, _eve, _case) = arrest(&rules);
    w.set_attr(cy, "need.food", 20);
    w.set_attr(cy, "need.energy", 20);
    // One round of the jail: a meal of 300 and a rest of 150.
    w.step(60);
    assert_eq!(w.attr(cy, "need.food"), 320);
    assert_eq!(w.attr(cy, "need.energy"), 170);
    // The next rounds: one more meal below 500, one more rest below 250.
    w.step(60);
    assert_eq!(w.attr(cy, "need.food"), 620);
    assert_eq!(w.attr(cy, "need.energy"), 320);
    w.step(600);
    assert_eq!(w.attr(cy, "need.food"), 620);
    assert_eq!(w.attr(cy, "need.energy"), 320);
    assert!(w.has_condition(cy, "detained"));
    w.assert_conserved();
}

#[test]
fn a_suspect_is_kept_alive_while_it_waits_with_real_hunger() {
    let rules = [("crime.hold_ticks", 9_000), ("life.reflexes", 0)];
    let (mut w, _bo, cy, _eve, _case) = arrest(&rules);
    w.step(8_000);
    assert!(w.is_alive(cy));
    assert!(w.has_condition(cy, "detained"));
    assert!(w.attr(cy, "need.food") >= 480, "food is {}", w.attr(cy, "need.food"));
    assert_eq!(w.attr(cy, "need.health"), 1_000);
    w.assert_conserved();
}
