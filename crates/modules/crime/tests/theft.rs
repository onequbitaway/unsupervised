//! `pickpocket`, `burgle` and `fence`: the normal path, each refusal and
//! the hot value that a thief holds.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::{ErrorCode, Holder, Money, Pos};

/// A tile on a street of the first zone with shops. No site covers it.
const STREET: Pos = Pos::new(16, 96);

#[test]
fn a_pickpocket_takes_a_part_of_the_cash_and_the_victim_learns_of_it() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));

    let (report, events) = run_with_events(&mut w, cy, "pickpocket", json!({ "target": bo }));
    assert!(report.ok);
    let case = case_of(&report.result);
    // 25 percent of 2,000 cents.
    assert_eq!(
        report.result,
        json!({ "success": true, "case": case, "witnesses": 1, "target": bo, "cash": 500 })
    );
    assert_eq!(w.cash(cy), Money(500));
    assert_eq!(w.cash(bo), Money(1_500));
    assert_eq!(w.attr(cy, "skill.stealth"), 12);

    // The public event has no actor and does not name the thief.
    let thefts = of_kind(&events, "crime.theft_committed");
    assert_eq!(thefts.len(), 1);
    assert_eq!(thefts[0].actor, None);
    assert_eq!(thefts[0].pos, Some(STREET));
    assert_eq!(
        thefts[0].data,
        json!({
            "case": case, "kind": "theft", "city": 0, "site": null, "value": 500,
            "attempt": false,
        })
    );
    let named: Vec<&str> = events
        .iter()
        .filter(|event| event.actor == Some(cy))
        .map(|event| event.kind.as_str())
        .collect();
    // Only the kernel names the thief: it appeared in the world.
    assert_eq!(named, ["kernel.spawned"]);

    // The victim gets a notice. It did not recognize the thief.
    let seen = notice(&w, bo, "crime.witnessed").expect("the victim gets a notice");
    assert_eq!(seen.from, None);
    assert_eq!(
        seen.data,
        json!({ "case": case, "kind": "theft", "role": "victim", "pos": STREET, "value": 500 })
    );

    // The stolen cash is hot in the record of the thief.
    let law = law(&w, cy).expect("the thief has a record");
    assert_eq!(law, json!({ "hot": { "money": 500, "items": {} } }));
    w.assert_conserved();
}

#[test]
fn a_pickpocket_takes_at_most_the_limit() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 100_000, STREET);
    let cy = resident_at(&mut w, "Cy", 40, STREET.offset(0, 1));
    let result = ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    assert_eq!(num(&result, "cash"), 5_000);
    assert_eq!(w.cash(cy), Money(5_040));
    assert_eq!(w.cash(bo), Money(95_000));
    w.assert_conserved();
}

#[test]
fn a_pickpocket_that_fails_takes_nothing_and_is_still_a_case() {
    let mut w = town(&with(&[&[NEVER, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 1));
    let result = ok(&mut w, cy, "pickpocket", json!({ "target": bo }));
    let case = case_of(&result);
    assert_eq!(
        result,
        json!({ "success": false, "case": case, "witnesses": 1, "target": bo, "cash": 0 })
    );
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(w.attr(cy, "skill.stealth"), 4);
    let seen = notice(&w, bo, "crime.witnessed").expect("the victim gets a notice");
    assert_eq!(seen.data["attempt"], json!(true));
    assert_eq!(seen.data["value"], json!(0));
    // The case is on the docket of the city.
    let view = case_view(&mut w, bo, case);
    assert_eq!(view["kind"], json!("theft"));
    assert_eq!(view["attempt"], json!(true));
    assert_eq!(view["status"], json!("open"));
    assert_eq!(view.get("suspect"), None);
    assert_eq!(law(&w, cy), None);
    w.assert_conserved();
}

#[test]
fn pickpocket_refuses_a_bad_target() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let bo = resident_at(&mut w, "Bo", 2_000, STREET);
    let cy = resident_at(&mut w, "Cy", 0, STREET.offset(1, 0));
    let dee = resident_at(&mut w, "Dee", 900, STREET.offset(5, 0));

    assert_eq!(fail(&mut w, cy, "pickpocket", json!({ "target": cy })), ErrorCode::InvalidArgs);
    // Two tiles are too far, and nobody has the id 999.
    assert_eq!(fail(&mut w, cy, "pickpocket", json!({ "target": dee })), ErrorCode::NotHere);
    assert_eq!(fail(&mut w, cy, "pickpocket", json!({ "target": 999 })), ErrorCode::NotHere);
    assert_eq!(fail(&mut w, cy, "pickpocket", json!({})), ErrorCode::InvalidArgs);

    // The guards protect a resident in custody.
    w.set_condition(bo, "jailed", None);
    assert_eq!(fail(&mut w, cy, "pickpocket", json!({ "target": bo })), ErrorCode::NotAllowed);
    w.with_resident(bo, |realm, idx| {
        let jailed = realm.defs().condition_id("jailed").expect("the condition");
        realm.clear_condition(idx, jailed);
    });

    // A victim that walks away during the action is safe.
    w.act_ok(cy, "pickpocket", json!({ "target": bo }));
    w.set_pos(bo, STREET.offset(0, -8));
    w.run_until_idle(cy, 10);
    let report = w.last_report(cy).expect("the action ended");
    assert_eq!(report.error.map(|error| error.code), Some(ErrorCode::NotHere));
    assert_eq!(w.cash(bo), Money(2_000));
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(law(&w, cy), None);
    w.assert_conserved();
}

#[test]
fn a_burglar_takes_half_the_till_and_the_best_goods() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let shop = site(&w, "shop");
    fill_till(&mut w, &shop, 10_000);
    fill_stock(&mut w, &shop, "bread", 8);
    fill_stock(&mut w, &shop, "grain", 20);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let till = Holder::Site(shop.id);

    let (report, events) = run_with_events(&mut w, cy, "burgle", json!({}));
    assert!(report.ok);
    let case = case_of(&report.result);
    // Half of the till. Ten units: the bread first (800 each), then grain.
    assert_eq!(
        report.result,
        json!({
            "success": true, "case": case, "witnesses": 0, "site": shop.id,
            "cash": 5_000, "goods": { "bread": 8, "grain": 2 }, "value": 11_800,
        })
    );
    assert_eq!(w.money(till), Money(5_000));
    assert_eq!((w.qty(till, "bread"), w.qty(till, "grain")), (0, 18));
    assert_eq!(w.cash(cy), Money(5_000));
    let thief = Holder::Resident(cy);
    assert_eq!((w.qty(thief, "bread"), w.qty(thief, "grain")), (8, 2));

    let thefts = of_kind(&events, "crime.theft_committed");
    assert_eq!(thefts.len(), 1);
    assert_eq!(thefts[0].actor, None);
    assert_eq!(
        thefts[0].data,
        json!({
            "case": case, "kind": "burglary", "city": 0, "site": shop.id, "value": 11_800,
            "attempt": false,
        })
    );

    assert_eq!(
        law(&w, cy),
        Some(json!({ "hot": { "money": 5_000, "items": { "bread": 8, "grain": 2 } } }))
    );
    // Value that leaves the hands of the thief is no longer hot.
    ok(&mut w, cy, "drop", json!({ "item": "bread", "qty": 5 }));
    assert_eq!(
        law(&w, cy),
        Some(json!({ "hot": { "money": 5_000, "items": { "bread": 3, "grain": 2 } } }))
    );
    w.assert_conserved();
}

#[test]
fn a_burglar_can_take_one_good_only() {
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    let shop = site(&w, "shop");
    fill_stock(&mut w, &shop, "bread", 8);
    fill_stock(&mut w, &shop, "grain", 20);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let result = ok(&mut w, cy, "burgle", json!({ "item": "grain" }));
    assert_eq!(result["goods"], json!({ "grain": 10 }));
    assert_eq!(result["cash"], json!(0));
    assert_eq!(result["value"], json!(2_000));
    assert_eq!(w.qty(Holder::Site(shop.id), "bread"), 8);
    w.assert_conserved();
}

#[test]
fn burgle_refuses_a_wrong_place_and_fails_without_loot() {
    let mut w = town(&with(&[&[NEVER, NO_WATCH], &BLIND]));
    let shop = site(&w, "shop");
    fill_till(&mut w, &shop, 10_000);
    let cy = resident_at(&mut w, "Cy", 0, STREET);
    assert_eq!(fail(&mut w, cy, "burgle", json!({})), ErrorCode::NotHere);

    w.set_pos(cy, shop.rect.centre());
    assert_eq!(fail(&mut w, cy, "burgle", json!({ "item": "gold" })), ErrorCode::NotFound);

    // The attempt fails: nothing moves, and there is a case.
    let result = ok(&mut w, cy, "burgle", json!({}));
    assert_eq!(result["success"], json!(false));
    assert_eq!(result["cash"], json!(0));
    assert_eq!(result["goods"], json!({}));
    assert_eq!(w.money(Holder::Site(shop.id)), Money(10_000));
    assert_eq!(w.cash(cy), Money(0));
    let view = case_view(&mut w, cy, case_of(&result));
    assert_eq!((&view["kind"], &view["attempt"]), (&json!("burglary"), &json!(true)));

    // A site of the burglar is not a target.
    let zone = zone_at(&w, shop.rect.centre());
    w.zone(zone).sites_mut().get_mut(shop.id).expect("the shop").owner = Some(Holder::Resident(cy));
    w.world.settle();
    assert_eq!(fail(&mut w, cy, "burgle", json!({})), ErrorCode::NotAllowed);
    w.assert_conserved();
}

/// Runs one burglary in a world where only one bonus counts, and returns
/// whether it worked.
fn burglary_works(
    bonus: &'static str,
    prepare: impl Fn(&mut unsup_kernel::testing::TestWorld<'_>, &unsup_kernel::Site),
) -> bool {
    // Without the bonus the chance is 0. With it the chance is 100 percent.
    let mut rules = vec![
        ("crime.chance_min_bp", 0),
        ("crime.chance_max_bp", 10_000),
        ("crime.burgle_base_bp", 0),
        ("crime.night_bonus_bp", 0),
        ("crime.burgle_empty_bonus_bp", 0),
        ("crime.burgle_tool_bonus_bp", 0),
        ("crime.burgle_public_malus_bp", 0),
        NO_WATCH,
    ];
    rules.retain(|(name, _)| *name != bonus);
    rules.push((bonus, 10_000));
    let mut w = town(&with(&[&rules, &BLIND]));
    let shop = site(&w, "shop");
    fill_till(&mut w, &shop, 1_000);
    prepare(&mut w, &shop);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    let result = ok(&mut w, cy, "burgle", json!({}));
    w.assert_conserved();
    result["success"] == json!(true)
}

#[test]
fn a_burglary_is_easier_at_night_in_an_empty_site_and_with_a_tool() {
    // The world starts at midnight. Day starts at 06:00.
    assert!(burglary_works("crime.night_bonus_bp", |_, _| {}));
    assert!(!burglary_works("crime.night_bonus_bp", |w, _| {
        w.step(6 * 60);
    }));

    // Nobody is there. A resident that is awake in the shop spoils it; one
    // that sleeps does not.
    assert!(burglary_works("crime.burgle_empty_bonus_bp", |_, _| {}));
    assert!(!burglary_works("crime.burgle_empty_bonus_bp", |w, shop| {
        resident_at(w, "Bo", 0, shop.rect.centre().offset(1, 0));
    }));
    assert!(burglary_works("crime.burgle_empty_bonus_bp", |w, shop| {
        let bo = resident_at(w, "Bo", 0, shop.rect.centre().offset(1, 0));
        w.set_attr(bo, "need.energy", 100);
        w.act_ok(bo, "sleep", json!({ "ticks": 300 }));
    }));
}

#[test]
fn a_tool_helps_a_burglar_and_a_public_building_is_harder() {
    let with_tool = |tool: bool| {
        let rules = [
            ("crime.chance_min_bp", 0),
            ("crime.burgle_base_bp", 0),
            ("crime.night_bonus_bp", 0),
            ("crime.burgle_empty_bonus_bp", 0),
            ("crime.burgle_tool_bonus_bp", 10_000),
            NO_WATCH,
        ];
        let mut w = town(&with(&[&rules, &BLIND]));
        let shop = site(&w, "shop");
        let cy = resident_in(&mut w, "Cy", 0, &shop);
        if tool {
            w.give_item(cy, "tools", 1);
        }
        let worked = ok(&mut w, cy, "burgle", json!({}))["success"] == json!(true);
        w.assert_conserved();
        worked
    };
    assert!(with_tool(true));
    assert!(!with_tool(false));

    // The same chance works in a shop and fails in the police station.
    let at = |kind: &str| {
        let rules = [
            ("crime.chance_min_bp", 0),
            ("crime.chance_max_bp", 10_000),
            ("crime.burgle_base_bp", 10_000),
            ("crime.burgle_public_malus_bp", 10_000),
            ("crime.night_bonus_bp", 0),
            ("crime.burgle_empty_bonus_bp", 0),
            NO_WATCH,
        ];
        let mut w = town(&with(&[&rules, &BLIND]));
        let place = site(&w, kind);
        let cy = resident_in(&mut w, "Cy", 0, &place);
        let worked = ok(&mut w, cy, "burgle", json!({}))["success"] == json!(true);
        w.assert_conserved();
        worked
    };
    assert!(at("shop"));
    assert!(!at("police_station"));
}

#[test]
fn a_fence_buys_stolen_goods_below_the_depot_price() {
    // A law opens the fence for goods that the depot does not trade.
    let mut w = town(&with(&[&[ALWAYS, NO_WATCH, ("crime.fence_base_bp", 4_000)], &BLIND]));
    let shop = site(&w, "shop");
    fill_stock(&mut w, &shop, "bread", 8);
    fill_stock(&mut w, &shop, "moonshine", 2);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    // Clean bread of its own. A fence does not buy it.
    w.give_item(cy, "bread", 4);
    ok(&mut w, cy, "burgle", json!({}));
    let thief = Holder::Resident(cy);
    assert_eq!((w.qty(thief, "bread"), w.qty(thief, "moonshine")), (12, 2));

    // The depot pays 320 for bread. The fence pays 60 percent of that.
    let (report, events) =
        run_with_events(&mut w, cy, "fence", json!({ "item": "bread", "qty": 3 }));
    assert_eq!(report.result, json!({ "item": "bread", "qty": 3, "unit_price": 192, "paid": 576 }));
    assert_eq!(w.cash(cy), Money(576));
    assert_eq!(w.qty(thief, "bread"), 9);
    let fenced = of_kind(&events, "crime.goods_fenced");
    assert_eq!(fenced.len(), 1);
    assert_eq!(fenced[0].actor, None);
    assert_eq!(fenced[0].data, json!({ "city": 0, "value": 576 }));

    // Without `qty`: every stolen unit that is left, and no clean unit.
    let result = ok(&mut w, cy, "fence", json!({ "item": "bread" }));
    assert_eq!(result, json!({ "item": "bread", "qty": 5, "unit_price": 192, "paid": 960 }));
    assert_eq!(w.qty(thief, "bread"), 4);
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "bread" })), ErrorCode::Insufficient);

    // The depot does not trade contraband. The reference is 40 percent of
    // the normal price of 2,000.
    let result = ok(&mut w, cy, "fence", json!({ "item": "moonshine" }));
    assert_eq!(result, json!({ "item": "moonshine", "qty": 2, "unit_price": 480, "paid": 960 }));
    assert_eq!(w.cash(cy), Money(576 + 960 + 960));
    assert_eq!(law(&w, cy), None);

    // The goods left through the sink and the money came through the
    // faucet of a sale to the depot.
    let blueprint = w.world.blueprint();
    let faucet = blueprint.faucet_id("export.sale").expect("the faucet");
    let sink = blueprint.sink_id("export.sale").expect("the sink");
    let bread = blueprint.item_id("bread").expect("bread");
    let zone = w.zone(zone_at(&w, shop.rect.centre()));
    assert_eq!(zone.ledger().minted_by(faucet), Money(2_496));
    assert_eq!(zone.ledger().consumed_by(sink, bread), 8);
    w.assert_conserved();
}

#[test]
fn fence_refuses_bad_requests() {
    let junk = "[[items]]\nname = \"junk\"\nweight = 1\nsummary = \"A thing without a price.\"\n";
    let mut cfg = config(0, &with(&[&[ALWAYS, NO_WATCH], &BLIND]));
    cfg.data.push(junk.to_string());
    let mut w = unsup_kernel::testing::TestWorld::with_config(&PLUGINS, cfg);
    w.step(2);
    let shop = site(&w, "shop");
    fill_stock(&mut w, &shop, "bread", 2);
    fill_stock(&mut w, &shop, "junk", 1);
    fill_stock(&mut w, &shop, "stims", 1);
    let cy = resident_in(&mut w, "Cy", 0, &shop);
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "bread" })), ErrorCode::Insufficient);
    ok(&mut w, cy, "burgle", json!({}));

    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "gold" })), ErrorCode::NotFound);
    assert_eq!(
        fail(&mut w, cy, "fence", json!({ "item": "bread", "qty": 0 })),
        ErrorCode::InvalidArgs
    );
    assert_eq!(fail(&mut w, cy, "fence", json!({})), ErrorCode::InvalidArgs);
    // Nobody pays for a good without a price. The depot does not trade
    // contraband, and by default the fence does not either.
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "junk" })), ErrorCode::NotAllowed);
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "stims" })), ErrorCode::NotAllowed);
    // No fence deals in a public building.
    let station = site(&w, "police_station");
    w.set_pos(cy, station.rect.centre());
    assert_eq!(fail(&mut w, cy, "fence", json!({ "item": "bread" })), ErrorCode::NotHere);
    assert_eq!(w.cash(cy), Money(0));
    assert_eq!(w.qty(Holder::Resident(cy), "bread"), 2);
    let hot: Value = law(&w, cy).expect("the loot is hot");
    assert_eq!(hot["hot"]["items"], json!({ "bread": 2, "junk": 1, "stims": 1 }));
    w.assert_conserved();
}
