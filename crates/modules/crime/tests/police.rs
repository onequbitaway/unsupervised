//! The verbs of the police: `patrol`, `investigate`, `search`, `arrest`
//! and `release`, with each refusal.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Holder, Money, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// Makes Eve an officer of the city with the two police powers.
fn officer(w: &mut TestWorld<'_>, pos: Pos) -> ResidentId {
    let eve = resident_at(w, "Eve", 0, pos);
    appoint(w, "police", &["law.arrest", "law.search"], &[eve]);
    eve
}

#[test]
fn an_officer_on_patrol_catches_a_thief_in_the_act() {
    let rules = with(&[&[ALWAYS, NO_WATCH], &BLIND, &[("crime.patrol_detect_bp", 10_000)]]);
    let mut w = town(&rules);
    let eve = officer(&mut w, STREET.offset(0, 9));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));

    w.act_ok(eve, "patrol", json!({ "ticks": 30 }));
    let theft = ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    let case = case_of(&theft);
    // The officer is nine tiles away: too far for a bystander, near enough
    // for a patrol. The victim and the officer noticed the theft.
    assert_eq!(theft["witnesses"], json!(2));
    let alert = notice(&w, eve, "crime.patrol_alert").expect("the patrol sees the theft");
    assert_eq!(
        alert.data,
        json!({
            "case": case, "kind": "theft", "role": "witness", "pos": STREET, "suspect": cy,
            "can_arrest": true,
        })
    );
    // A crime that an officer saw has 100 evidence points: a warrant.
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));
    let view = case_view(&mut w, eve, case);
    assert_eq!(view["evidence_by_source"]["caught"], json!(100));
    assert_eq!(view["suspect"], json!(cy));

    // The patrol ends. Its result names the wanted resident nearby.
    w.run_until_idle(eve, 40);
    let patrol = w.reports(eve).into_iter().find(|r| r.verb == "patrol").expect("the patrol ended");
    assert_eq!(patrol.result, json!({ "ticks": 30, "wanted": [cy], "more_wanted": 0 }));
    // Six experience points for each hour.
    assert_eq!(w.attr(eve, "skill.investigation"), 3);

    w.set_pos(eve, STREET.offset(2, 0));
    let arrest = ok(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest["cases"], json!([case]));
    assert!(w.has_condition(cy, "detained"));
    assert_eq!(w.attr(eve, "skill.law"), 8);
    w.assert_conserved();
}

#[test]
fn an_officer_that_saw_the_crime_can_arrest_without_a_warrant_for_a_time() {
    // The sight of an officer gives no evidence here, so no warrant comes.
    let rules = with(&[
        &[ALWAYS, NO_WATCH],
        &SHARP,
        &[("crime.evidence_caught", 0), ("crime.caught_ticks", 40), ("crime.hold_ticks", 50)],
    ]);
    let mut w = town(&rules);
    let eve = officer(&mut w, STREET.offset(2, 0));
    let gus = resident_at(&mut w, "Gus", 0, STREET.offset(2, 1));
    appoint(&mut w, "sheriff", &["law.arrest"], &[gus]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(1, 1));

    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    let saw = notice(&w, eve, "crime.witnessed").expect("the officer saw the theft");
    assert_eq!(saw.data["can_arrest"], json!(true));
    assert_eq!(notice(&w, dee, "crime.witnessed").expect("a witness").data.get("can_arrest"), None);
    w.step(2);
    assert!(!w.has_condition(cy, "wanted"));

    // Dee did nothing: no grounds. Cy did, and Eve saw it.
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": dee })), ErrorCode::NotAllowed);
    let (arrest, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest.result["cases"], json!([case]));
    assert_eq!(of_kind(&events, "crime.arrest_made")[0].data["case"], json!(case));
    assert!(w.has_condition(cy, "detained"));
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotHere);

    // The case has no evidence, so nobody can charge. After the hold time
    // the engine frees the suspect.
    assert_eq!(
        fail(&mut w, eve, "charge", json!({ "target": cy, "case": case })),
        ErrorCode::Insufficient
    );
    let events = step_events(&mut w, 60);
    assert!(!w.has_condition(cy, "detained"));
    let released = of_kind(&events, "crime.suspect_released");
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].data, json!({ "target": cy, "reason": "hold_expired", "by": null }));
    assert_eq!(
        notice(&w, cy, "crime.released").expect("the suspect is told").data,
        json!({ "reason": "hold_expired", "free": true })
    );

    // Gus saw the theft too, but too long ago.
    let beside = w.pos(cy).offset(1, 0);
    w.set_pos(gus, beside);
    assert_eq!(fail(&mut w, gus, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    w.assert_conserved();
}

#[test]
fn arrest_refuses_bad_requests() {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND]));
    // Eve stands next to Bo and next to Cy.
    let eve = officer(&mut w, STREET.offset(0, 1));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    w.step(1);
    assert!(w.has_condition(cy, "wanted"));

    // Bo is no officer. A warrant does not change that.
    assert_eq!(fail(&mut w, bo, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": eve })), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, eve, "arrest", json!({})), ErrorCode::InvalidArgs);
    // Bo is not wanted.
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": bo })), ErrorCode::NotAllowed);
    // Cy is wanted, but two tiles are too far.
    w.set_pos(eve, STREET.offset(3, 0));
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotHere);
    assert!(!w.has_condition(cy, "detained"));

    w.set_pos(eve, STREET.offset(2, 0));
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    // A second officer cannot arrest a resident that is in custody.
    let cell = w.pos(cy).offset(0, 1);
    let gus = resident_at(&mut w, "Gus", 0, cell);
    appoint(&mut w, "sheriff", &["law.arrest"], &[gus]);
    assert_eq!(fail(&mut w, gus, "arrest", json!({ "target": cy })), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn investigate_reads_a_trace_one_time_and_old_traces_give_less() {
    let rules = with(&[&[ALWAYS, NO_WATCH], &BLIND, &[("crime.trace_ticks", 200)]]);
    let mut w = town(&rules);
    let shop = site(&w, "shop");
    let eve = officer(&mut w, STREET);
    let gus = resident_at(&mut w, "Gus", 0, shop.rect.centre().offset(1, 1));
    appoint(&mut w, "detective", &["law.search"], &[gus]);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    w.set_pos(gus, STREET);
    let crime = run(&mut w, cy, "burgle", json!({}));
    let case = case_of(&crime.result);

    // Bo is no officer.
    let bo = resident_in(&mut w, "Bo", 0, &shop);
    assert_eq!(fail(&mut w, bo, "investigate", json!({})), ErrorCode::NotAllowed);
    // Eve is too far from the scene.
    assert_eq!(fail(&mut w, eve, "investigate", json!({})), ErrorCode::NotFound);
    w.set_pos(eve, shop.rect.centre().offset(2, 0));
    assert_eq!(fail(&mut w, eve, "investigate", json!({ "case": case + 1 })), ErrorCode::NotFound);

    let first = run(&mut w, eve, "investigate", json!({}));
    let age = i64::try_from(first.tick.0 - crime.tick.0).expect("a small number");
    let points = 30 * (200 - age) / 200;
    assert_eq!(
        first.result,
        json!({ "case": case, "kind": "burglary", "found": true, "evidence": points })
    );
    // The same officer reads a trace one time.
    assert_eq!(fail(&mut w, eve, "investigate", json!({})), ErrorCode::NotFound);

    // A second officer reads it later and finds less.
    w.step(60);
    w.set_pos(gus, shop.rect.centre().offset(0, 2));
    let second = run(&mut w, gus, "investigate", json!({ "case": case }));
    let age = i64::try_from(second.tick.0 - crime.tick.0).expect("a small number");
    let later = 30 * (200 - age) / 200;
    assert!(later < points && later > 0, "{later} and {points}");
    assert_eq!(second.result["evidence"], json!(later));
    w.step(1);
    let view = case_view(&mut w, eve, case);
    assert_eq!(view["evidence_by_source"]["investigation"], json!(points + later));

    // After 200 ticks the trace is gone.
    let hal = resident_at(&mut w, "Hal", 0, shop.rect.centre().offset(0, 1));
    appoint(&mut w, "inspector", &["law.search"], &[hal]);
    w.step(200);
    assert_eq!(fail(&mut w, hal, "investigate", json!({})), ErrorCode::NotFound);
    w.assert_conserved();
}

#[test]
fn an_investigation_can_fail_and_the_officer_can_try_again() {
    // The burglary works by its own bonus. The investigation has no
    // chance.
    let rules = with(&[
        &[
            ("crime.chance_min_bp", 0),
            ("crime.burgle_base_bp", 10_000),
            ("crime.investigate_base_bp", 0),
            ("crime.investigate_level_bp", 0),
            NO_WATCH,
        ],
        &BLIND,
    ]);
    let mut w = town(&rules);
    let shop = site(&w, "shop");
    let eve = officer(&mut w, shop.rect.centre().offset(1, 0));
    w.set_pos(eve, STREET);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let case = case_of(&ok(&mut w, cy, "burgle", json!({})));
    w.set_pos(eve, shop.rect.centre().offset(1, 0));
    for tries in 1..=2 {
        let result = ok(&mut w, eve, "investigate", json!({}));
        assert_eq!(result, json!({ "case": case, "kind": "burglary", "found": false }));
        // Half of the experience for each try that failed.
        assert_eq!(w.attr(eve, "skill.investigation"), 5 * tries);
    }
    w.step(1);
    assert_eq!(case_view(&mut w, eve, case)["evidence"], json!(0));
    w.assert_conserved();
}

#[test]
fn the_investigations_of_one_case_have_a_limit() {
    let rules = with(&[&[ALWAYS, NO_WATCH], &BLIND, &[("crime.evidence_investigate", 45)]]);
    let mut w = town(&rules);
    let shop = site(&w, "shop");
    let eve = officer(&mut w, shop.rect.centre().offset(1, 0));
    let gus = resident_at(&mut w, "Gus", 0, shop.rect.centre().offset(1, 1));
    appoint(&mut w, "detective", &["law.search"], &[gus]);
    w.set_pos(eve, STREET);
    w.set_pos(gus, STREET);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let case = case_of(&ok(&mut w, cy, "burgle", json!({})));
    w.set_pos(eve, shop.rect.centre().offset(1, 0));
    w.set_pos(gus, shop.rect.centre().offset(1, 1));
    assert_eq!(ok(&mut w, eve, "investigate", json!({}))["evidence"], json!(44));
    assert_eq!(ok(&mut w, gus, "investigate", json!({}))["evidence"], json!(44));
    w.step(1);
    // 44 + 44 is above the limit of 60.
    let view = case_view(&mut w, eve, case);
    assert_eq!(view["evidence_by_source"]["investigation"], json!(60));
    assert_eq!(view["status"], json!("wanted"));
    w.assert_conserved();
}

#[test]
fn a_search_takes_contraband_and_opens_a_case() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let station = site(&w, "police_station");
    let evidence_room = Holder::Site(station.id);
    // The home is in another district than the police station.
    let home = site(&w, "home");
    assert_ne!(zone_at(&w, home.rect.centre()), zone_at(&w, station.rect.centre()));
    let eve = officer(&mut w, home.rect.centre());
    let cy = resident_at(&mut w, "Cy", 900, home.rect.centre().offset(1, 0));
    w.give_item(cy, "moonshine", 3);
    w.give_item(cy, "bread", 2);

    let (report, events) = run_with_events(&mut w, eve, "search", json!({ "target": cy }));
    let case = report.result["smuggling_case"].as_u64().expect("a case");
    assert_eq!(
        report.result,
        json!({
            "target": cy, "found": true, "money": 0, "goods": {},
            "contraband": { "moonshine": 3 }, "cases": [], "smuggling_case": case,
        })
    );
    // Clean cash and clean goods stay.
    assert_eq!(w.cash(cy), Money(900));
    let carrier = Holder::Resident(cy);
    assert_eq!((w.qty(carrier, "moonshine"), w.qty(carrier, "bread")), (0, 2));
    assert_eq!(w.qty(evidence_room, "moonshine"), 3);
    assert_eq!(w.attr(eve, "skill.investigation"), 4);

    let seized = of_kind(&events, "crime.contraband_seized");
    assert_eq!(seized.len(), 1);
    assert_eq!(seized[0].actor, None);
    assert_eq!(seized[0].data, json!({ "case": case, "city": 0, "value": 6_000 }));
    // The event of the search names the officer and not the carrier.
    let searches = of_kind(&events, "crime.search_made");
    assert_eq!(searches.len(), 1);
    assert_eq!(searches[0].actor, Some(eve));
    assert_eq!(searches[0].data, json!({ "officer": eve, "city": 0, "found": true }));
    let told = notice(&w, cy, "crime.searched").expect("the carrier is told");
    assert_eq!(told.from, Some(eve));
    assert_eq!(told.data, report.result);

    // The officer found it: grounds for an arrest, and a warrant.
    let view = case_view(&mut w, eve, case);
    assert_eq!((&view["kind"], &view["status"]), (&json!("smuggling"), &json!("wanted")));
    assert_eq!(view["evidence_by_source"]["caught"], json!(100));
    assert!(w.has_condition(cy, "wanted"));
    let arrest = ok(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest["cases"], json!([case]));
    w.assert_conserved();
}

#[test]
fn contraband_is_legal_when_the_policy_says_so() {
    let rules = with(&[&[ALWAYS, NO_WATCH], &BLIND, &[("crime.contraband_illegal", 0)]]);
    let mut w = town(&rules);
    let eve = officer(&mut w, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    w.give_item(cy, "stims", 2);
    let (report, events) = run_with_events(&mut w, eve, "search", json!({ "target": cy }));
    assert_eq!(
        report.result,
        json!({
            "target": cy, "found": false, "money": 0, "goods": {}, "contraband": {},
            "cases": [],
        })
    );
    assert_eq!(w.qty(Holder::Resident(cy), "stims"), 2);
    assert_eq!(of_kind(&events, "crime.contraband_seized").len(), 0);
    assert_eq!(of_kind(&events, "crime.search_made")[0].data["found"], json!(false));
    assert!(!w.has_condition(cy, "wanted"));
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    w.assert_conserved();
}

#[test]
fn search_refuses_bad_requests() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let eve = officer(&mut w, STREET);
    let gus = resident_at(&mut w, "Gus", 0, STREET.offset(0, 1));
    appoint(&mut w, "sheriff", &["law.arrest"], &[gus]);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let far = resident_at(&mut w, "Far", 0, STREET.offset(4, 0));
    w.give_item(cy, "stims", 2);
    // `law.arrest` is not `law.search`, and Cy has no power at all.
    assert_eq!(fail(&mut w, gus, "search", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, cy, "search", json!({ "target": gus })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, eve, "search", json!({ "target": eve })), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, eve, "search", json!({ "target": far })), ErrorCode::NotHere);
    assert_eq!(fail(&mut w, eve, "search", json!({ "target": 77 })), ErrorCode::NotHere);
    assert_eq!(w.qty(Holder::Resident(cy), "stims"), 2);
    // A patrol needs a power too, and has a greatest length.
    assert_eq!(fail(&mut w, cy, "patrol", json!({})), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, eve, "patrol", json!({ "ticks": 0 })), ErrorCode::InvalidArgs);
    assert_eq!(fail(&mut w, eve, "patrol", json!({ "ticks": 241 })), ErrorCode::InvalidArgs);
    w.assert_conserved();
}

#[test]
fn an_officer_can_release_a_suspect_and_a_warden_an_inmate() {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND]));
    let jail = site(&w, "jail");
    let eve = officer(&mut w, STREET.offset(2, 0));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);

    // Nobody is in custody yet.
    assert_eq!(fail(&mut w, eve, "release", json!({ "target": cy })), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, bo, "release", json!({ "target": cy })), ErrorCode::NotAllowed);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(site_kind_of(&w, cy).as_deref(), Some("jail"));
    assert_eq!(fail(&mut w, eve, "release", json!({ "target": cy })), ErrorCode::NotHere);

    // The officer goes to the jail and lets the suspect go.
    w.set_pos(eve, jail.rect.centre().offset(0, 1));
    let (release, events) = run_with_events(&mut w, eve, "release", json!({ "target": cy }));
    assert_eq!(release.result, json!({ "target": cy, "released": true }));
    assert!(!w.has_condition(cy, "detained"));
    let released = of_kind(&events, "crime.suspect_released");
    assert_eq!(released.len(), 1);
    assert_eq!(released[0].data, json!({ "target": cy, "reason": "released", "by": eve }));
    // The warrant is open again.
    w.step(2);
    assert!(w.has_condition(cy, "wanted"));
    assert_eq!(case_view(&mut w, eve, case)["status"], json!("wanted"));

    // Arrest and charge. Now only the court ends the custody.
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    w.step(1);
    assert_eq!(fail(&mut w, eve, "release", json!({ "target": cy })), ErrorCode::Conflict);

    // The trial ends by the evidence of the watch: 240 ticks of jail.
    w.step(720);
    assert!(w.has_condition(cy, "jailed"));
    assert!(!w.has_condition(cy, "detained"));
    // `law.arrest` does not end a sentence. `law.jail` does.
    assert_eq!(fail(&mut w, eve, "release", json!({ "target": cy })), ErrorCode::NotAllowed);
    let gus = resident_at(&mut w, "Gus", 0, jail.rect.centre().offset(0, -1));
    appoint(&mut w, "warden", &["law.jail"], &[gus]);
    let (release, events) = run_with_events(&mut w, gus, "release", json!({ "target": cy }));
    assert!(release.ok);
    assert!(!w.has_condition(cy, "jailed"));
    assert_eq!(
        of_kind(&events, "crime.suspect_released")[0].data,
        json!({ "target": cy, "reason": "paroled", "by": gus })
    );
    // An early release is not the end of a sentence.
    assert_eq!(of_kind(&events, "crime.sentence_served").len(), 0);
    w.assert_conserved();
}

#[test]
fn an_officer_with_only_the_search_power_is_evidence_but_cannot_arrest() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &SHARP]));
    let gus = resident_at(&mut w, "Gus", 0, STREET.offset(0, 2));
    appoint(&mut w, "detective", &["law.search"], &[gus]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    // The detective saw who did it. That is the evidence of an officer.
    let saw = notice(&w, gus, "crime.witnessed").expect("the detective saw the theft");
    assert_eq!(saw.data["suspect"], json!(cy));
    assert_eq!(saw.data.get("can_arrest"), None);
    w.step(1);
    assert_eq!(case_view(&mut w, gus, case)["evidence_by_source"]["caught"], json!(100));
    assert!(w.has_condition(cy, "wanted"));
    // It has no power to arrest, with or without a warrant.
    w.set_pos(gus, STREET.offset(1, 1));
    assert_eq!(fail(&mut w, gus, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert!(!w.has_condition(cy, "detained"));
    w.assert_conserved();
}
