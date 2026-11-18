//! The whole chain through verbs: a burglary, evidence, a warrant, a
//! search, an arrest, a charge, a ruling, jail and the release at the
//! right tick, with exact balances.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

#[test]
fn a_burglar_is_found_arrested_tried_jailed_and_released() {
    // Every crime and every investigation works, every witness sees who
    // did it, and the city watch sees nothing. So only residents act.
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &SHARP]));
    let shop = site(&w, "shop");
    let station = site(&w, "police_station");
    let jail = site(&w, "jail");
    let shelter = site(&w, "shelter");
    let till = Holder::Site(shop.id);
    let evidence_room = Holder::Site(station.id);
    let door = shop.rect.centre();

    // Bo buys the shop, fills the till and the shelves, and goes home.
    let bo = resident_in(&mut w, "Bo", shop.attr("value") + 10_000, &shop);
    w.give_item(bo, "bread", 8);
    ok(&mut w, bo, "buy_property", json!({}));
    ok(&mut w, bo, "till_fund", json!({ "amount": 10_000 }));
    ok(&mut w, bo, "stock", json!({ "item": "bread", "qty": 8 }));
    ok(&mut w, bo, "move_to", json!({ "site": shelter.id }));
    assert_eq!(w.cash(bo), Money(0));
    assert_eq!(w.money(till), Money(10_000));

    // Dee stands in the street in front of the shop. Eve is the police and
    // Finn is the judge of the city.
    let dee = resident_at(&mut w, "Dee", 0, Pos::new(16, door.y));
    let eve = resident_in(&mut w, "Eve", 0, &station);
    let courthouse = site(&w, "courthouse");
    let finn = resident_in(&mut w, "Finn", 0, &courthouse);
    appoint(&mut w, "police", &["law.arrest", "law.search"], &[eve]);
    appoint(&mut w, "judge", &["law.judge"], &[finn]);

    // 1. The burglary. Cy has 3,000 cents of its own.
    let cy = resident_at(&mut w, "Cy", 3_000, door);
    let (crime, events) = run_with_events(&mut w, cy, "burgle", json!({}));
    assert!(crime.ok);
    let case = case_of(&crime.result);
    assert_eq!(
        crime.result,
        json!({
            "success": true, "case": case, "witnesses": 1, "site": shop.id,
            "cash": 5_000, "goods": { "bread": 8 }, "value": 11_400,
        })
    );
    assert_eq!(w.cash(cy), Money(8_000));
    assert_eq!(w.money(till), Money(5_000));
    let theft = of_kind(&events, "crime.theft_committed");
    assert_eq!((theft.len(), theft[0].actor), (1, None));
    w.step(1);

    // 2. Evidence. The owner was away: it learns of the loss and reports
    // it, but it saw nobody. Dee saw Cy and says so.
    let loss = notice(&w, bo, "crime.witnessed").expect("the owner learns of the burglary");
    assert_eq!(
        loss.data,
        json!({ "case": case, "kind": "burglary", "role": "victim", "site": shop.id, "value": 11_400 })
    );
    let saw = notice(&w, dee, "crime.witnessed").expect("the witness gets a notice");
    assert_eq!(saw.data["suspect"], json!(cy));
    assert_eq!(saw.data["role"], json!("witness"));
    assert_eq!(
        ok(&mut w, bo, "report_crime", json!({ "case": case })),
        json!({ "case": case, "reported": true, "named_suspect": false })
    );
    assert_eq!(
        ok(&mut w, dee, "report_crime", json!({ "case": case })),
        json!({ "case": case, "reported": true, "named_suspect": true })
    );
    w.step(1);
    // One witness is 25 points: below the 40 of a warrant. The docket
    // shows the case without a suspect.
    let docket = ok(&mut w, eve, "case_list", json!({}));
    assert_eq!(docket["cases"].as_array().map(Vec::len), Some(1));
    assert_eq!(docket["cases"][0]["case"], json!(case));
    assert_eq!(docket["cases"][0]["status"], json!("open"));
    assert_eq!(docket["cases"][0]["evidence"], json!(25));
    assert_eq!(docket["cases"][0].get("suspect"), None);
    assert!(!w.has_condition(cy, "wanted"));

    // An arrest without grounds is refused.
    w.set_pos(eve, door.offset(1, 0));
    assert_eq!(fail(&mut w, eve, "arrest", json!({ "target": cy })), ErrorCode::NotAllowed);
    assert!(!w.has_condition(cy, "detained"));

    // Eve reads the traces at the scene. A trace loses strength in a
    // straight line over 2,880 ticks.
    let (found, mut events) = run_with_events(&mut w, eve, "investigate", json!({ "case": case }));
    let age = i64::try_from(found.tick.0 - crime.tick.0).expect("a small number");
    let points = 30 * (2_880 - age) / 2_880;
    assert!((28..30).contains(&points), "{points}");
    assert_eq!(
        found.result,
        json!({ "case": case, "kind": "burglary", "found": true, "evidence": points })
    );
    assert_eq!(w.attr(eve, "skill.investigation"), 10);

    // 3. The warrant. 25 + 29 points are above 40. The registry writes
    // the warrant in the frame of the find. Cy is wanted one frame later.
    events.extend(step_events(&mut w, 2));
    assert!(w.has_condition(cy, "wanted"));
    let warrants = of_kind(&events, "crime.warrant_issued");
    assert_eq!(warrants.len(), 1);
    assert_eq!(warrants[0].actor, None);
    assert_eq!(warrants[0].data, json!({ "case": case, "kind": "burglary", "city": 0 }));
    let wanted = notice(&w, cy, "crime.wanted").expect("the suspect learns of the warrant");
    assert_eq!(wanted.data, json!({ "case": case, "kind": "burglary", "wanted": true }));
    let view = case_view(&mut w, eve, case);
    assert_eq!(view["status"], json!("wanted"));
    assert_eq!(view["suspect"], json!(cy));
    assert_eq!(view["evidence"], json!(25 + points));

    // 4. The search. The loot goes to the police station.
    let found = ok(&mut w, eve, "search", json!({ "target": cy }));
    assert_eq!(
        found,
        json!({
            "target": cy, "found": true, "money": 5_000, "goods": { "bread": 8 },
            "contraband": {}, "cases": [case],
        })
    );
    assert_eq!(w.cash(cy), Money(3_000));
    assert_eq!(w.qty(Holder::Resident(cy), "bread"), 0);
    assert_eq!(w.money(evidence_room), Money(5_000));
    assert_eq!(w.qty(evidence_room, "bread"), 8);

    // 5. The arrest. Cy goes to the jail, which is in another district.
    let (arrest, events) = run_with_events(&mut w, eve, "arrest", json!({ "target": cy }));
    assert_eq!(arrest.result, json!({ "target": cy, "cases": [case], "jail": jail.id }));
    assert!(w.has_condition(cy, "detained"));
    assert!(!w.has_condition(cy, "wanted"));
    assert_eq!(site_kind_of(&w, cy).as_deref(), Some("jail"));
    let arrests = of_kind(&events, "crime.arrest_made");
    assert_eq!(arrests.len(), 1);
    assert_eq!(arrests[0].actor, Some(eve));
    assert_eq!(
        arrests[0].data,
        json!({
            "officer": eve, "target": cy, "case": case, "cases": [case], "city": 0,
            "watch": false,
        })
    );
    assert_eq!(fail(&mut w, cy, "move_to", json!({ "x": 100, "y": 20 })), ErrorCode::Blocked);

    // 6. The charge and the trial. The city has a judge, so no jury yet.
    let (charge, events) =
        run_with_events(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    assert_eq!(charge.result, json!({ "case": case, "target": cy, "decided_in": 720 }));
    let charged = of_kind(&events, "crime.suspect_charged");
    assert_eq!(charged.len(), 1);
    assert_eq!(charged[0].data["target"], json!(cy));
    assert_eq!(of_kind(&events, "crime.jury_summoned").len(), 0);
    let record = law(&w, cy).expect("the suspect has a record");
    assert_eq!(record["custody"]["state"], json!("detained"));
    assert_eq!(record["custody"]["site"], json!(jail.id));
    assert_eq!(record["trials"][0]["case"], json!(case));

    // 7. The ruling: a fine of 1,000, restitution of 2,000, 300 ticks.
    let treasury = w.money(TREASURY);
    let ruling = json!({
        "case": case, "guilty": true, "fine": 1_000, "restitution": 2_000, "jail_ticks": 300,
    });
    let (ruled, events) = run_with_events(&mut w, finn, "rule_case", ruling);
    assert_eq!(
        ruled.result,
        json!({ "case": case, "guilty": true, "fine": 1_000, "jail_ticks": 300, "restitution": 2_000 })
    );
    let verdicts = of_kind(&events, "crime.verdict_given");
    assert_eq!(verdicts.len(), 1);
    assert_eq!(
        verdicts[0].data,
        json!({
            "case": case, "kind": "burglary", "city": 0, "target": cy, "guilty": true,
            "by": "judge", "judge": finn, "fine": 1_000, "jail_ticks": 300,
            "restitution": 2_000, "votes_guilty": 0, "votes_not_guilty": 0,
        })
    );

    // 8. The engine carries out the sentence in the zone of the jail.
    let until = jailed_until(&w, cy).expect("the convict is in jail");
    let verdict = notice(&w, cy, "crime.verdict").expect("the convict gets the verdict");
    assert_eq!(
        verdict.data,
        json!({
            "case": case, "kind": "burglary", "guilty": true, "fine": 1_000,
            "restitution": 2_000, "jail_ticks": 300, "paid": 3_000, "owed": 0,
            "jailed_until": until,
        })
    );
    assert_eq!(until, verdict.tick.0 + 300);
    assert!(!w.has_condition(cy, "detained"));
    assert_eq!(w.attr(cy, "rep.notoriety"), 40);
    assert_eq!(w.attr(cy, "rep.standing"), -60);
    w.step(1);
    // Exact balances: the fine reached the city, the restitution reached
    // the owner of the shop, the loot is in the police station.
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.money(TREASURY), treasury + Money(1_000));
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(w.money(till), Money(5_000));
    assert_eq!(w.money(evidence_room), Money(5_000));
    assert_eq!(
        notice(&w, bo, "crime.restitution").expect("the owner is told").data,
        json!({ "case": case, "amount": 2_000 })
    );
    w.assert_conserved();

    // 9. Jail. The convict cannot walk out, and is free at the tick of the
    // sentence, not one tick before.
    assert_eq!(fail(&mut w, cy, "move_to", json!({ "x": 100, "y": 20 })), ErrorCode::Blocked);
    let now = tick_of(&w, cy);
    let events = step_events(&mut w, u32::try_from(until - now - 1).expect("a short sentence"));
    assert_eq!(tick_of(&w, cy), until - 1);
    assert!(w.has_condition(cy, "jailed"));
    assert_eq!(of_kind(&events, "crime.sentence_served").len(), 0);
    let events = step_events(&mut w, 1);
    assert_eq!(tick_of(&w, cy), until);
    assert!(!w.has_condition(cy, "jailed"));
    let served = of_kind(&events, "crime.sentence_served");
    assert_eq!(served.len(), 1);
    assert_eq!(served[0].data, json!({ "resident": cy, "free": true }));
    assert!(w.is_alive(cy));
    // The record is clean again, and Cy can walk out of the jail.
    assert_eq!(law(&w, cy), None);
    w.act_ok(cy, "move_to", json!({ "site": shelter.id }));
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["status"], json!("convicted"));
    assert_eq!(view["sentence"]["by"], json!("judge"));
    w.assert_conserved();
}
