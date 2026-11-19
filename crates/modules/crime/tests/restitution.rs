//! A restitution to a site: where the money goes when the site can be
//! reached, and when it cannot.
//!
//! A site without an owner is its own victim, so a burglar owes the
//! restitution to the site. The debt names the zone of the site. These
//! tests take the site away, or the zone out of the debt, and check that
//! the money then goes to the city treasury. The debt must not turn into
//! jail time while the convict has the cash.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_crime::Record;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Money, Pos, ResidentId, Site};

/// Returns a tile in another zone than a site: the middle of the first
/// such zone.
fn far_from(w: &TestWorld<'_>, place: &Site) -> Pos {
    let home = zone_at(w, place.rect.centre());
    let zones = w.world.layout().zones();
    let other = zones.iter().find(|zone| zone.id != home).expect("the city has more zones");
    other.bounds.centre()
}

/// A world in which a judge just sentenced a burglar.
struct Sentenced {
    w: TestWorld<'static>,
    cy: ResidentId,
    shop: Site,
    case: u64,
    /// Frame of the verdict.
    frame: u64,
}

/// Cy takes 10 units of grain (2,000 cents) out of a shop without an
/// owner and has no cash. The judge gives a restitution of 1,500 cents to
/// the shop and a fine of 1,000 cents, without jail time. Cy is free and
/// owes 2,500 cents.
fn sentenced(rules: &[(&'static str, i64)]) -> Sentenced {
    let mut w = town(&with(&[&[ALWAYS, WATCH_SEES_ALL], &BLIND, rules]));
    let shop = site(&w, "shop");
    let zone = zone_at(&w, shop.rect.centre());
    w.zone(zone).sites_mut().get_mut(shop.id).expect("the shop").owner = None;
    w.world.settle();
    fill_stock(&mut w, &shop, "grain", 20);
    let eve = resident_at(&mut w, "Eve", 0, shop.rect.centre());
    appoint(&mut w, "police", &["law.arrest"], &[eve]);
    let finn = resident_at(&mut w, "Finn", 0, Pos::new(80, 80));
    appoint(&mut w, "judge", &["law.judge"], &[finn]);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let case = case_of(&ok(&mut w, cy, "burgle", json!({ "item": "grain" })));
    w.step(1);
    ok(&mut w, eve, "arrest", json!({ "target": cy }));
    ok(&mut w, eve, "charge", json!({ "target": cy, "case": case }));
    let ruling = json!({
        "case": case, "guilty": true, "fine": 1_000, "restitution": 1_500, "jail_ticks": 0,
    });
    let (_, mut events) = run_with_events(&mut w, finn, "rule_case", ruling);
    events.extend(step_events(&mut w, 1));
    let frame = frame_of(&events, "crime.verdict_given");
    assert_eq!(w.cash(cy), Money(0));
    let debts = w.with_resident(cy, |realm, idx| realm.col::<Record>()[idx].debts().to_vec());
    let owed: Vec<(Holder, i64, bool)> =
        debts.iter().map(|debt| (debt.to, debt.amount, debt.fine)).collect();
    assert_eq!(owed, [(Holder::Site(shop.id), 1_500, false), (TREASURY, 1_000, true)]);
    assert_eq!(debts[0].zone, Some(zone), "the debt names the zone of the shop");
    assert!(!w.has_condition(cy, "jailed"));
    Sentenced { w, cy, shop, case, frame }
}

/// Changes the restitution in the record of a resident as JSON. `z` is
/// the zone of the site in the debt.
fn rewrite_restitution(w: &mut TestWorld<'_>, r: ResidentId, change: impl Fn(&mut Value)) {
    w.with_resident(r, |realm, idx| {
        let mut record =
            serde_json::to_value(&realm.col::<Record>()[idx]).expect("a record is JSON");
        let debts = record["d"].as_array_mut().expect("the record has debts");
        let restitution =
            debts.iter_mut().find(|debt| debt["f"] == json!(false)).expect("a restitution");
        change(restitution);
        realm.col_mut::<Record>()[idx] = serde_json::from_value(record).expect("a record");
    });
    w.world.settle();
}

#[test]
fn a_restitution_reaches_a_site_in_another_zone() {
    let Sentenced { mut w, cy, shop, .. } = sentenced(&[]);
    let treasury = w.money(TREASURY);
    let far = far_from(&w, &shop);
    w.set_pos(cy, far);
    assert_ne!(w.zone_of(cy), zone_at(&w, shop.rect.centre()));
    w.give_money(cy, Money(1_500));
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 1_500, "owed": 1_000 }));
    w.step(1);
    // The restitution comes first, and it goes to the till of the shop.
    assert_eq!(w.money(Holder::Site(shop.id)), Money(1_500));
    assert_eq!(w.money(TREASURY), treasury);
    w.assert_conserved();
}

#[test]
fn a_restitution_to_a_site_that_is_gone_goes_to_the_city_treasury() {
    let Sentenced { mut w, cy, shop, .. } = sentenced(&[]);
    let zone = zone_at(&w, shop.rect.centre());
    // The shop is pulled down. Cy stands where it was.
    w.zone(zone).sites_mut().remove(shop.id).expect("the shop was there");
    w.world.settle();
    w.set_pos(cy, shop.rect.centre());
    let treasury = w.money(TREASURY);

    w.give_money(cy, Money(1_500));
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 1_500, "owed": 1_000 }));
    w.step(1);
    // No money goes to a site that nobody can use any more.
    assert_eq!(w.money(Holder::Site(shop.id)), Money(0));
    assert_eq!(w.money(TREASURY), treasury + Money(1_500));
    w.assert_conserved();
}

#[test]
fn a_restitution_to_a_site_in_an_unknown_zone_does_not_become_jail_time() {
    let Sentenced { mut w, cy, shop, frame, .. } = sentenced(&[("crime.fine_due_ticks", 200)]);
    // The debt names a zone that this world does not have. No wire can go
    // there.
    rewrite_restitution(&mut w, cy, |debt| debt["z"] = json!(9_999));
    let far = far_from(&w, &shop);
    w.set_pos(cy, far);
    let treasury = w.money(TREASURY);

    // Cy has the cash for all of it when the time to pay is over.
    w.give_money(cy, Money(4_000));
    let now = w.world.frame();
    let events = step_events(&mut w, u32::try_from(frame + 202 - now).expect("soon"));
    assert_eq!(of_kind(&events, "crime.fine_defaulted").len(), 0, "nothing is left to default on");
    assert!(!w.has_condition(cy, "jailed"));
    assert_eq!(law(&w, cy), None, "the debts are paid");
    assert_eq!(w.cash(cy), Money(1_500));
    w.step(1);
    assert_eq!(w.money(TREASURY), treasury + Money(2_500));
    assert_eq!(w.money(Holder::Site(shop.id)), Money(0));
    w.assert_conserved();
}

#[test]
fn a_restitution_to_a_site_without_a_zone_goes_to_the_site_only_where_the_site_is() {
    // The debt does not say where the site is.
    let Sentenced { mut w, cy, shop, case, .. } = sentenced(&[]);
    rewrite_restitution(&mut w, cy, |debt| {
        debt.as_object_mut().expect("a debt is a map").remove("z");
    });
    let treasury = w.money(TREASURY);
    let far = far_from(&w, &shop);
    let far_zone = zone_at(&w, far);

    // In another zone, the zone of Cy has no such site: the treasury gets
    // the money. Nothing stays in the ledger of that zone under the name
    // of the site.
    w.set_pos(cy, far);
    w.give_money(cy, Money(500));
    assert_eq!(
        ok(&mut w, cy, "pay_fine", json!({ "amount": 500 })),
        json!({ "paid": 500, "owed": 2_000 })
    );
    w.step(1);
    assert_eq!(w.money(TREASURY), treasury + Money(500));
    assert_eq!(w.zone(far_zone).ledger().money(Holder::Site(shop.id)), Money(0));

    // In the zone of the shop, the shop gets the rest of the restitution.
    w.set_pos(cy, shop.rect.centre());
    w.give_money(cy, Money(1_000));
    assert_eq!(ok(&mut w, cy, "pay_fine", json!({})), json!({ "paid": 1_000, "owed": 1_000 }));
    w.step(1);
    assert_eq!(w.money(Holder::Site(shop.id)), Money(1_000));
    assert_eq!(w.money(TREASURY), treasury + Money(500));
    let debts = w.with_resident(cy, |realm, idx| realm.col::<Record>()[idx].debts().to_vec());
    assert_eq!(debts.len(), 1, "only the fine of case {case} is left");
    assert!(debts[0].fine);
    w.assert_conserved();
}
