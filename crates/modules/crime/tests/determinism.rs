//! Determinism: the same seed and inputs give the same state hash, also
//! across a snapshot and a restore in the middle of cases, custody, trials
//! and jail time.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Money, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// The residents of the scenario.
#[derive(Clone, Copy)]
struct Cast {
    bo: ResidentId,
    cy: ResidentId,
    dee: ResidentId,
    eve: ResidentId,
    finn: ResidentId,
}

/// The parameters of the scenario. The outcomes of crimes are real rolls.
/// The watch is strong and the times are short, so that a short run has
/// warrants, arrests, trials, verdicts, fines and jail.
const RULES: [(&str, i64); 8] = [
    ("crime.watch_detect_bp", 5_000),
    ("crime.watch_arrest_bp", 6_000),
    ("crime.watch_contraband_bp", 5_000),
    ("crime.trial_ticks", 90),
    ("crime.trial_bench_ticks", 30),
    ("crime.hold_ticks", 60),
    ("crime.fine_due_ticks", 150),
    ("crime.theft.jail_ticks", 120),
];

/// Puts the residents into the world. Ids are the same in every run.
fn setup(w: &mut TestWorld<'_>) -> Cast {
    w.step(2);
    let shop = site(w, "shop");
    fill_till(w, &shop, 20_000);
    fill_stock(w, &shop, "bread", 12);
    fill_stock(w, &shop, "moonshine", 3);
    let bo = resident_at(w, "Bo", 50_000, STREET);
    let cy = resident_at(w, "Cy", 300, STREET.offset(1, 0));
    let dee = resident_at(w, "Dee", 900, STREET.offset(0, 1));
    let eve = resident_at(w, "Eve", 0, STREET.offset(1, 1));
    let finn = stranger_at(w, "Finn", 7, 0, STREET.offset(2, 1));
    appoint(w, "police", &["law.arrest", "law.search"], &[eve]);
    appoint(w, "judge", &["law.judge"], &[finn]);
    Cast { bo, cy, dee, eve, finn }
}

/// The first half: crimes of every kind, a patrol, reports and a bounty.
/// A verb that the engine refuses is part of the scenario too.
fn crimes(w: &mut TestWorld<'_>, cast: Cast) {
    let Cast { bo, cy, dee, eve, .. } = cast;
    let shop = site(w, "shop");
    w.act(eve, "patrol", json!({ "ticks": 40 }));
    w.act(bo, "post_bounty", json!({ "target": cy, "amount": 2_000, "ticks": 400 }));
    for round in 0..6 {
        w.act(cy, "pickpocket", json!({ "target": bo }));
        w.act(dee, if round % 2 == 0 { "rob" } else { "assault" }, json!({ "target": bo }));
        w.step(8);
        for case in
            law(w, bo).and_then(|law| law["known_cases"].as_array().cloned()).unwrap_or_default()
        {
            w.act(bo, "report_crime", json!({ "case": case["case"] }));
        }
    }
    w.act(cy, "bribe", json!({ "to": dee, "amount": 50, "note": "Say nothing." }));
    w.set_pos(cy, shop.rect.centre());
    w.set_pos(dee, shop.rect.centre().offset(1, 0));
    w.act(cy, "burgle", json!({}));
    w.act(dee, "vandalize", json!({}));
    w.step(25);
    w.act(cy, "fence", json!({ "item": "bread", "qty": 2 }));
    w.act(eve, "investigate", json!({}));
    w.step(40);
}

/// The second half: the police act, the thieves go to a public building,
/// and time passes: the watch, the trials, the fines and the jail.
fn justice(w: &mut TestWorld<'_>, cast: Cast) {
    let Cast { bo, cy, dee, eve, finn } = cast;
    let shelter = site(w, "shelter");
    let beside = w.pos(cy).offset(0, 1);
    w.set_pos(eve, beside);
    w.act(eve, "search", json!({ "target": cy }));
    w.step(6);
    w.act(eve, "arrest", json!({ "target": cy }));
    w.act(eve, "arrest", json!({ "target": dee }));
    w.step(6);
    for case in ok(w, eve, "case_list", json!({}))["cases"].as_array().cloned().unwrap_or_default()
    {
        w.act(eve, "charge", json!({ "target": cy, "case": case["case"] }));
        w.step(2);
        w.act(finn, "rule_case", json!({ "case": case["case"], "guilty": true, "fine": 500 }));
        w.act(bo, "jury_vote", json!({ "case": case["case"], "guilty": true }));
        w.step(2);
    }
    if !w.has_condition(dee, "detained") && !w.has_condition(dee, "jailed") {
        w.set_pos(dee, shelter.rect.centre());
    }
    w.give_money(cy, Money(100));
    w.act(cy, "pay_fine", json!({}));
    w.step(700);
}

#[test]
fn the_same_seed_and_inputs_give_the_same_hash_also_after_a_restore() {
    let cfg = config(11, &RULES);
    let hash = assert_deterministic(
        &PLUGINS,
        &cfg,
        |w| {
            let cast = setup(w);
            crimes(w, cast);
        },
        |w| {
            // The ids of the residents are the same in every run.
            let cast = Cast {
                bo: ResidentId(1),
                cy: ResidentId(2),
                dee: ResidentId(3),
                eve: ResidentId(4),
                finn: ResidentId(5),
            };
            justice(w, cast);
        },
    );
    // Another seed gives another world.
    let mut other = TestWorld::with_config(&PLUGINS, config(12, &RULES));
    let cast = setup(&mut other);
    crimes(&mut other, cast);
    justice(&mut other, cast);
    assert_ne!(other.hash(), hash);
    other.assert_conserved();
}

#[test]
fn the_scenario_has_cases_custody_trials_and_jail_when_it_is_restored() {
    // The restore of the test above is worth something only when the
    // snapshot holds the state of this module. Check that it does.
    let mut w = TestWorld::with_config(&PLUGINS, config(11, &RULES));
    let cast = setup(&mut w);
    assert_eq!([cast.bo, cast.cy, cast.dee, cast.eve, cast.finn].map(|r| r.0), [1, 2, 3, 4, 5]);
    crimes(&mut w, cast);
    {
        let cases = w.global().res::<unsup_crime::Cases>();
        assert!(cases.len() >= 10, "{} cases", cases.len());
        assert!(cases.iter().any(|case| case.evidence.total() > 0));
    }
    let hot = law(&w, cast.cy).is_some_and(|law| law.get("hot").is_some());
    let wanted = [cast.cy, cast.dee].iter().any(|&r| {
        w.has_condition(r, "wanted")
            || w.has_condition(r, "detained")
            || w.has_condition(r, "jailed")
    });
    assert!(hot || wanted, "nothing of the scenario is in the records");

    // A restore in the middle changes nothing that follows.
    let mut twin = TestWorld::with_config(&PLUGINS, config(11, &RULES));
    let same = setup(&mut twin);
    crimes(&mut twin, same);
    assert_eq!(w.hash(), twin.hash());
    w.snapshot_restore();
    justice(&mut w, cast);
    justice(&mut twin, same);
    assert_eq!(w.hash(), twin.hash());

    // The run ended some trial, and the money is all there.
    let cases = w.global().res::<unsup_crime::Cases>();
    let decided = cases.iter().filter(|case| case.sentence.is_some()).count();
    assert!(decided >= 1, "no trial ended");
    drop(cases);
    w.assert_conserved();
    twin.assert_conserved();
}
