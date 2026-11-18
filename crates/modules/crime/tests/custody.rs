//! Custody with more than one case, an arrest on the bare condition
//! `wanted`, a restitution to a city, and loot that cools down.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn a_suspect_with_two_cases_stays_under_arrest_until_the_last_verdict() {
    let rules = [ALWAYS, WATCH_SEES_ALL, ("crime.hold_ticks", 100), ("crime.trial_ticks", 400)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let bo = resident_at(&mut w, "Bo", 8_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let first = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    let second = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    assert_eq!(
        law(&w, cy).expect("wanted")["warrants"],
        json!([{ "case": first, "kind": "theft" }, { "case": second, "kind": "theft" }])
    );
    let (arrest, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest.result["cases"], json!([first, second]));
    let arrested = frame_of(&events, "crime.arrest_made");

    // The officer charges the first case only. The judge acquits.
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": first }));
    assert_eq!(case_view(&mut w, bo, second)["status"], json!("held"));
    let (_, mut events) =
        run_with_events(&mut w, finn, "rule_case", json!({ "case": first, "guilty": false }));
    events.extend(step_events(&mut w, 1));
    // The engine does not let the suspect go with a case that has enough
    // evidence: it charges the second case in the frame of the verdict.
    let charged = of_kind(&events, "crime.suspect_charged");
    assert_eq!(charged.len(), 1);
    assert_eq!(charged[0].data["case"], json!(second));
    assert_eq!(charged[0].data["officer"], json!(null));
    assert_eq!(charged[0].frame, frame_of(&events, "crime.verdict_given"));
    assert_eq!(of_kind(&events, "crime.suspect_released").len(), 0);
    assert!(w.has_condition(cy, "detained"));
    assert_eq!(law(&w, cy).expect("on trial")["trials"][0]["case"], json!(second));

    // The hold time passes during the trial: nothing happens.
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(arrested + 105 - now).expect("soon"));
    assert_eq!(of_kind(&events, "crime.suspect_released").len(), 0);
    assert!(w.has_condition(cy, "detained"));

    // The judge convicts in the second case: 240 ticks. Custody ends.
    ok(&mut w, finn, "rule_case", json!({ "case": second, "guilty": true, "fine": 0 }));
    w.step(1);
    assert!(!w.has_condition(cy, "detained"));
    assert!(w.has_condition(cy, "jailed"));
    assert_eq!(w.attr(cy, "rep.notoriety"), 20);
    // The restitution of the second case went back: 25 percent of 6,000.
    assert_eq!(w.cash(bo), Money(8_000 - 2_000));
    w.assert_conserved();
}

#[test]
fn the_condition_wanted_alone_is_grounds_and_the_engine_frees_a_suspect_without_a_case() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH, ("crime.hold_ticks", 30)], &BLIND]));
    let jail = site(&w, "jail");
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let cy = resident_at(&mut w, "Cy", 0, STREET);
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    // Another module marks the resident as wanted.
    w.set_condition(cy, "wanted", None);
    let (arrest, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest.result, json!({ "target": cy, "cases": [], "jail": jail.id }));
    assert_eq!(of_kind(&events, "crime.arrest_made")[0].data["case"], json!(null));
    assert!(w.has_condition(cy, "detained"));
    assert!(!w.has_condition(cy, "wanted"));
    // There is nothing to charge. After the hold time the suspect is free.
    let events = step_events(&mut w, 35);
    assert_eq!(of_kind(&events, "crime.suspect_charged").len(), 0);
    assert_eq!(
        of_kind(&events, "crime.suspect_released")[0].data,
        json!({ "target": cy, "reason": "hold_expired", "by": null })
    );
    assert!(!w.has_condition(cy, "detained"));
    w.assert_conserved();
}

#[test]
fn the_restitution_for_a_site_of_the_city_goes_to_the_treasury() {
    let rules = [
        ALWAYS,
        WATCH_SEES_ALL,
        ("crime.watch_arrest_bp", 10_000),
        ("crime.trial_ticks", 50),
        ("crime.burglary.fine", 0),
    ];
    let mut w = town(&with(&[&rules, &BLIND]));
    // The shop belongs to the city.
    let shop = site(&w, "shop");
    assert_eq!(shop.owner, Some(TREASURY));
    fill_till(&mut w, &shop, 6_000);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let case = case_of(&ok(&mut w, cy, "burgle", json!({})));
    assert_eq!(w.cash(cy), Money(3_000));
    assert_eq!(case_view(&mut w, cy, case)["victim"], json!(TREASURY));
    let treasury = w.money(TREASURY);

    // The thief goes to the shelter. The watch takes it, the engine tries
    // it, and the cash that it still has goes to the city.
    let shelter = site(&w, "shelter");
    w.set_pos(cy, shelter.rect.centre());
    let events = step_events(&mut w, 61 + 50 + 3);
    let verdict = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdict.len(), 1);
    assert_eq!(verdict[0].data["restitution"], json!(3_000));
    assert_eq!(verdict[0].data["fine"], json!(0));
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.money(TREASURY), treasury + Money(3_000));
    assert_eq!(law(&w, cy).expect("in jail").get("debts"), None);
    assert_eq!(law(&w, cy).expect("in jail").get("hot"), None);
    w.assert_conserved();
}

#[test]
fn loot_cools_down_and_a_search_then_finds_nothing() {
    let rules = [ALWAYS, NO_WATCH, ("crime.hot_ticks", 100)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let shop = site(&w, "shop");
    fill_till(&mut w, &shop, 6_000);
    fill_stock(&mut w, &shop, "bread", 4);
    let eve = resident_at(&mut w, "Eve", 0, STREET);
    appoint(&mut w, "police", &["law.search", "law.arrest"], &[eve]);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let crime = run(&mut w, cy, "burgle", json!({}));
    assert_eq!(
        law(&w, cy).expect("the loot is hot")["hot"],
        json!({ "money": 3_000, "items": { "bread": 4 } })
    );
    // 99 frames after the theft the loot is still hot. One more, and it
    // is cold.
    let now = w.world.frame();
    w.step(u32::try_from(crime.tick.0 + 99 - now).expect("soon"));
    assert!(law(&w, cy).is_some());
    w.step(1);
    assert_eq!(law(&w, cy), None);
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "bread" })), ErrorCode::Insufficient);

    w.set_pos(eve, shop.rect.centre().offset(1, 0));
    let found = ok(&mut w, eve, "search", json!({ "target": cy }));
    assert_eq!(found["found"], json!(false));
    assert_eq!(w.cash(cy), Money(3_000));
    assert_eq!(w.qty(Holder::Resident(cy), "bread"), 4);
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    w.assert_conserved();
}

#[test]
fn the_docket_shows_the_newest_cases_and_counts_the_rest() {
    let rules = [ALWAYS, NO_WATCH, ("crime.case_list_max", 3)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let bo = resident_at(&mut w, "Bo", 50_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let cases: Vec<u64> =
        (0..5).map(|_| case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })))).collect();
    w.step(1);
    let docket = ok(&mut w, bo, "case_list", json!({}));
    let shown: Vec<u64> = docket["cases"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["case"].as_u64().expect("a case"))
        .collect();
    assert_eq!(shown, [cases[4], cases[3], cases[2]]);
    assert_eq!(docket["more"], json!(2));
    assert_eq!(docket["city"], json!(0));
    let entry = &docket["cases"][0];
    assert_eq!(entry["kind"], json!("theft"));
    assert_eq!(entry["status"], json!("open"));
    assert_eq!(entry["pos"], json!(STREET));
    assert_eq!(entry["evidence"], json!(0));
    assert_eq!(entry.get("suspect"), None);
    w.assert_conserved();
}
