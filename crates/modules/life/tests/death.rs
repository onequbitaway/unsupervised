//! Death and estates: starvation, the heir, the spouse and the city.

mod common;

use common::{treasury, with_params, world, world_with};
use unsup_kernel::protocol::{ActionStatus, ErrorCode, Event};
use unsup_kernel::serde_json::json;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Kin, Money, Pos, RealmId, ResidentId, UNCLAIMED};
use unsup_life::{Estates, die};

const HOUR: u32 = 60;

/// Makes a resident so weak that the next hourly pass kills it: no food and
/// less health than one hour without food costs.
fn starve(w: &mut TestWorld<'_>, r: ResidentId) {
    w.set_attr(r, "need.food", 0);
    w.set_attr(r, "need.health", 15);
}

/// Gives a resident an estate: cash and goods.
fn enrich(w: &mut TestWorld<'_>, r: ResidentId) {
    w.give_money(r, Money(12_345));
    w.give_item(r, "bread", 3);
    w.give_item(r, "stone", 2);
}

/// Runs one game hour and returns the events of the life module about
/// death.
fn hour(w: &mut TestWorld<'_>) -> Vec<Event> {
    w.step(HOUR)
        .into_iter()
        .flat_map(|frame| frame.events)
        .filter(|event| event.kind == "life.resident_died" || event.kind == "life.estate_settled")
        .collect()
}

fn holdings(w: &TestWorld<'_>, holder: Holder) -> (i64, i64, i64) {
    (w.money(holder).cents(), w.qty(holder, "bread"), w.qty(holder, "stone"))
}

#[test]
fn starvation_kills_and_the_estate_goes_to_the_heir_of_the_will() {
    let mut w = world();
    let ada = w.spawn("Ada");
    // The heir stands in another zone.
    let bob = w.spawn_at("Bob", Pos::new(100, 100));
    assert_ne!(w.zone_of(ada), w.zone_of(bob));
    enrich(&mut w, ada);
    assert_eq!(
        w.act_ok(ada, "write_will", json!({ "heir": bob })),
        ActionStatus::Done { result: json!({ "heir": bob }) }
    );
    let before = w.assert_conserved();
    starve(&mut w, ada);

    let events = hour(&mut w);
    assert!(!w.is_alive(ada));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.actor, Some(ada));
    assert_eq!(died.drama, 80);
    assert_eq!(
        died.data,
        json!({
            "resident": ada,
            "cause": "starvation",
            "age": 18,
            "heir": { "Resident": bob },
            "estate": 12_345,
        })
    );
    let settled = events.iter().find(|e| e.kind == "life.estate_settled").expect("a settlement");
    assert_eq!(settled.data["heir"], json!({ "Resident": bob }));
    assert_eq!(settled.data["money"], json!(12_345));
    assert_eq!(settled.data["items"], json!({ "bread": 3, "stone": 2 }));

    w.step(2);
    assert_eq!(holdings(&w, Holder::Resident(bob)), (12_345, 3, 2));
    assert_eq!(holdings(&w, treasury(0)), (0, 0, 0));
    assert_eq!(w.money(UNCLAIMED), Money::ZERO);
    // Bob learns about it.
    let notice =
        w.notices(bob).into_iter().find(|n| n.kind == "life.inheritance").expect("a notice");
    assert_eq!(notice.from, Some(ada));
    assert_eq!(
        notice.data,
        json!({ "from": ada, "money": 12_345, "items": { "bread": 3, "stone": 2 } })
    );
    // Nothing was made or lost.
    let after = w.assert_conserved();
    assert_eq!(after.money.held, before.money.held);
    assert_eq!(after.money.held, 12_345);
    let global = w.world.realm(RealmId::Global).expect("the global realm");
    assert!(global.res::<Estates>().open().is_empty());
}

#[test]
fn without_a_will_the_spouse_inherits() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    enrich(&mut w, ada);
    w.with_resident(ada, |realm, idx| realm.col_mut::<Kin>()[idx].spouse = Some(bob));
    starve(&mut w, ada);
    let events = hour(&mut w);
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["heir"], json!({ "Resident": bob }));
    w.step(2);
    assert_eq!(holdings(&w, Holder::Resident(bob)), (12_345, 3, 2));
    w.assert_conserved();
}

#[test]
fn the_will_comes_before_the_spouse() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    let cyd = w.spawn("Cyd");
    enrich(&mut w, ada);
    w.with_resident(ada, |realm, idx| realm.col_mut::<Kin>()[idx].spouse = Some(bob));
    w.act_ok(ada, "write_will", json!({ "heir": cyd }));
    starve(&mut w, ada);
    hour(&mut w);
    w.step(2);
    assert_eq!(holdings(&w, Holder::Resident(cyd)), (12_345, 3, 2));
    assert_eq!(holdings(&w, Holder::Resident(bob)), (0, 0, 0));
    w.assert_conserved();
}

#[test]
fn without_an_heir_the_city_inherits() {
    let mut w = world();
    let ada = w.spawn("Ada");
    enrich(&mut w, ada);
    starve(&mut w, ada);
    let events = hour(&mut w);
    assert!(!w.is_alive(ada));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["heir"], json!({ "Org": 100 }));
    assert_eq!(died.data["estate"], json!(12_345));
    w.step(1);
    assert_eq!(holdings(&w, treasury(0)), (12_345, 3, 2));
    let totals = w.assert_conserved();
    assert_eq!(totals.money.held, 12_345);
}

#[test]
fn an_heir_that_died_first_does_not_inherit_and_the_city_does() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    enrich(&mut w, ada);
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    w.despawn(bob);
    w.step(1);
    starve(&mut w, ada);
    let events = hour(&mut w);
    let settled = events.iter().find(|e| e.kind == "life.estate_settled").expect("a settlement");
    assert_eq!(settled.data["heir"], json!({ "Org": 100 }));
    w.step(2);
    assert_eq!(holdings(&w, treasury(0)), (12_345, 3, 2));
    assert_eq!(holdings(&w, UNCLAIMED), (0, 0, 0));
    w.assert_conserved();
}

#[test]
fn two_residents_that_name_each_other_and_die_together_leave_all_to_the_city() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    enrich(&mut w, ada);
    enrich(&mut w, bob);
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    w.act_ok(bob, "write_will", json!({ "heir": ada }));
    starve(&mut w, ada);
    starve(&mut w, bob);
    w.step(HOUR + 3);
    assert!(!w.is_alive(ada) && !w.is_alive(bob));
    assert_eq!(holdings(&w, treasury(0)), (24_690, 6, 4));
    assert_eq!(holdings(&w, UNCLAIMED), (0, 0, 0));
    w.assert_conserved();
}

#[test]
fn a_resident_with_nothing_leaves_no_open_estate() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    starve(&mut w, ada);
    w.step(HOUR + 5);
    assert!(!w.is_alive(ada));
    let global = w.world.realm(RealmId::Global).expect("the global realm");
    assert!(global.res::<Estates>().open().is_empty());
    assert!(w.notices(bob).iter().all(|n| n.kind != "life.inheritance"));
    w.assert_conserved();
}

#[test]
fn health_that_another_module_took_kills_at_the_next_pass() {
    let mut w = world_with(with_params(&[("life.reflexes", 0)]));
    let ada = w.spawn("Ada");
    w.set_attr(ada, "need.health", 0);
    let events = hour(&mut w);
    assert!(!w.is_alive(ada));
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["cause"], json!("injury"));
}

#[test]
fn die_is_a_public_function_and_a_second_call_does_nothing() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.give_money(ada, Money(500));
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    w.with_resident(ada, |realm, idx| {
        die(realm, idx, "duel");
        die(realm, idx, "duel");
    });
    let frames = w.step(3);
    let deaths: Vec<&Event> = frames
        .iter()
        .flat_map(|frame| frame.events.iter())
        .filter(|event| event.kind == "life.resident_died")
        .collect();
    assert_eq!(deaths.len(), 1);
    assert_eq!(deaths[0].data["cause"], json!("duel"));
    assert!(!w.is_alive(ada));
    assert_eq!(w.cash(bob), Money(500));
    w.assert_conserved();
}

#[test]
fn residents_near_a_death_get_a_notice() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let pos = w.pos(ada);
    let bob = w.spawn_at("Bob", Pos::new(pos.x + 2, pos.y));
    starve(&mut w, ada);
    w.step(HOUR);
    let notice =
        w.notices(bob).into_iter().find(|n| n.kind == "life.resident_died").expect("a notice");
    assert_eq!(notice.data["resident"], json!(ada));
}

#[test]
fn a_will_can_name_nobody_but_not_its_writer() {
    let mut w = world();
    let ada = w.spawn("Ada");
    let bob = w.spawn("Bob");
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    assert_eq!(w.with_resident(ada, |realm, idx| realm.col::<Kin>()[idx].heir), Some(bob));
    w.act_ok(ada, "write_will", json!({}));
    assert_eq!(w.with_resident(ada, |realm, idx| realm.col::<Kin>()[idx].heir), None);
    let error = w.act_err(ada, "write_will", json!({ "heir": ada }));
    assert_eq!(error.code, ErrorCode::InvalidArgs);
    // The verb is free: it works while the resident is busy.
    w.act_ok(ada, "wait", json!({ "ticks": 30 }));
    w.step(1);
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    assert_eq!(w.with_resident(ada, |realm, idx| realm.col::<Kin>()[idx].heir), Some(bob));
}

#[test]
fn a_resident_that_dies_when_it_arrives_in_a_zone_leaves_its_estate_to_the_heir() {
    let mut w = world_with(with_params(&[("life.reflexes", 0)]));
    // Ada walks from zone 0 to zone 1. She owes an hour for her needs, and
    // pays it when she arrives. Bob, her heir, is in a third zone.
    let ada = w.spawn_at("Ada", Pos::new(62, 0));
    let bob = w.spawn_at("Bob", Pos::new(0, 100));
    enrich(&mut w, ada);
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    w.step(1);
    starve(&mut w, ada);
    let ack = w.act(ada, "move_to", json!({ "x": 66, "y": 0 }));
    assert_eq!(ack.status, ActionStatus::Pending);

    let frames = w.step(6);
    assert!(!w.is_alive(ada));
    let events: Vec<&Event> = frames.iter().flat_map(|frame| frame.events.iter()).collect();
    let died = events.iter().find(|e| e.kind == "life.resident_died").expect("a death");
    assert_eq!(died.data["cause"], json!("starvation"));
    assert_eq!(died.data["heir"], json!({ "Resident": bob }));
    // She died in zone 1, two steps after the border at the most.
    let place = died.pos.expect("a place");
    assert!((64..=65).contains(&place.x), "{place}");
    let settled = events.iter().find(|e| e.kind == "life.estate_settled").expect("a settlement");
    assert_eq!(settled.data["complete"], json!(true));

    w.step(2);
    assert_eq!(holdings(&w, Holder::Resident(bob)), (12_345, 3, 2));
    let global = w.world.realm(RealmId::Global).expect("the global realm");
    assert!(global.res::<Estates>().open().is_empty());
    w.assert_conserved();
}

#[test]
fn an_estate_with_nothing_in_it_waits_as_long_as_the_data_file_says() {
    let mut w = world_with(with_params(&[("life.estate_wait_ticks", 30)]));
    let ada = w.spawn_at("Ada", Pos::new(0, 0));
    let bob = w.spawn("Bob");
    w.act_ok(ada, "write_will", json!({ "heir": bob }));
    starve(&mut w, ada);
    // Zone 0 has its hourly pass in frame 60.
    w.step(HOUR + 10);
    assert!(!w.is_alive(ada));
    let open = |w: &TestWorld<'_>| {
        w.world.realm(RealmId::Global).expect("the global realm").res::<Estates>().open().len()
    };
    assert_eq!(open(&w), 1);
    w.step(20);
    assert_eq!(open(&w), 0);
}
