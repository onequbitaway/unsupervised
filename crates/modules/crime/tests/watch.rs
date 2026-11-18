//! The city watch: a world with no police and no judge still finds,
//! arrests, tries and punishes a criminal. The engine does it all.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn the_watch_arrests_a_wanted_resident_in_a_public_building_and_the_engine_tries_it() {
    let rules = with(&[&[ALWAYS, WATCH_SEES_ALL, ("crime.watch_arrest_bp", 10_000)], &BLIND]);
    let mut w = town(&rules);
    let jail = site(&w, "jail");
    let shelter = site(&w, "shelter");
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let treasury = w.money(TREASURY);

    // The watch sees the theft: 60 evidence points and a warrant.
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));
    let view = case_view(&mut w, bo, case);
    assert_eq!(
        view["evidence_by_source"],
        json!({
            "testimony": 0, "investigation": 0, "search": 0, "caught": 0, "watch": 60,
        })
    );
    assert_eq!(
        law(&w, cy).expect("wanted")["warrants"],
        json!([{ "case": case, "kind": "theft" }])
    );

    // In the street the watch does not find the thief, hour after hour.
    let events = step_events(&mut w, 200);
    assert_eq!(of_kind(&events, "crime.arrest_made").len(), 0);
    assert!(w.has_condition(cy, "wanted"));
    assert!(!w.has_condition(cy, "detained"));

    // The thief goes to sleep in the shelter, a public building. The next
    // round of the watch, at most one game hour later, takes it.
    w.set_pos(cy, shelter.rect.centre());
    let events = step_events(&mut w, 61);
    let arrests = of_kind(&events, "crime.arrest_made");
    assert_eq!(arrests.len(), 1);
    assert_eq!(arrests[0].actor, None);
    assert_eq!(
        arrests[0].data,
        json!({
            "officer": null, "target": cy, "case": case, "cases": [case], "city": 0,
            "watch": true,
        })
    );
    assert_eq!(site_kind_of(&w, cy).as_deref(), Some("jail"));
    assert_eq!(w.pos(cy), jail.rect.centre());
    assert_eq!(
        notice(&w, cy, "crime.arrested").expect("the suspect is told").data,
        json!({ "officer": null, "cases": [case], "jail": jail.id })
    );
    // The engine charged in the frame of the arrest. No officer did.
    let charged = of_kind(&events, "crime.suspect_charged");
    assert_eq!(charged.len(), 1);
    assert_eq!(charged[0].frame, arrests[0].frame);
    assert_eq!(charged[0].data["officer"], json!(null));
    assert_eq!(charged[0].data["decided_in"], json!(720));

    // Nobody judges and nobody votes. At the deadline the evidence
    // decides: 60 points reach the threshold of 60.
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(charged[0].frame + 720 - now).expect("soon"));
    let verdicts = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].frame, charged[0].frame + 720);
    assert_eq!(
        verdicts[0].data,
        json!({
            "case": case, "kind": "theft", "city": 0, "target": cy, "guilty": true,
            "by": "evidence", "judge": null, "fine": 3_000, "jail_ticks": 240,
            "restitution": 500, "votes_guilty": 0, "votes_not_guilty": 0,
        })
    );
    w.step(2);

    // The engine carries out the sentence: the stolen 500 go back to the
    // victim, the thief is in jail for 240 ticks and owes the fine.
    let verdict = notice(&w, cy, "crime.verdict").expect("the convict gets the verdict");
    assert_eq!(verdict.data["paid"], json!(500));
    assert_eq!(verdict.data["owed"], json!(3_000));
    assert_eq!(jailed_until(&w, cy), Some(verdict.tick.0 + 240));
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.attr(cy, "rep.notoriety"), 20);
    assert_eq!(w.attr(cy, "rep.standing"), -60);
    assert!(!w.has_condition(cy, "detained"));

    // The fine is not paid in one day: 300 more ticks, after the first
    // 240. The treasury got nothing.
    let events = step_events(&mut w, 1_440);
    let defaults = of_kind(&events, "crime.fine_defaulted");
    assert_eq!(defaults.len(), 1);
    assert_eq!(
        defaults[0].data,
        json!({ "resident": cy, "case": case, "owed": 3_000, "jail_ticks": 300 })
    );
    assert_eq!(w.money(TREASURY), treasury);
    assert!(w.is_alive(cy));
    w.assert_conserved();
}

#[test]
fn a_law_can_make_the_watch_blind_or_sharp() {
    // The watch sees nothing in this world: no theft gets a warrant.
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 100_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    for _ in 0..5 {
        ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    }
    w.step(2);
    assert!(!w.has_condition(cy, "wanted"));
    let docket = ok(&mut w, bo, "case_list", json!({}));
    assert_eq!(docket["cases"].as_array().map(Vec::len), Some(5));
    assert_eq!(ok(&mut w, bo, "case_list", json!({ "status": "wanted" }))["cases"], json!([]));

    // A law changes the policy parameter. Zones see it in the next frame.
    w.set_param("crime.watch_detect_bp", 10_000);
    w.step(1);
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));
    let wanted = ok(&mut w, bo, "case_list", json!({ "status": "wanted" }));
    assert_eq!(wanted["cases"].as_array().map(Vec::len), Some(1));
    assert_eq!(wanted["cases"][0]["case"], json!(case));
    assert_eq!(wanted["cases"][0]["suspect"], json!(cy));
    assert_eq!(wanted["more"], json!(0));
    assert_eq!(
        fail(&mut w, bo, "case_list", json!({ "status": "unsolved" })),
        ErrorCode::InvalidArgs
    );
    w.assert_conserved();
}

#[test]
fn the_watch_is_weak_by_default() {
    // The default chance to see a crime is 10 percent. Of 100 thefts the
    // watch sees some and misses most.
    let mut w = town(&with(&[&[ALWAYS], &BLIND]));
    assert_eq!(w.param("crime.watch_detect_bp"), 1_000);
    assert_eq!(w.param("crime.watch_arrest_bp"), 2_500);
    let bo = resident_at(&mut w, "Bo", 10_000_000, STREET);
    let mut seen = 0;
    for i in 0..100 {
        let thief = resident_at(&mut w, "Thief", 0, STREET.offset(1, 0));
        ok(&mut w, thief, "pickpocket", json!({ "target": bo }));
        w.step(1);
        if w.has_condition(thief, "wanted") {
            seen += 1;
        }
        // The thief leaves, so that the next one stands alone.
        w.set_pos(thief, Pos::new(70 + i % 50, 70 + i / 50));
    }
    assert!((2..=25).contains(&seen), "the watch saw {seen} of 100 thefts");
    w.assert_conserved();
}

#[test]
fn the_watch_finds_contraband_in_a_public_building() {
    let rules = with(&[
        &[ALWAYS, NO_WATCH, ("crime.watch_contraband_bp", 10_000), ("crime.watch_arrest_bp", 0)],
        &BLIND,
    ]);
    let mut w = town(&rules);
    let station = site(&w, "police_station");
    let shelter = site(&w, "shelter");
    // In the street nobody looks.
    let cy = resident_at(&mut w, "Cy", 0, STREET);
    w.give_item(cy, "stims", 4);
    let events = step_events(&mut w, 120);
    assert_eq!(of_kind(&events, "crime.contraband_seized").len(), 0);
    assert_eq!(w.qty(Holder::Resident(cy), "stims"), 4);

    w.set_pos(cy, shelter.rect.centre());
    let events = step_events(&mut w, 61);
    let seized = of_kind(&events, "crime.contraband_seized");
    assert_eq!(seized.len(), 1);
    assert_eq!(seized[0].actor, None);
    let case = seized[0].data["case"].as_u64().expect("a case");
    assert_eq!(seized[0].data, json!({ "case": case, "city": 0, "value": 16_000 }));
    assert_eq!(w.qty(Holder::Resident(cy), "stims"), 0);
    w.step(1);
    assert_eq!(w.qty(Holder::Site(station.id), "stims"), 4);
    assert!(w.has_condition(cy, "wanted"));
    let told = notice(&w, cy, "crime.searched").expect("the carrier is told");
    assert_eq!(told.from, None);
    assert_eq!(told.data["contraband"], json!({ "stims": 4 }));
    let view = case_view(&mut w, cy, case);
    assert_eq!(view["kind"], json!("smuggling"));
    assert_eq!(view["evidence_by_source"]["watch"], json!(60));
    w.assert_conserved();
}
