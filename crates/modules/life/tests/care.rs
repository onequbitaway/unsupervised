//! Sleep, rest, hospital care, food and medicine.

mod common;

use common::{make_owner, make_tenant, report, result, site, with_params, world_with};
use unsup_kernel::protocol::{ActionStatus, ErrorCode};
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Money, Pos, ResidentId};

/// A world without the hourly decay, so that only the verbs change needs.
fn calm() -> TestWorld<'static> {
    world_with(with_params(&[
        ("life.reflexes", 0),
        ("life.food_decay_per_hour", 0),
        ("life.energy_decay_per_hour", 0),
        ("life.mood_decay_per_hour", 0),
        ("life.health_regen_per_hour", 0),
    ]))
}

/// Puts a resident inside the first site of a kind and returns the site.
fn enter(
    w: &mut TestWorld<'_>,
    r: ResidentId,
    kind: &str,
) -> (unsup_kernel::ZoneId, unsup_kernel::Site) {
    let (zone, place) = site(w, kind);
    w.set_pos(r, place.rect.centre());
    (zone, place)
}

fn ticks_of(status: &ActionStatus) -> u32 {
    match status {
        ActionStatus::Started { ticks, .. } => *ticks,
        other => panic!("the action did not start: {other:?}"),
    }
}

/// Sleeps for two hours at the place where the resident is and returns the
/// result.
fn sleep_two_hours(w: &mut TestWorld<'_>, r: ResidentId) -> unsup_kernel::Value {
    w.set_attr(r, "need.energy", 100);
    w.set_attr(r, "need.mood", 500);
    w.act_ok(r, "sleep", json!({ "ticks": 120 }));
    assert!(w.has_condition(r, "asleep"));
    assert_eq!(w.run_until_idle(r, 200), 120);
    assert!(!w.has_condition(r, "asleep"));
    result(w, r)
}

#[test]
fn sleep_is_good_in_an_own_home_fair_in_a_shelter_and_poor_elsewhere() {
    let mut w = calm();
    let ada = w.spawn("Ada");

    // On the street: 60 energy for each hour, no mood.
    w.set_pos(ada, Pos::new(0, 0));
    let out = sleep_two_hours(&mut w, ada);
    assert_eq!(out, json!({ "ticks": 120, "quality": "poor", "energy": 220, "mood": 500 }));

    // In a shelter: 90 energy and 4 mood for each hour.
    enter(&mut w, ada, "shelter");
    let out = sleep_two_hours(&mut w, ada);
    assert_eq!(out, json!({ "ticks": 120, "quality": "fair", "energy": 280, "mood": 508 }));

    // In the home of somebody else: poor.
    let (zone, home) = enter(&mut w, ada, "home");
    let out = sleep_two_hours(&mut w, ada);
    assert_eq!(out["quality"], "poor");
    assert_eq!(out["energy"], 220);

    // In a home that the resident owns: 125 energy and 10 mood for each hour.
    make_owner(&w, zone, home.id, ada);
    let out = sleep_two_hours(&mut w, ada);
    assert_eq!(out, json!({ "ticks": 120, "quality": "good", "energy": 350, "mood": 520 }));

    // In an apartment that the resident rents: good too.
    let (zone, flat) = enter(&mut w, ada, "apartment");
    make_tenant(&w, zone, flat.id, ada);
    let out = sleep_two_hours(&mut w, ada);
    assert_eq!(out["quality"], "good");
    assert_eq!(out["energy"], 350);

    // Owning a shop does not make it a bed.
    let (zone, shop) = enter(&mut w, ada, "shop");
    make_owner(&w, zone, shop.id, ada);
    assert_eq!(sleep_two_hours(&mut w, ada)["quality"], "poor");
}

#[test]
fn sleep_without_a_length_lasts_until_the_energy_is_full() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    enter(&mut w, ada, "shelter");
    // 450 points at 90 for each hour: five hours.
    w.set_attr(ada, "need.energy", 550);
    let status = w.act_ok(ada, "sleep", json!({}));
    assert_eq!(ticks_of(&status), 300);
    w.run_until_idle(ada, 400);
    assert_eq!(w.attr(ada, "need.energy"), 1_000);

    // Almost full: the shortest sleep. Empty on the street: the longest.
    w.set_attr(ada, "need.energy", 999);
    assert_eq!(ticks_of(&w.act_ok(ada, "sleep", json!({}))), 30);
    w.run_until_idle(ada, 40);
    w.set_pos(ada, Pos::new(0, 0));
    w.set_attr(ada, "need.energy", 0);
    assert_eq!(ticks_of(&w.act_ok(ada, "sleep", json!({}))), 720);
}

#[test]
fn sleep_refuses_a_rested_resident_and_a_wrong_length() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    assert_eq!(w.act_err(ada, "sleep", json!({})).code, ErrorCode::Conflict);
    w.set_attr(ada, "need.energy", 500);
    for ticks in [0, 29, 721] {
        assert_eq!(w.act_err(ada, "sleep", json!({ "ticks": ticks })).code, ErrorCode::InvalidArgs);
    }
    assert!(!w.has_condition(ada, "asleep"));
    assert_eq!(w.activity(ada), None);
}

#[test]
fn a_sleep_that_is_cut_short_gives_the_energy_of_the_time_slept() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    enter(&mut w, ada, "shelter");
    w.set_attr(ada, "need.energy", 100);
    w.act_ok(ada, "sleep", json!({ "ticks": 480 }));
    w.step(100);
    assert_eq!(w.attr(ada, "need.energy"), 100);
    w.act_ok(ada, "cancel", json!({}));
    // 100 ticks at 90 for each hour.
    assert_eq!(w.attr(ada, "need.energy"), 250);
    assert!(!w.has_condition(ada, "asleep"));
    assert_eq!(report(&w, ada).error.expect("an error").code, ErrorCode::Interrupted);
    // The points are not given twice.
    w.step(500);
    assert_eq!(w.attr(ada, "need.energy"), 250);
}

#[test]
fn rest_gives_a_little_energy_and_mood_and_more_mood_in_a_park() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    w.set_attr(ada, "need.energy", 500);
    w.set_attr(ada, "need.mood", 500);
    w.set_pos(ada, Pos::new(0, 0));
    w.act_ok(ada, "rest", json!({ "ticks": 90 }));
    assert_eq!(w.run_until_idle(ada, 100), 90);
    // 20 energy and 20 mood for each hour.
    assert_eq!(result(&w, ada), json!({ "ticks": 90, "energy": 530, "mood": 530 }));

    // In a park: 40 mood for each hour. A park has a pond; stand beside it.
    let (_, park) = site(&w, "park");
    w.set_pos(ada, Pos::new(park.rect.x0, park.rect.y0));
    w.act_ok(ada, "rest", json!({ "ticks": 60 }));
    w.run_until_idle(ada, 100);
    assert_eq!(result(&w, ada), json!({ "ticks": 60, "energy": 550, "mood": 570 }));

    for ticks in [0, 241] {
        assert_eq!(w.act_err(ada, "rest", json!({ "ticks": ticks })).code, ErrorCode::InvalidArgs);
    }
}

#[test]
fn a_hospital_heals_for_a_fee_that_goes_to_its_till() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    w.give_money(ada, Money(50_000));
    w.set_attr(ada, "need.health", 400);

    // Not in a hospital.
    assert_eq!(w.act_err(ada, "heal", json!({})).code, ErrorCode::NotHere);
    let (_, hospital) = enter(&mut w, ada, "hospital");
    let till = Holder::Site(hospital.id);

    // 600 points at 20 cents: 12,000 cents. At 150 for each hour: 240 ticks.
    let status = w.act_ok(ada, "heal", json!({}));
    assert_eq!(ticks_of(&status), 240);
    assert_eq!(w.cash(ada), Money(38_000));
    assert_eq!(w.money(till), Money(12_000));
    assert!(w.has_condition(ada, "hospitalized"));
    // The stay blocks other verbs of the resident.
    assert_eq!(w.act_err(ada, "give", json!({ "to": 99, "money": 1 })).code, ErrorCode::Blocked);
    w.step(239);
    assert_eq!(w.attr(ada, "need.health"), 400);
    w.step(1);
    assert_eq!(w.attr(ada, "need.health"), 1_000);
    assert!(!w.has_condition(ada, "hospitalized"));
    assert_eq!(result(&w, ada), json!({ "ticks": 240, "healed": 600, "health": 1_000 }));
    w.assert_conserved();

    // Healthy: refused. Nothing is paid.
    assert_eq!(w.act_err(ada, "heal", json!({})).code, ErrorCode::Conflict);
    assert_eq!(w.money(till), Money(12_000));
}

#[test]
fn heal_can_buy_a_part_and_needs_the_cash() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    enter(&mut w, ada, "hospital");
    w.set_attr(ada, "need.health", 100);
    w.give_money(ada, Money(2_000));
    // Full healing costs 18,000 cents.
    let error = w.act_err(ada, "heal", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 18_000, "available": 2_000 }));
    assert!(!w.has_condition(ada, "hospitalized"));
    assert_eq!(w.act_err(ada, "heal", json!({ "points": 0 })).code, ErrorCode::InvalidArgs);
    // 100 points cost 2,000 cents and take 40 ticks.
    let status = w.act_ok(ada, "heal", json!({ "points": 100 }));
    assert_eq!(ticks_of(&status), 40);
    assert_eq!(w.cash(ada), Money::ZERO);
    w.run_until_idle(ada, 50);
    assert_eq!(w.attr(ada, "need.health"), 200);
}

#[test]
fn a_patient_that_leaves_early_keeps_what_was_healed_and_gets_no_money_back() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    enter(&mut w, ada, "hospital");
    w.set_attr(ada, "need.health", 400);
    w.give_money(ada, Money(12_000));
    w.act_ok(ada, "heal", json!({}));
    w.step(100);
    w.act_ok(ada, "cancel", json!({}));
    // 100 ticks at 150 for each hour.
    assert_eq!(w.attr(ada, "need.health"), 650);
    assert!(!w.has_condition(ada, "hospitalized"));
    assert_eq!(w.cash(ada), Money::ZERO);
    w.assert_conserved();
}

#[test]
fn eat_takes_one_unit_of_food_and_gives_its_nutrition() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    w.give_item(ada, "bread", 2);
    w.give_item(ada, "apple", 1);
    w.give_item(ada, "feast", 1);
    w.give_item(ada, "stone", 1);
    let me = Holder::Resident(ada);
    w.set_attr(ada, "need.food", 500);
    w.set_attr(ada, "need.mood", 500);

    let status = w.act_ok(ada, "eat", json!({ "item": "bread" }));
    assert_eq!(ticks_of(&status), 10);
    w.step(9);
    assert_eq!(w.qty(me, "bread"), 2);
    w.step(1);
    assert_eq!(w.qty(me, "bread"), 1);
    assert_eq!(result(&w, ada), json!({ "item": "bread", "nutrition": 300, "food": 800 }));
    assert_eq!(w.attr(ada, "need.mood"), 505);

    // Without a name: the food that fits the hunger best. Hunger is 200:
    // the apple (100) fits, the bread (300) and the feast (900) do not.
    w.act_ok(ada, "eat", json!({}));
    w.run_until_idle(ada, 20);
    assert_eq!(result(&w, ada)["item"], "apple");
    assert_eq!(w.attr(ada, "need.food"), 900);
    // Hunger is 100 and nothing fits: the smallest food, the bread.
    w.act_ok(ada, "eat", json!({}));
    w.run_until_idle(ada, 20);
    assert_eq!(result(&w, ada)["item"], "bread");
    assert_eq!(w.attr(ada, "need.food"), 1_000);
    // Very hungry: the feast.
    w.set_attr(ada, "need.food", 50);
    w.act_ok(ada, "eat", json!({}));
    w.run_until_idle(ada, 20);
    assert_eq!(result(&w, ada)["item"], "feast");
    assert_eq!(w.attr(ada, "need.food"), 950);

    // The food left the world through the sink of eaten food.
    let totals = w.assert_conserved();
    let bread = w.world.blueprint().item_id("bread").expect("bread");
    let eaten = totals.items.get(&bread).expect("bread totals");
    assert_eq!((eaten.produced, eaten.consumed, eaten.held), (2, 2, 0));
}

#[test]
fn eat_refusals() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    w.give_item(ada, "stone", 1);
    w.set_attr(ada, "need.food", 500);
    assert_eq!(w.act_err(ada, "eat", json!({})).code, ErrorCode::Insufficient);
    assert_eq!(w.act_err(ada, "eat", json!({ "item": "bread" })).code, ErrorCode::Insufficient);
    assert_eq!(w.act_err(ada, "eat", json!({ "item": "stone" })).code, ErrorCode::NotAllowed);
    assert_eq!(w.act_err(ada, "eat", json!({ "item": "gold" })).code, ErrorCode::NotFound);
    w.give_item(ada, "bread", 1);
    w.set_attr(ada, "need.food", 1_000);
    assert_eq!(w.act_err(ada, "eat", json!({ "item": "bread" })).code, ErrorCode::Conflict);

    // Food that is gone when the meal ends: the meal fails.
    w.set_attr(ada, "need.food", 500);
    let bob = w.spawn_at("Bob", w.pos(ada));
    w.act_ok(ada, "eat", json!({ "item": "bread" }));
    let bread = w.world.blueprint().item_id("bread").expect("bread");
    w.with_resident(ada, |realm, _| {
        realm
            .ledger_mut()
            .transfer_item(Holder::Resident(ada), Holder::Resident(bob), bread, 1, "theft")
            .expect("the bread moves");
    });
    w.run_until_idle(ada, 20);
    let failed = report(&w, ada);
    assert_eq!(failed.error.expect("an error").code, ErrorCode::Insufficient);
    assert_eq!(w.attr(ada, "need.food"), 500);
}

#[test]
fn medicine_gives_health_back() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    w.give_item(ada, "bandage", 2);
    w.give_item(ada, "bread", 1);
    w.set_attr(ada, "need.health", 700);
    let status = w.act_ok(ada, "use_item", json!({ "item": "bandage" }));
    assert_eq!(ticks_of(&status), 5);
    w.run_until_idle(ada, 10);
    assert_eq!(result(&w, ada), json!({ "item": "bandage", "healing": 200, "health": 900 }));
    assert_eq!(w.qty(Holder::Resident(ada), "bandage"), 1);
    w.assert_conserved();

    assert_eq!(w.act_err(ada, "use_item", json!({ "item": "bread" })).code, ErrorCode::NotAllowed);
    assert_eq!(w.act_err(ada, "use_item", json!({ "item": "gold" })).code, ErrorCode::NotFound);
    assert_eq!(w.act_err(ada, "use_item", json!({})).code, ErrorCode::InvalidArgs);
    w.set_attr(ada, "need.health", 1_000);
    assert_eq!(w.act_err(ada, "use_item", json!({ "item": "bandage" })).code, ErrorCode::Conflict);
    w.set_attr(ada, "need.health", 500);
    let bob = w.spawn("Bob");
    w.set_attr(bob, "need.health", 500);
    assert_eq!(
        w.act_err(bob, "use_item", json!({ "item": "bandage" })).code,
        ErrorCode::Insufficient
    );
}

#[test]
fn a_patient_that_something_moves_out_of_the_hospital_is_not_healed() {
    let mut w = calm();
    let ada = w.spawn("Ada");
    enter(&mut w, ada, "hospital");
    w.set_attr(ada, "need.health", 400);
    w.give_money(ada, Money(12_000));
    let status = w.act_ok(ada, "heal", json!({}));
    assert_eq!(ticks_of(&status), 240);
    w.step(100);
    // Something takes the patient out and does not stop the stay.
    w.set_pos(ada, Pos::new(0, 0));
    w.run_until_idle(ada, 200);
    assert_eq!(result(&w, ada)["healed"], 0);
    assert_eq!(w.attr(ada, "need.health"), 400);
    assert!(!w.has_condition(ada, "hospitalized"));
    w.assert_conserved();
}
