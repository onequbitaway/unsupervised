//! `rob` and `assault`: the normal path, each refusal, injuries and a
//! death by violence.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn a_robber_takes_cash_and_goods_and_hurts_the_victim() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &SHARP]));
    let bo = resident_at(&mut w, "Bo", 1_000, STREET);
    w.give_item(bo, "bread", 3);
    w.give_item(bo, "grain", 4);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    // The victim does something that the robbery stops.
    let waiting = w.act(bo, "wait", json!({ "ticks": 100 })).action_id;

    let (report, events) = run_with_events(&mut w, cy, "rob", json!({ "target": bo }));
    assert!(report.ok);
    let case = case_of(&report.result);
    // 60 percent of the cash. Five units: the bread first, then grain.
    assert_eq!(
        report.result,
        json!({
            "success": true, "case": case, "witnesses": 1, "target": bo,
            "cash": 600, "goods": { "bread": 3, "grain": 2 }, "value": 3_400,
            "injury": 100, "fatal": false, "hurt": 0,
        })
    );
    assert_eq!((w.cash(cy), w.cash(bo)), (Money(600), Money(400)));
    let (robber, victim) = (Holder::Resident(cy), Holder::Resident(bo));
    assert_eq!((w.qty(robber, "bread"), w.qty(robber, "grain")), (3, 2));
    assert_eq!((w.qty(victim, "bread"), w.qty(victim, "grain")), (0, 2));
    assert_eq!(w.attr(bo, "need.health"), 900);
    assert_eq!(w.attr(cy, "need.health"), 1_000);
    assert_eq!(w.attr(cy, "skill.combat"), 12);

    // The action of the victim stopped.
    let stopped = w.reports(bo).into_iter().find(|r| r.action_id == waiting).expect("a report");
    assert_eq!(stopped.error.map(|error| error.code), Some(ErrorCode::Interrupted));

    // The victim stood face to face with the robber and recognized it.
    let seen = notice(&w, bo, "crime.witnessed").expect("the victim gets a notice");
    assert_eq!(
        seen.data,
        json!({
            "case": case, "kind": "robbery", "role": "victim", "pos": STREET, "value": 3_400,
            "suspect": cy,
        })
    );
    // The public event still does not name the robber.
    let thefts = of_kind(&events, "crime.theft_committed");
    assert_eq!(thefts.len(), 1);
    assert_eq!(thefts[0].actor, None);
    assert_eq!(
        thefts[0].data,
        json!({
            "case": case, "kind": "robbery", "city": 0, "site": null, "value": 3_400,
            "attempt": false,
        })
    );
    w.assert_conserved();
}

#[test]
fn a_robbery_that_fails_hurts_the_robber() {
    let mut w = town(&with(&[&[NEVER, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 1_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let result = ok(&mut w, cy, "rob", json!({ "target": bo }));
    assert_eq!(result["success"], json!(false));
    assert_eq!(result["hurt"], json!(60));
    assert_eq!(result["cash"], json!(0));
    assert_eq!((w.cash(cy), w.cash(bo)), (Money(0), Money(1_000)));
    assert_eq!(w.attr(cy, "need.health"), 940);
    assert_eq!(w.attr(bo, "need.health"), 1_000);
    assert_eq!(w.attr(cy, "skill.combat"), 4);

    // A failed attack never kills the attacker.
    w.set_attr(cy, "need.health", 20);
    let result = ok(&mut w, cy, "assault", json!({ "target": bo }));
    assert_eq!(result["hurt"], json!(19));
    assert_eq!(w.attr(cy, "need.health"), 1);
    assert!(w.is_alive(cy));
    w.assert_conserved();
}

#[test]
fn rob_and_assault_refuse_a_bad_target() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 1_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let dee = resident_at(&mut w, "Dee", 0, STREET.offset(0, 4));
    for verb in ["rob", "assault"] {
        assert_eq!(fail(&mut w, cy, verb, json!({ "target": cy })), ErrorCode::InvalidArgs);
        assert_eq!(fail(&mut w, cy, verb, json!({ "target": dee })), ErrorCode::NotHere);
        assert_eq!(fail(&mut w, cy, verb, json!({ "target": 404 })), ErrorCode::NotHere);
        assert_eq!(fail(&mut w, cy, verb, json!({ "target": "bo" })), ErrorCode::InvalidArgs);
    }
    // The guards protect a resident under arrest.
    w.set_condition(bo, "detained", None);
    for verb in ["rob", "assault"] {
        assert_eq!(fail(&mut w, cy, verb, json!({ "target": bo })), ErrorCode::NotAllowed);
    }
    w.with_resident(bo, |realm, idx| {
        let detained = realm.defs().condition_id("detained").expect("the condition");
        realm.clear_condition(idx, detained);
    });
    // A victim that is gone when the blow comes is safe.
    w.act_ok(cy, "assault", json!({ "target": bo }));
    w.set_pos(bo, STREET.offset(0, -9));
    w.run_until_idle(cy, 10);
    let report = w.last_report(cy).expect("the action ended");
    assert_eq!(report.error.map(|error| error.code), Some(ErrorCode::NotHere));
    assert_eq!(w.attr(bo, "need.health"), 1_000);
    assert_eq!(w.cash(bo), Money(1_000));
    w.assert_conserved();
}

#[test]
fn an_assault_takes_health_and_the_combat_level_adds_to_it() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 0, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));

    let (report, events) = run_with_events(&mut w, cy, "assault", json!({ "target": bo }));
    let case = case_of(&report.result);
    assert_eq!(
        report.result,
        json!({
            "success": true, "case": case, "witnesses": 1, "target": bo,
            "damage": 150, "fatal": false, "hurt": 0,
        })
    );
    assert_eq!(w.attr(bo, "need.health"), 850);
    let assaults = of_kind(&events, "crime.assault_committed");
    assert_eq!(assaults.len(), 1);
    assert_eq!(assaults[0].actor, None);
    assert_eq!(
        assaults[0].data,
        json!({
            "case": case, "kind": "assault", "city": 0, "victim": bo, "fatal": false,
            "attempt": false,
        })
    );
    // The loss of the victim is the fee of the hospital for 150 points.
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["value"], json!(150 * 20));

    // Combat level 2 (400 experience points) adds 2 * 25.
    w.set_attr(cy, "skill.combat", 400);
    let result = ok(&mut w, cy, "assault", json!({ "target": bo }));
    assert_eq!(result["damage"], json!(200));
    assert_eq!(w.attr(bo, "need.health"), 650);
    w.assert_conserved();
}

#[test]
fn an_assault_can_kill_and_the_death_is_a_death_by_violence() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 700, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    w.set_attr(bo, "need.health", 120);
    let treasury = w.money(TREASURY);

    let (report, events) = run_with_events(&mut w, cy, "assault", json!({ "target": bo }));
    let case = case_of(&report.result);
    assert_eq!(report.result["damage"], json!(120));
    assert_eq!(report.result["fatal"], json!(true));
    let deaths = of_kind(&events, "life.resident_died");
    assert_eq!(deaths.len(), 1);
    assert_eq!(deaths[0].data["resident"], json!(bo));
    assert_eq!(deaths[0].data["cause"], json!("violence"));
    assert_eq!(of_kind(&events, "crime.assault_committed")[0].data["fatal"], json!(true));
    w.step(2);
    assert!(!w.is_alive(bo));
    // The victim had no heir: its cash went to the city.
    assert_eq!(w.money(TREASURY), treasury + Money(700));
    let view = case_view(&mut w, cy, case);
    assert_eq!(view["kind"], json!("homicide"));
    w.assert_conserved();
}

#[test]
fn a_robbery_can_kill_and_the_robber_keeps_the_loot() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 1_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    w.set_attr(bo, "need.health", 100);
    let treasury = w.money(TREASURY);

    let (report, events) = run_with_events(&mut w, cy, "rob", json!({ "target": bo }));
    assert_eq!(report.result["cash"], json!(600));
    assert_eq!(report.result["injury"], json!(100));
    assert_eq!(report.result["fatal"], json!(true));
    assert_eq!(of_kind(&events, "life.resident_died")[0].data["cause"], json!("violence"));
    // A robbery that kills is an attack for viewers.
    assert_eq!(of_kind(&events, "crime.theft_committed").len(), 0);
    assert_eq!(of_kind(&events, "crime.assault_committed")[0].data["kind"], json!("homicide"));
    w.step(2);
    assert!(!w.is_alive(bo));
    assert_eq!(w.cash(cy), Money(600));
    // The rest of the cash is the estate.
    assert_eq!(w.money(TREASURY), treasury + Money(400));
    w.assert_conserved();
}

#[test]
fn the_stronger_fighter_wins_and_a_victim_that_sleeps_is_easy() {
    // Only the difference of the combat levels decides.
    let rules = [
        ("crime.chance_min_bp", 0),
        ("crime.chance_max_bp", 10_000),
        ("crime.assault_base_bp", 0),
        ("crime.combat_level_bp", 10_000),
        ("crime.force_asleep_bonus_bp", 0),
        NO_WATCH,
    ];
    let fight = |attacker_xp: i64, victim_xp: i64| {
        let mut w = town(&with(&[&rules, &BLIND]));
        let bo = resident_at(&mut w, "Bo", 0, STREET);
        let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
        w.set_attr(cy, "skill.combat", attacker_xp);
        w.set_attr(bo, "skill.combat", victim_xp);
        let won = ok(&mut w, cy, "assault", json!({ "target": bo }))["success"] == json!(true);
        w.assert_conserved();
        won
    };
    assert!(fight(100, 0));
    assert!(!fight(100, 100));
    assert!(!fight(0, 400));

    // Only the bonus against a victim that sleeps decides. The residents
    // have no routines: the routine `sleep when tired` would put the tired
    // victim back to sleep in the tick in which the robbery wakes it.
    let rules = [
        ("crime.chance_min_bp", 0),
        ("crime.chance_max_bp", 10_000),
        ("crime.rob_base_bp", 0),
        ("crime.combat_level_bp", 0),
        ("crime.force_asleep_bonus_bp", 10_000),
        ("life.spawn_routines", 0),
        NO_WATCH,
    ];
    let rob = |asleep: bool| {
        let mut w = town(&with(&[&rules, &BLIND]));
        let bo = resident_at(&mut w, "Bo", 500, STREET);
        let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
        if asleep {
            w.set_attr(bo, "need.energy", 100);
            w.act_ok(bo, "sleep", json!({ "ticks": 300 }));
        }
        let result = ok(&mut w, cy, "rob", json!({ "target": bo }));
        // A robbery wakes its victim.
        assert!(!w.has_condition(bo, "asleep"));
        w.assert_conserved();
        result["success"] == json!(true)
    };
    assert!(rob(true));
    assert!(!rob(false));
}
