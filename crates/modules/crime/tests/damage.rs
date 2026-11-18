//! `vandalize` and `repair`: destroyed stock goes through a sink, the
//! value of the site falls until somebody repairs it.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

/// A tile on a street. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn vandalism_destroys_stock_and_lowers_the_value_until_a_repair() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let shop = site(&w, "shop");
    let value = shop.attr("value");
    assert_eq!(value, 5_450_000);
    fill_stock(&mut w, &shop, "bread", 8);
    fill_stock(&mut w, &shop, "grain", 20);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let stock = Holder::Site(shop.id);

    let (report, events) = run_with_events(&mut w, cy, "vandalize", json!({}));
    assert!(report.ok);
    let case = case_of(&report.result);
    // A quarter of each good, rounded up. Half a percent of the value.
    let loss = value * 50 / 10_000;
    assert_eq!(loss, 27_250);
    let total = 2 * 800 + 5 * 200 + loss;
    assert_eq!(
        report.result,
        json!({
            "success": true, "case": case, "witnesses": 0, "site": shop.id,
            "destroyed": { "bread": 2, "grain": 5 }, "damage": loss, "value": total,
        })
    );
    assert_eq!((w.qty(stock, "bread"), w.qty(stock, "grain")), (6, 15));
    let now = site_now(&w, shop.id);
    assert_eq!((now.attr("value"), now.attr("damage")), (value - loss, loss));
    assert_eq!(w.attr(cy, "skill.stealth"), 12);

    // The goods left the world through the sink `destruction`.
    let blueprint = w.world.blueprint();
    let sink = blueprint.sink_id("destruction").expect("the sink");
    let (bread, grain) =
        (blueprint.item_id("bread").expect("bread"), blueprint.item_id("grain").expect("grain"));
    let zone = zone_at(&w, shop.rect.centre());
    assert_eq!(w.zone(zone).ledger().consumed_by(sink, bread), 2);
    assert_eq!(w.zone(zone).ledger().consumed_by(sink, grain), 5);

    let vandalism = of_kind(&events, "crime.vandalism_committed");
    assert_eq!(vandalism.len(), 1);
    assert_eq!(vandalism[0].actor, None);
    assert_eq!(
        vandalism[0].data,
        json!({ "case": case, "city": 0, "site": shop.id, "damage": total, "attempt": false })
    );

    // Somebody repairs a part. The cost is the value that comes back.
    let bo = resident_in(&mut w, "Bo", 30_000, &shop);
    let (report, events) = run_with_events(&mut w, bo, "repair", json!({ "amount": 10_000 }));
    assert_eq!(
        report.result,
        json!({ "site": shop.id, "repaired": 10_000, "cost": 10_000, "damage": loss - 10_000 })
    );
    assert_eq!(w.cash(bo), Money(20_000));
    assert_eq!(w.attr(bo, "skill.labor"), 6);
    let now = site_now(&w, shop.id);
    assert_eq!((now.attr("value"), now.attr("damage")), (value - loss + 10_000, loss - 10_000));
    let repaired = of_kind(&events, "crime.site_repaired");
    assert_eq!(repaired.len(), 1);
    assert_eq!(
        repaired[0].data,
        json!({ "site": shop.id, "by": bo, "value": 10_000, "cost": 10_000 })
    );
    // The money left the world through the sink `repair.materials`.
    let sink = w.world.blueprint().sink_id("repair.materials").expect("the sink");
    assert_eq!(w.zone(zone).ledger().burned_by(sink), Money(10_000));

    // The rest costs 17,250. Bo has 20,000.
    let result = ok(&mut w, bo, "repair", json!({}));
    assert_eq!(result["repaired"], json!(17_250));
    assert_eq!(result["damage"], json!(0));
    assert_eq!(w.cash(bo), Money(2_750));
    let now = site_now(&w, shop.id);
    assert_eq!(now.attr("value"), value);
    assert!(!now.attrs.contains_key("damage"));
    assert_eq!(fail(&mut w, bo, "repair", json!({})), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn vandalism_stops_at_the_damage_limit() {
    // Each act takes 4 percent. The limit is 10 percent.
    let rules = [
        ALWAYS,
        NO_WATCH,
        ("crime.vandalize_value_bp", 400),
        ("crime.vandalize_damage_max_bp", 1_000),
    ];
    let mut w = town(&with(&[&rules, &BLIND]));
    let shop = site(&w, "shop");
    let value = shop.attr("value");
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let damages: Vec<i64> =
        (0..3).map(|_| num(&ok(&mut w, cy, "vandalize", json!({})), "damage")).collect();
    assert_eq!(damages, [value * 4 / 100, value * 4 / 100, value * 2 / 100]);
    assert_eq!(site_now(&w, shop.id).attr("damage"), value / 10);
    // The stock is empty and the limit is reached: nothing is left.
    assert_eq!(fail(&mut w, cy, "vandalize", json!({})), ErrorCode::Conflict);
    w.assert_conserved();
}

#[test]
fn vandalize_and_repair_refuse_bad_requests() {
    let mut w = town(&with(&[&[NEVER, NO_WATCH], &BLIND]));
    let shop = site(&w, "shop");
    fill_stock(&mut w, &shop, "bread", 8);
    let cy = resident_at(&mut w, "Cy", 100, STREET);
    assert_eq!(fail(&mut w, cy, "vandalize", json!({})), ErrorCode::NotHere);
    assert_eq!(fail(&mut w, cy, "repair", json!({})), ErrorCode::NotHere);

    w.set_pos(cy, shop.rect.centre());
    assert_eq!(fail(&mut w, cy, "repair", json!({})), ErrorCode::Conflict);
    assert_eq!(fail(&mut w, cy, "repair", json!({ "amount": 0 })), ErrorCode::InvalidArgs);

    // The attempt fails: nothing is destroyed, and there is a case.
    let result = ok(&mut w, cy, "vandalize", json!({}));
    assert_eq!(result["success"], json!(false));
    assert_eq!(result["destroyed"], json!({}));
    assert_eq!(result["damage"], json!(0));
    assert_eq!(w.qty(Holder::Site(shop.id), "bread"), 8);
    assert_eq!(site_now(&w, shop.id).attr("value"), shop.attr("value"));
    assert_eq!(case_view(&mut w, cy, case_of(&result))["kind"], json!("vandalism"));

    // A repair needs the cash for the materials.
    let zone = zone_at(&w, shop.rect.centre());
    {
        let mut sites = w.zone(zone).sites_mut();
        let entry = sites.get_mut(shop.id).expect("the shop");
        entry.set_attr("value", entry.attr("value") - 500);
        entry.set_attr("damage", 500);
    }
    w.world.settle();
    assert_eq!(fail(&mut w, cy, "repair", json!({})), ErrorCode::Insufficient);
    assert_eq!(ok(&mut w, cy, "repair", json!({ "amount": 100 }))["cost"], json!(100));
    assert_eq!(w.cash(cy), Money(0));

    // A site of the vandal is not a target.
    w.zone(zone).sites_mut().get_mut(shop.id).expect("the shop").owner = Some(Holder::Resident(cy));
    w.world.settle();
    assert_eq!(fail(&mut w, cy, "vandalize", json!({})), ErrorCode::NotAllowed);
    w.assert_conserved();
}
