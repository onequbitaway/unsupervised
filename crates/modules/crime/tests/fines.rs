//! Fines and restitutions: the engine takes what the convict has, the
//! convict can pay the rest with `pay_fine`, and what is not paid in time
//! becomes jail time.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Money, Pos, ResidentId};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

/// A world in which a judge just fined a thief.
struct Fined {
    w: TestWorld<'static>,
    bo: ResidentId,
    cy: ResidentId,
    case: u64,
    /// Frame of the verdict.
    frame: u64,
}

/// Cy takes 500 of the 2,000 cents of Bo and has `cash` more of its own.
/// The judge gives a fine of 4,000, a restitution of 500 and `jail` ticks.
fn fined(cash: i64, jail: i64, rules: &[(&'static str, i64)]) -> Fined {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND, rules]));
    let eve = resident_at(&mut w, "Eve", 0, STREET.offset(0, 1));
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", cash, STREET.offset(1, 0));
    let case = case_of(&ok(&mut w, cy, "pickpocket", json!({ "target": bo })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let ruling = json!({
        "case": case, "guilty": true, "fine": 4_000, "restitution": 500, "jail_ticks": jail,
    });
    let (_, mut events) = run_with_events(&mut w, finn, "rule_case", ruling);
    events.extend(step_events(&mut w, 1));
    let frame = frame_of(&events, "crime.verdict_given");
    Fined { w, bo, cy, case, frame }
}

#[test]
fn the_court_takes_the_cash_of_the_convict_and_the_victim_comes_first() {
    // Cy has the 500 stolen cents and 300 of its own: 800.
    let Fined { w, bo, cy, case, .. } = fined(300, 0, &[]);
    let verdict = notice(&w, cy, "crime.verdict").expect("the convict gets the verdict");
    assert_eq!(verdict.data["paid"], json!(800));
    assert_eq!(verdict.data["owed"], json!(3_700));
    assert_eq!(verdict.data.get("jailed_until"), None);
    // The restitution is paid in full, then 300 of the fine.
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(
        notice(&w, bo, "crime.restitution").expect("the victim is told").data,
        json!({ "case": case, "amount": 500 })
    );
    // No jail time: the convict is free, with a debt.
    assert!(!w.has_condition(cy, "detained"));
    assert!(!w.has_condition(cy, "jailed"));
    let record = law(&w, cy).expect("the convict has a debt");
    assert_eq!(record["debts"]["owed"], json!(3_700));
    w.assert_conserved();
}

#[test]
fn pay_fine_pays_the_debt_to_the_city() {
    let Fined { mut w, cy, .. } = fined(300, 0, &[]);
    let treasury = w.money(TREASURY);

    // No cash: nothing to pay with.
    assert_eq!(fail(&mut w, cy, "pay_fine", json!({})), ErrorCode::Insufficient);
    assert_eq!(fail(&mut w, cy, "pay_fine", json!({ "amount": 0 })), ErrorCode::InvalidArgs);

    // A part.
    w.give_money(cy, Money(5_000));
    let paid = ok(&mut w, cy, "pay_fine", json!({ "amount": 1_000 }));
    assert_eq!(paid, json!({ "paid": 1_000, "owed": 2_700 }));
    let events = step_events(&mut w, 1);
    let receipts = of_kind(&events, "crime.fine_paid");
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].data, json!({ "resident": cy, "amount": 1_000, "owed": 2_700 }));
    assert_eq!(w.cash(cy), Money(4_000));
    // The rest. The verb never takes more than the debt.
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 2_700, "owed": 0 }));
    assert_eq!(w.cash(cy), Money(1_300));
    w.step(1);
    assert_eq!(w.money(TREASURY), treasury + Money(3_700));
    assert_eq!(law(&w, cy), None);
    assert_eq!(fail(&mut w, cy, "pay_fine", json!({})), ErrorCode::Conflict);

    // Nothing more happens when the time to pay is over.
    let events = step_events(&mut w, 1_500);
    assert_eq!(of_kind(&events, "crime.fine_defaulted").len(), 0);
    assert!(!w.has_condition(cy, "jailed"));
    w.assert_conserved();
}

#[test]
fn a_fine_that_is_not_paid_in_time_becomes_jail_time() {
    let Fined { mut w, cy, case, frame, .. } = fined(300, 0, &[("crime.fine_due_ticks", 200)]);
    let jail = site(&w, "jail");
    // The convict is free and walks away from the jail.
    let home = site(&w, "home");
    w.set_pos(cy, home.rect.centre());
    // It earns 700 cents. The engine takes them when the time is over.
    w.give_money(cy, Money(700));
    let treasury = w.money(TREASURY);

    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(frame + 200 - now).expect("soon"));
    assert_eq!(of_kind(&events, "crime.fine_defaulted").len(), 0);
    assert!(!w.has_condition(cy, "jailed"));
    // The order leaves the registry at the frame of the deadline and
    // reaches the zone of the convict one frame later.
    let events = step_events(&mut w, 2);
    let defaults = of_kind(&events, "crime.fine_defaulted");
    assert_eq!(defaults.len(), 1);
    // 3,700 owed, 700 taken: 3,000 left. One tick for each 10 cents.
    assert_eq!(
        defaults[0].data,
        json!({ "resident": cy, "case": case, "owed": 3_000, "jail_ticks": 300 })
    );
    let told = notice(&w, cy, "crime.fine_defaulted").expect("the convict is told");
    assert_eq!(told.data, json!({ "case": case, "owed": 3_000, "jail_ticks": 300 }));
    // The engine moved the convict into the jail, for exactly 300 ticks.
    assert_eq!(site_kind_of(&w, cy).as_deref(), Some("jail"));
    assert_eq!(w.pos(cy), jail.rect.centre());
    assert_eq!(jailed_until(&w, cy), Some(told.tick.0 + 300));
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.money(TREASURY), treasury + Money(700));
    // The debt is gone: jail replaced it.
    assert_eq!(law(&w, cy).expect("in jail")["custody"]["state"], json!("jailed"));
    assert_eq!(law(&w, cy).expect("in jail").get("debts"), None);
    assert_eq!(fail(&mut w, cy, "pay_fine", json!({})), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn jail_for_a_debt_comes_after_the_sentence_and_has_a_limit() {
    // 100 ticks of jail from the judge. The debt of 3,700 would be 370
    // ticks; the limit is 250.
    let rules = [("crime.fine_due_ticks", 40), ("crime.fine_jail_max_ticks", 250)];
    let Fined { mut w, cy, .. } = fined(300, 100, &rules);
    let first = jailed_until(&w, cy).expect("the convict is in jail");
    // An inmate can pay from its cell.
    w.give_money(cy, Money(200));
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 200, "owed": 3_500 }));

    let events = step_events(&mut w, 45);
    let defaults = of_kind(&events, "crime.fine_defaulted");
    assert_eq!(defaults.len(), 1);
    assert_eq!(defaults[0].data["owed"], json!(3_500));
    assert_eq!(defaults[0].data["jail_ticks"], json!(250));
    // The new time starts when the first sentence ends.
    assert_eq!(jailed_until(&w, cy), Some(first + 250));
    w.assert_conserved();
}
