//! The life of a case in the registry: it goes cold, it becomes moot, it
//! leaves the registry, and a public event never names a suspect before
//! an arrest does.

mod common;

use common::*;
use serde_json::json;
use unsup_crime::{Cases, Status};
use unsup_kernel::{ErrorCode, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn a_case_without_a_charge_goes_cold_and_its_warrant_ends() {
    let rules =
        [ALWAYS, WATCH_SEES_ALL, ("crime.case_ttl_ticks", 100), ("crime.case_keep_ticks", 50)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let crime = run(&mut w, cy, "pickpocket", json!({ "target": bo }));
    let case = case_of(&crime.result);
    let opened = w.global().res::<Cases>().get(case).expect("the case").opened;
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));

    // 100 frames after the crime the case is cold.
    let now = w.world.frame();
    w.step(u32::try_from(opened + 99 - now).expect("soon"));
    assert_eq!(w.global().res::<Cases>().get(case).map(|c| c.status), Some(Status::Wanted));
    w.step(1);
    {
        let cases = w.global().res::<Cases>();
        let cold = cases.get(case).expect("the case is still in the registry");
        assert_eq!((cold.status, cold.closed), (Status::Cold, Some(opened + 100)));
        // A cold case does not name its suspect.
        assert_eq!(cold.suspect(), None);
    }
    w.step(1);
    assert!(!w.has_condition(cy, "wanted"));
    assert_eq!(
        notice(&w, cy, "crime.wanted").expect("the suspect is told").data,
        json!({ "case": case, "kind": "theft", "wanted": false })
    );
    assert_eq!(law(&w, cy).expect("the loot is still hot").get("warrants"), None);
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["status"], json!("cold"));
    assert_eq!(view.get("suspect"), None);
    assert_eq!(ok(&mut w, bo, "case_list", json!({}))["cases"], json!([]));
    assert_eq!(
        ok(&mut w, bo, "case_list", json!({ "status": "cold" }))["cases"][0]["case"],
        json!(case)
    );

    // 50 frames later the case leaves the registry.
    w.step(50);
    assert!(w.global().res::<Cases>().is_empty());
    assert_eq!(fail(&mut w, bo, "case_view", json!({ "case": case })), ErrorCode::NotFound);
    w.assert_conserved();
}

#[test]
fn the_cases_of_a_resident_that_leaves_the_world_are_moot() {
    let rules = [ALWAYS, WATCH_SEES_ALL, ("crime.jury_size", 1)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = stranger_at(&mut w, "Cy", 2, 0, STREET.offset(1, 0));
    let juror = resident_at(&mut w, "Juror", 0, Pos::new(70, 70));
    let first = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    let second = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": first }));
    w.step(1);
    assert_eq!(law(&w, juror).expect("a duty")["jury_duty"][0]["case"], json!(first));
    assert!(w.global().res::<Cases>().in_custody(cy));

    // The defendant leaves the world in the middle of its trial.
    w.despawn(cy);
    let events = step_events(&mut w, 2);
    {
        let cases = w.global().res::<Cases>();
        assert_eq!(cases.get(first).map(|c| c.status), Some(Status::Moot));
        assert_eq!(cases.get(second).map(|c| c.status), Some(Status::Moot));
        assert!(!cases.in_custody(cy));
    }
    // No verdict comes, now or at the deadline, and the juror is free.
    assert_eq!(of_kind(&events, "crime.verdict_given").len(), 0);
    assert_eq!(law(&w, juror), None);
    assert_eq!(
        fail(&mut w, juror, "jury_vote", json!({ "case": first, "guilty": true })),
        ErrorCode::Conflict
    );
    let events = step_events(&mut w, 800);
    assert_eq!(of_kind(&events, "crime.verdict_given").len(), 0);
    w.assert_conserved();
}

#[test]
fn residents_of_the_owner_of_the_defendant_do_not_sit_on_the_jury() {
    let rules = [ALWAYS, WATCH_SEES_ALL, ("crime.jury_size", 2)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    // The thief has owner 2. So do four friends. One resident has owner 3.
    let cy = stranger_at(&mut w, "Cy", 2, 0, STREET.offset(1, 0));
    let friends: Vec<ResidentId> =
        (0..4).map(|i| stranger_at(&mut w, "Friend", 2, 0, Pos::new(70 + i, 70))).collect();
    let fair = stranger_at(&mut w, "Fair", 3, 0, Pos::new(80, 70));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    let (_, events) = run_with_events(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    // Two seats, and only one resident that can take one.
    assert_eq!(of_kind(&events, "crime.jury_summoned")[0].data["jurors"], json!(1));
    assert_eq!(law(&w, fair).expect("a duty")["jury_duty"][0]["case"], json!(case));
    for friend in friends {
        assert_eq!(law(&w, friend), None);
    }
    w.assert_conserved();
}

#[test]
fn no_public_event_names_the_thief_before_the_arrest() {
    // Every witness recognizes the thief, the watch sees the crime, and
    // the victim reports it. Viewers still do not learn the name.
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &SHARP]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 9));
    appoint(&mut w, "police", &["law.arrest", "law.search"], &[eve]);
    let shop = site(&w, "shop");
    fill_till(&mut w, &shop, 4_000);
    let bo = resident_in(&mut w, "Bo", 2_000, &shop);
    w.step(1);
    // The thief gets a high id that no other number of an event has.
    for i in 0..40 {
        resident_at(&mut w, "Crowd", 0, Pos::new(70 + i, 80));
    }
    let cy = resident_at(&mut w, "Cy", 0, shop.rect.centre().offset(1, 0));
    assert_eq!(cy.0, 43);
    w.step(1);

    let mut events = Vec::new();
    for (verb, args) in [
        ("burgle", json!({})),
        ("vandalize", json!({})),
        ("pickpocket", json!({ "target": bo })),
        ("rob", json!({ "target": bo })),
        ("assault", json!({ "target": bo })),
    ] {
        let (report, more) = run_with_events(&mut w, cy, verb, args);
        assert_eq!(report.result["success"], json!(true), "{verb}");
        events.extend(more);
    }
    for known in
        law(&w, bo).expect("the victim knows the cases")["known_cases"].as_array().expect("a list")
    {
        assert_eq!(known["suspect"], json!(cy));
        w.act_ok(bo, "report_crime", json!({ "case": known["case"] }));
    }
    events.extend(step_events(&mut w, 3));
    assert!(w.has_condition(cy, "wanted"));
    w.set_pos(eve, shop.rect.centre().offset(2, 0));
    let (_, more) = run_with_events(&mut w, eve, "search", json!({ "target": cy }));
    events.extend(more);

    let crime: Vec<_> = events.iter().filter(|event| event.kind.starts_with("crime.")).collect();
    let kinds: Vec<&str> = crime.iter().map(|event| event.kind.as_str()).collect();
    for kind in [
        "crime.theft_committed",
        "crime.vandalism_committed",
        "crime.assault_committed",
        "crime.warrant_issued",
        "crime.search_made",
    ] {
        assert!(kinds.contains(&kind), "no {kind} in {kinds:?}");
    }
    for event in &crime {
        assert_ne!(event.actor, Some(cy), "{} has the thief as its actor", event.kind);
        let text = event.data.to_string();
        assert!(!text.contains("43"), "{} names the thief: {text}", event.kind);
    }

    // The arrest names the target. From then on the name is public.
    let (_, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(of_kind(&events, "crime.arrest_made")[0].data["target"], json!(43));
    w.assert_conserved();
}

#[test]
fn events_have_the_drama_scores_of_the_data_file() {
    let rules =
        [ALWAYS, WATCH_SEES_ALL, ("crime.watch_arrest_bp", 10_000), ("crime.trial_ticks", 20)];
    let mut w = town(&with(&[&rules, &BLIND]));
    let shop = site(&w, "shop");
    let shelter = site(&w, "shelter");
    fill_till(&mut w, &shop, 200_000);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let drama = |events: &[unsup_kernel::protocol::Event], kind: &str| -> Vec<u8> {
        of_kind(events, kind).iter().map(|event| event.drama).collect()
    };

    // A theft: 45. A burglary of 100,000 cents is a heist: 75.
    let (_, mut events) = run_with_events(&mut w, cy, "pickpocket", json!({ "target": bo }));
    w.set_pos(cy, shop.rect.centre());
    events.extend(run_with_events(&mut w, cy, "burgle", json!({})).1);
    assert_eq!(drama(&events, "crime.theft_committed"), [45, 75]);

    // The watch arrests in the shelter, the engine charges and decides.
    w.set_pos(cy, shelter.rect.centre());
    events.extend(step_events(&mut w, 61 + 20 + 3));
    assert_eq!(drama(&events, "crime.warrant_issued"), [40, 40]);
    assert_eq!(drama(&events, "crime.arrest_made"), [55]);
    assert_eq!(drama(&events, "crime.suspect_charged"), [45, 45]);
    assert_eq!(drama(&events, "crime.jury_summoned"), [35, 35]);
    assert_eq!(drama(&events, "crime.verdict_given"), [65, 65]);
    w.assert_conserved();

    // A crime that failed: 30.
    let mut w = town(&with(&[&[NEVER, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let (_, events) = run_with_events(&mut w, cy, "assault", json!({ "target": bo }));
    assert_eq!(drama(&events, "crime.assault_committed"), [30]);
    w.assert_conserved();
}
