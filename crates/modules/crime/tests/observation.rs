//! The observation section `law`, and the module in a world without the
//! other modules.

mod common;

use common::*;
use serde_json::json;
use unsup_crime::CrimePlugin;
use unsup_kernel::protocol::{Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityId, ErrorCode, Money, OrgId, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn the_law_section_is_absent_for_a_clean_resident_and_small_for_a_busy_one() {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &SHARP]));
    let bo = resident_at(&mut w, "Bo", 100_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let eve = resident_at(&mut w, "Eve", 0, Pos::new(80, 80));
    appoint(&mut w, "police", &["law.arrest", "law.search", "law.judge"], &[eve]);
    assert_eq!(law(&w, bo), None);
    assert_eq!(law(&w, cy), None);
    // An officer sees its powers in the city where it stands.
    assert_eq!(
        law(&w, eve),
        Some(json!({ "powers_here": ["law.arrest", "law.search", "law.judge"] }))
    );

    let cases: Vec<u64> =
        (0..5).map(|_| case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })))).collect();
    w.step(1);

    // The thief: five warrants, the newest three in the brief section.
    let brief = law(&w, cy).expect("the thief has a record");
    assert_eq!(
        brief["warrants"],
        json!([
            { "case": cases[2], "kind": "theft" },
            { "case": cases[3], "kind": "theft" },
            { "case": cases[4], "kind": "theft" },
        ])
    );
    assert_eq!(brief["more_warrants"], json!(2));
    // 5,000 cents each time: the limit of one theft.
    assert_eq!(brief["hot"], json!({ "money": 25_000, "items": {} }));
    assert!(brief.to_string().len() < 400, "{} bytes", brief.to_string().len());

    // The victim: five cases that it can report.
    let brief = law(&w, bo).expect("the victim knows its cases");
    assert_eq!(brief["known_cases"].as_array().map(Vec::len), Some(3));
    assert_eq!(
        brief["known_cases"][2],
        json!({ "case": cases[4], "kind": "theft", "role": "victim", "suspect": cy, "reported": false })
    );
    assert_eq!(brief["more_known_cases"], json!(2));
    assert!(brief.to_string().len() < 400, "{} bytes", brief.to_string().len());

    // Full detail shows every entry.
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let full = w.observe_with(cy, &opts).sections["law"].clone();
    assert_eq!(full["warrants"].as_array().map(Vec::len), Some(5));
    assert_eq!(full.get("more_warrants"), None);

    // Spent cash is no longer hot: the section follows the purse.
    w.with_resident(cy, |realm, _| {
        let sink = realm.defs().sink_id("destruction").expect("a sink");
        realm
            .ledger_mut()
            .burn(unsup_kernel::Holder::Resident(cy), sink, Money(24_000))
            .expect("cash");
    });
    assert_eq!(law(&w, cy).expect("a record")["hot"], json!({ "money": 1_000, "items": {} }));
    w.assert_conserved();
}

#[test]
fn the_module_works_alone_and_holds_a_suspect_in_place_without_a_jail() {
    // Only the kernel and this module: no map, no jail, no society. The
    // test writes the membership of the officer as the society module
    // would.
    static CRIME: CrimePlugin = CrimePlugin;
    let rules = with(&[&[ALWAYS, WATCH_SEES_ALL, ("crime.trial_ticks", 40)], &BLIND]);
    let mut w = TestWorld::with_config(&[&CRIME], config(3, &rules));
    w.step(2);
    let bo = w.spawn_at("Bo", Pos::new(10, 10));
    w.give_money(bo, Money(2_000));
    let cy = w.spawn_at("Cy", Pos::new(11, 10));
    let eve = w.spawn_at("Eve", Pos::new(10, 11));
    let city = OrgId::city(CityId(0));
    w.global().grant(eve, city, "police", &["law.arrest", "law.arrest@100"]);
    w.world.settle();
    w.step(1);

    // Without the scoped name of the city, a permission has no effect.
    w.global().grant(bo, city, "fake", &["law.arrest"]);
    w.world.settle();
    w.step(1);
    assert_eq!(fail(&mut w, bo, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);

    let result = ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    assert_eq!(result["cash"], json!(500));
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));
    let arrest = ok(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest["jail"], json!(null));
    // The suspect is held where it stands.
    assert!(w.has_condition(cy, "detained"));
    assert_eq!(w.pos(cy), Pos::new(11, 10));
    assert_eq!(law(&w, cy).expect("in custody")["custody"]["site"], json!(null));

    // The engine charges, calls no jury (nobody is free for it) and
    // decides by the evidence.
    let events = step_events(&mut w, 800);
    let verdict = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdict.len(), 1);
    assert_eq!(verdict[0].data["guilty"], json!(true));
    assert_eq!(w.cash(bo), Money(2_000));
    assert!(!w.has_condition(cy, "detained"));
    w.assert_conserved();
}
