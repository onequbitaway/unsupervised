//! Trials: the judge, the jury, the deadline, and the refusals of
//! `charge`, `rule_case` and `jury_vote`.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Money, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// A world after a theft that the city watch saw (60 evidence points):
/// the thief is under arrest and the officer has not charged it yet.
struct Arrested {
    w: TestWorld<'static>,
    bo: ResidentId,
    cy: ResidentId,
    eve: ResidentId,
    case: u64,
    /// Frame of the arrest.
    frame: u64,
}

/// Cy, a resident of owner 2, takes 500 of the 2,000 cents of Bo. Eve
/// arrests Cy.
fn arrested(rules: &[(&'static str, i64)]) -> Arrested {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND, rules]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest", "law.search"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = stranger_at(&mut w, "Cy", 2, 0, STREET.offset(1, 0));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    let (_, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    let frame = frame_of(&events, "crime.arrest_made");
    Arrested { w, bo, cy, eve, case, frame }
}

/// Returns the residents with a jury duty in a case.
fn jurors(w: &TestWorld<'_>, pool: &[ResidentId], case: u64) -> Vec<ResidentId> {
    pool.iter()
        .copied()
        .filter(|&r| law(w, r).is_some_and(|law| law["jury_duty"][0]["case"] == json!(case)))
        .collect()
}

#[test]
fn with_no_judge_a_jury_is_called_and_its_votes_decide_at_the_deadline() {
    let Arrested { mut w, bo, cy, eve, case, .. } =
        arrested(&[("crime.jury_size", 3), ("crime.jury_quorum", 2)]);
    let pool: Vec<ResidentId> =
        (0..6).map(|i| resident_at(&mut w, "Juror", 0, Pos::new(70 + i, 70))).collect();
    w.step(1);

    let (charge, events) =
        run_with_events(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    assert_eq!(charge.result["decided_in"], json!(720));
    let charged = frame_of(&events, "crime.suspect_charged");
    // The city has no judge: the jury is called in the frame of the charge.
    let called = of_kind(&events, "crime.jury_summoned");
    assert_eq!(called.len(), 1);
    assert_eq!(called[0].frame, charged);
    assert_eq!(called[0].data, json!({ "case": case, "kind": "theft", "city": 0, "jurors": 3 }));
    let jury = jurors(&w, &pool, case);
    assert_eq!(jury.len(), 3);
    let duty = notice(&w, jury[0], "crime.jury_duty").expect("a juror is told");
    assert_eq!(duty.data["case"], json!(case));
    assert_eq!(duty.data["kind"], json!("theft"));
    // The victim, the officer and the defendant do not sit on the jury.
    assert_eq!(jurors(&w, &[bo, cy, eve], case), []);

    // Refusals of `jury_vote`.
    let outsider = *pool.iter().find(|r| !jury.contains(r)).expect("a resident without duty");
    let vote = |guilty: bool| json!({ "case": case, "guilty": guilty });
    assert_eq!(fail(&mut w, outsider, "jury_vote", vote(true)), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, bo, "jury_vote", vote(true)), ErrorCode::NotAllowed);
    assert_eq!(
        fail(&mut w, jury[0], "jury_vote", json!({ "case": case + 1, "guilty": true })),
        ErrorCode::NotFound
    );
    assert_eq!(fail(&mut w, jury[0], "jury_vote", json!({ "case": case })), ErrorCode::InvalidArgs);

    // Two of three jurors vote for guilty. The third never votes.
    assert_eq!(
        ok(&mut w, jury[0], "jury_vote", vote(true)),
        json!({ "case": case, "guilty": true, "votes": 1, "jurors": 3 })
    );
    assert_eq!(fail(&mut w, jury[0], "jury_vote", vote(false)), ErrorCode::Conflict);
    assert_eq!(ok(&mut w, jury[1], "jury_vote", vote(true))["votes"], json!(2));
    assert_eq!(w.attr(jury[0], "skill.law"), 5);
    // A juror that voted has no duty any more. The trial still runs.
    assert_eq!(jurors(&w, &pool, case), [jury[2]]);
    let view = case_view(&mut w, jury[0], case);
    assert_eq!(view["status"], json!("trial"));
    assert_eq!(
        view["trial"],
        json!({
            "charged_by": eve, "jury": true, "jurors": 3, "votes": 2,
            "you_are_juror": true, "your_vote": true,
        })
    );

    // The engine decides at the deadline: 720 frames after the charge.
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(charged + 720 - now).expect("soon"));
    let verdicts = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(verdicts[0].frame, charged + 720);
    // The sentence of the engine for a theft: 3,000 fine, 240 ticks, and
    // the loss of the victim as restitution.
    assert_eq!(
        verdicts[0].data,
        json!({
            "case": case, "kind": "theft", "city": 0, "target": cy, "guilty": true,
            "by": "jury", "judge": null, "fine": 3_000, "jail_ticks": 240,
            "restitution": 500, "votes_guilty": 2, "votes_not_guilty": 0,
        })
    );
    w.step(1);
    assert!(w.has_condition(cy, "jailed"));
    assert_eq!(jurors(&w, &pool, case), []);
    assert_eq!(fail(&mut w, jury[2], "jury_vote", vote(true)), ErrorCode::Conflict);
    // The victim got its 500 cents back and a notice of the verdict.
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(
        notice(&w, bo, "crime.verdict").expect("the victim is told").data,
        json!({ "case": case, "kind": "theft", "guilty": true, "defendant": cy, "restitution": 500 })
    );
    w.assert_conserved();
}

#[test]
fn a_jury_that_votes_in_full_ends_the_trial_at_once_and_can_acquit() {
    let Arrested { mut w, bo, cy, eve, case, .. } =
        arrested(&[("crime.jury_size", 3), ("crime.jury_quorum", 2)]);
    let pool: Vec<ResidentId> =
        (0..3).map(|i| resident_at(&mut w, "Juror", 0, Pos::new(70 + i, 70))).collect();
    w.step(1);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let jury = jurors(&w, &pool, case);
    assert_eq!(jury.len(), 3);

    // The evidence is enough for the engine to convict. The jury does not
    // care: two of three vote for not guilty.
    assert_eq!(case_view(&mut w, bo, case)["evidence"], json!(60));
    ok(&mut w, jury[0], "jury_vote", json!({ "case": case, "guilty": true }));
    ok(&mut w, jury[1], "jury_vote", json!({ "case": case, "guilty": false }));
    let (_, mut events) =
        run_with_events(&mut w, jury[2], "jury_vote", json!({ "case": case, "guilty": false }));
    events.extend(step_events(&mut w, 1));
    let verdicts = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(
        verdicts[0].data,
        json!({
            "case": case, "kind": "theft", "city": 0, "target": cy, "guilty": false,
            "by": "jury", "judge": null, "fine": 0, "jail_ticks": 0, "restitution": 0,
            "votes_guilty": 1, "votes_not_guilty": 2,
        })
    );
    // The defendant is free, keeps the loot and has no record.
    assert!(!w.has_condition(cy, "detained"));
    assert!(!w.has_condition(cy, "jailed"));
    assert_eq!(
        of_kind(&events, "crime.suspect_released")[0].data,
        json!({ "target": cy, "reason": "acquitted", "by": null })
    );
    assert_eq!(w.attr(cy, "rep.notoriety"), 0);
    assert_eq!(w.attr(cy, "rep.standing"), 0);
    assert_eq!(w.cash(cy), Money(500));
    assert_eq!(case_view(&mut w, bo, case)["status"], json!("acquitted"));
    // An acquitted resident is not wanted again for the same case.
    w.step(5);
    assert!(!w.has_condition(cy, "wanted"));
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotHere);
    w.assert_conserved();
}

#[test]
fn without_votes_the_evidence_decides_at_the_deadline() {
    // 60 points of the watch reach the threshold of 60: guilty.
    let Arrested { mut w, cy, eve, case, .. } = arrested(&[("crime.trial_ticks", 100)]);
    let (_, events) = run_with_events(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let charged = frame_of(&events, "crime.suspect_charged");
    // The world has three residents: the defendant, the victim and the
    // officer that charged. None of them can sit on the jury.
    assert_eq!(of_kind(&events, "crime.jury_summoned")[0].data["jurors"], json!(0));
    let events = step_events(&mut w, 100);
    let verdict = of_kind(&events, "crime.verdict_given")[0];
    assert_eq!(verdict.frame, charged + 100);
    assert_eq!(verdict.data["by"], json!("evidence"));
    assert_eq!(verdict.data["guilty"], json!(true));
    w.step(1);
    assert!(w.has_condition(cy, "jailed"));

    // 59 points are below it: not guilty.
    let Arrested { mut w, cy, eve, case, .. } =
        arrested(&[("crime.trial_ticks", 100), ("crime.evidence_watch", 59)]);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let events = step_events(&mut w, 100);
    let verdict = of_kind(&events, "crime.verdict_given")[0];
    assert_eq!(verdict.data["by"], json!("evidence"));
    assert_eq!(verdict.data["guilty"], json!(false));
    assert_eq!(verdict.data["fine"], json!(0));
    w.step(1);
    assert!(!w.has_condition(cy, "jailed"));
    assert!(!w.has_condition(cy, "detained"));
    w.assert_conserved();
}

#[test]
fn the_engine_waits_for_a_judge_of_the_city_before_it_calls_a_jury() {
    let Arrested { mut w, cy, eve, case, .. } = arrested(&[]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let (_, events) = run_with_events(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let charged = frame_of(&events, "crime.suspect_charged");
    assert_eq!(of_kind(&events, "crime.jury_summoned").len(), 0);

    // The judge does nothing. After 240 frames the jury is called. The
    // judge is free for it, the officer that charged is not.
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(charged + 240 - now).expect("soon"));
    let called = of_kind(&events, "crime.jury_summoned");
    assert_eq!(called.len(), 1);
    assert_eq!(called[0].frame, charged + 240);
    w.step(1);
    assert_eq!(jurors(&w, &[finn, eve], case), [finn]);

    // The judge can still rule: an acquittal.
    let (ruled, events) =
        run_with_events(&mut w, finn, "rule_case", json!({ "case": case, "guilty": false }));
    assert_eq!(
        ruled.result,
        json!({ "case": case, "guilty": false, "fine": 0, "jail_ticks": 0, "restitution": 0 })
    );
    assert_eq!(of_kind(&events, "crime.verdict_given")[0].data["by"], json!("judge"));
    w.step(1);
    assert!(!w.has_condition(cy, "detained"));
    // The duty of the juror ended with the trial.
    assert_eq!(jurors(&w, &[finn], case), []);
    assert_eq!(w.attr(finn, "skill.law"), 5);
    w.assert_conserved();
}

#[test]
fn rule_case_keeps_the_limits_and_refuses_bad_requests() {
    let Arrested { mut w, bo, cy, eve, case, .. } = arrested(&[]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let guilty = |more: serde_json::Value| {
        let mut args = json!({ "case": case, "guilty": true });
        args.as_object_mut()
            .expect("an object")
            .extend(more.as_object().expect("an object").clone());
        args
    };

    // No trial yet.
    assert_eq!(fail(&mut w, finn, "rule_case", guilty(json!({}))), ErrorCode::Conflict);
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));

    // Only a judge rules.
    assert_eq!(fail(&mut w, eve, "rule_case", guilty(json!({}))), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, bo, "rule_case", guilty(json!({}))), ErrorCode::NotAllowed);
    assert_eq!(
        fail(&mut w, finn, "rule_case", json!({ "case": case + 1, "guilty": true })),
        ErrorCode::NotFound
    );
    assert_eq!(fail(&mut w, finn, "rule_case", json!({ "case": case })), ErrorCode::InvalidArgs);
    // The limits of a theft: a fine of 12,000 and 1,440 ticks. The loss
    // was 500.
    for over in [
        json!({ "fine": 12_001 }),
        json!({ "jail_ticks": 1_441 }),
        json!({ "restitution": 501 }),
        json!({ "fine": -1 }),
        json!({ "jail_ticks": -1 }),
        json!({ "restitution": -1 }),
    ] {
        assert_eq!(fail(&mut w, finn, "rule_case", guilty(over)), ErrorCode::InvalidArgs);
    }
    assert!(w.has_condition(cy, "detained"));
    assert!(!w.has_condition(cy, "jailed"));

    // The greatest sentence is inside the limits.
    let most = json!({ "fine": 12_000, "jail_ticks": 1_440, "restitution": 500 });
    let result = ok(&mut w, finn, "rule_case", guilty(most));
    assert_eq!(
        result,
        json!({ "case": case, "guilty": true, "fine": 12_000, "jail_ticks": 1_440, "restitution": 500 })
    );
    w.step(1);
    let until = jailed_until(&w, cy).expect("the convict is in jail");
    assert_eq!(until, notice(&w, cy, "crime.verdict").expect("a verdict").tick.0 + 1_440);
    // A case that is decided cannot be ruled again.
    assert_eq!(fail(&mut w, finn, "rule_case", guilty(json!({}))), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn a_ruling_without_numbers_gives_the_sentence_of_the_engine_and_a_judge_cannot_judge_itself() {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    // The thief is a judge of the city.
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[cy, finn]);
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    // A resident under arrest cannot rule at all.
    assert_eq!(
        fail(&mut w, cy, "rule_case", json!({ "case": case, "guilty": false })),
        ErrorCode::Blocked
    );
    let result = ok(&mut w, finn, "rule_case", json!({ "case": case, "guilty": true }));
    assert_eq!(
        result,
        json!({ "case": case, "guilty": true, "fine": 3_000, "jail_ticks": 240, "restitution": 500 })
    );
    w.assert_conserved();
}

#[test]
fn a_defendant_that_an_officer_frees_stays_on_trial_and_cannot_rule_its_own_case() {
    // Cy is a judge of the city. Eve saw its theft on patrol: 100 points.
    let rules = [("crime.patrol_detect_bp", 10_000), ("crime.trial_ticks", 100)];
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND, &rules]));
    let jail = site(&w, "jail");
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    let gus = resident_at(&mut w, "Gus", 0, jail.rect.centre().offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve, gus]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    appoint(&mut w, "judge", &["law.judge"], &[cy]);
    w.act_ok(eve, "patrol", json!({ "ticks": 10 }));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.run_until_idle(eve, 20);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));

    // In one frame: Eve charges, and Gus, who does not know of it yet,
    // lets Cy go. The charge counts. Cy is free and on trial.
    w.act_ok(eve, "charge", json!({ "target": cy, "case": case }));
    w.act_ok(gus, "release", json!({ "target": cy }));
    let events = step_events(&mut w, 2);
    let charged = frame_of(&events, "crime.suspect_charged");
    assert!(!w.has_condition(cy, "detained"));
    assert_eq!(case_view(&mut w, bo, case)["status"], json!("trial"));

    // Cy holds `law.judge` and is not blocked. The registry still refuses.
    assert_eq!(
        fail(&mut w, cy, "rule_case", json!({ "case": case, "guilty": false })),
        ErrorCode::NotAllowed
    );
    assert_eq!(case_view(&mut w, bo, case)["status"], json!("trial"));

    // Cy walks away. At the deadline the evidence convicts, and the engine
    // brings Cy back to the jail.
    w.set_pos(cy, STREET);
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(charged + 100 - now).expect("soon") + 2);
    assert_eq!(of_kind(&events, "crime.verdict_given")[0].data["guilty"], json!(true));
    assert!(w.has_condition(cy, "jailed"));
    assert_eq!(site_kind_of(&w, cy).as_deref(), Some("jail"));
    w.assert_conserved();
}

#[test]
fn charge_refuses_bad_requests() {
    let Arrested { mut w, bo, cy, eve, case, .. } = arrested(&[]);
    let charge = |target: ResidentId, case: u64| json!({ "target": target, "case": case });
    // Only the police charge.
    assert_eq!(fail(&mut w, bo, "charge", charge(cy, case)), ErrorCode::NotAllowed);
    // Bo is not under arrest. No case has this number.
    assert_eq!(fail(&mut w, eve, "charge", charge(bo, case)), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, eve, "charge", charge(cy, case + 1)), ErrorCode::NotFound);
    assert_eq!(fail(&mut w, eve, "charge", json!({ "target": cy })), ErrorCode::InvalidArgs);

    // Dee is under arrest for another theft. The evidence of the first
    // case does not point at Dee: the answer is the same as for too
    // little evidence.
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(1, 0));
    let other = case_of(&ok(&mut w, dee, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": dee }));
    assert_eq!(fail(&mut w, eve, "charge", charge(dee, case)), ErrorCode::Insufficient);
    assert_eq!(fail(&mut w, eve, "charge", charge(cy, other)), ErrorCode::Insufficient);

    ok(&mut w, eve, "charge", charge(cy, case));
    assert_eq!(w.attr(eve, "skill.law"), 8 + 8 + 5);
    // A case on trial cannot be charged again.
    assert_eq!(fail(&mut w, eve, "charge", charge(cy, case)), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn the_engine_charges_when_the_officer_does_not() {
    let Arrested { mut w, cy, case, frame, .. } =
        arrested(&[("crime.hold_ticks", 50), ("crime.trial_ticks", 100)]);
    // The officer does nothing. After the hold time the engine charges.
    let events = step_events(&mut w, 60);
    let charged = of_kind(&events, "crime.suspect_charged");
    assert_eq!(charged.len(), 1);
    assert_eq!(charged[0].frame, frame + 50);
    assert_eq!(
        charged[0].data,
        json!({
            "case": case, "kind": "theft", "city": 0, "target": cy, "officer": null,
            "decided_in": 100,
        })
    );
    assert!(w.has_condition(cy, "detained"));
    let events = step_events(&mut w, 100);
    assert_eq!(of_kind(&events, "crime.verdict_given")[0].data["guilty"], json!(true));
    w.assert_conserved();
}
