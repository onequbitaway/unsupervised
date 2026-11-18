//! The depot: the buyer and seller of last resort.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder};

#[test]
fn the_depot_sells_at_the_ceiling_and_the_money_leaves_the_world() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 5_000, t.at.depot);

    let (result, events) =
        ok_with_events(&mut t.world, ada, "buy", json!({ "item": "grain", "qty": 4 }));
    assert_eq!(
        result,
        json!({
            "site": t.at.depot, "item": "grain", "qty": 4, "unit_price": 500, "total": 2_000,
            "tax": 0,
        })
    );
    assert_eq!(cash(&t.world, ada), 3_000);
    assert_eq!(t.world.qty(Holder::Resident(ada), "grain"), 4);
    assert_eq!(till(&t.world, t.at.depot), 0, "the depot has no till");
    assert_eq!(
        of_kind(&events, "economy.goods_sold"),
        [json!({
            "seller": { "Org": 100 },
            "buyer": ada,
            "site": t.at.depot,
            "city": 0,
            "item": "grain",
            "qty": 4,
            "unit_price": 500,
            "tax": 0,
        })]
    );
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", 5_000)]));
    assert_eq!(flows.burned, map([("import.purchase", 2_000)]));
    assert_eq!(flows.produced, map([("import.purchase:grain", 4)]));
    assert_eq!(flows.consumed, map([]));
}

#[test]
fn the_depot_buys_at_the_floor_and_the_goods_leave_the_world() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.depot);
    t.world.give_item(ada, "grain", 12);

    let (result, events) =
        ok_with_events(&mut t.world, ada, "sell", json!({ "item": "grain", "qty": 10 }));
    assert_eq!(
        result,
        json!({ "site": t.at.depot, "item": "grain", "qty": 10, "unit_price": 80, "total": 800 })
    );
    assert_eq!(cash(&t.world, ada), 800);
    assert_eq!(t.world.qty(Holder::Resident(ada), "grain"), 2);
    assert_eq!(t.world.qty(Holder::Site(t.at.depot), "grain"), 0, "the depot keeps no stock");
    assert_eq!(
        of_kind(&events, "economy.goods_sold"),
        [json!({
            "seller": { "Resident": ada },
            "buyer": null,
            "site": t.at.depot,
            "city": 0,
            "item": "grain",
            "qty": 10,
            "unit_price": 80,
            "tax": 0,
        })]
    );
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("export.sale", 800)]));
    assert_eq!(flows.burned, map([]));
    assert_eq!(flows.produced, map([("kernel.grant:grain", 12)]));
    assert_eq!(flows.consumed, map([("export.sale:grain", 10)]));
}

#[test]
fn every_good_of_the_depot_has_its_own_floor_and_ceiling() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 100_000, t.at.depot);
    for (item, floor, ceiling) in
        [("bread", 320, 2_000), ("ore", 200, 1_250), ("medicine", 1_200, 7_500)]
    {
        let bought = ok(&mut t.world, ada, "buy", json!({ "item": item, "qty": 1 }));
        assert_eq!(bought["unit_price"], ceiling, "{item}");
        let sold = ok(&mut t.world, ada, "sell", json!({ "item": item, "qty": 1 }));
        assert_eq!(sold["unit_price"], floor, "{item}");
    }
    // The prices are parameters. A law can change one.
    t.world.set_param("economy.depot_floor.grain", 90);
    t.world.set_param("economy.depot_ceiling.grain", 450);
    t.world.step(1);
    assert_eq!(ok(&mut t.world, ada, "buy", json!({ "item": "grain", "qty": 2 }))["total"], 900);
    assert_eq!(ok(&mut t.world, ada, "sell", json!({ "item": "grain", "qty": 2 }))["total"], 180);
    // In full detail the depot shows its price list.
    let opts = unsup_kernel::ObserveOpts {
        detail: unsup_kernel::protocol::Detail::Full,
        ..Default::default()
    };
    let seen = t.world.observe_with(ada, &opts).sections["business"].clone();
    let prices = seen["prices"].as_array().expect("a price list");
    assert_eq!(prices.len(), 14);
    assert_eq!(prices[0], json!({ "item": "grain", "price": 450, "buy_price": 90 }));
    assert_eq!(section(&t.world, ada, "business").expect("a section").get("prices"), None);

    let flows = flows(&t.world);
    let spent = 2_000 + 1_250 + 7_500 + 900;
    let earned = 320 + 200 + 1_200 + 180;
    assert_eq!(flows.burned, map([("import.purchase", spent)]));
    assert_eq!(flows.minted, map([("export.sale", earned), ("kernel.grant", 100_000)]));
    // Every unit that came in went out again.
    let units = [("bread", 1), ("grain", 2), ("medicine", 1), ("ore", 1)];
    let named = |flow: &str| units.map(|(item, qty)| (format!("{flow}:{item}"), qty)).into();
    assert_eq!(flows.produced, named("import.purchase"));
    assert_eq!(flows.consumed, named("export.sale"));
    assert_eq!(cash(&t.world, ada), 100_000 - spent + earned);
}

#[test]
fn the_depot_refuses_what_it_must() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 1_000, t.at.depot);
    t.world.give_item(ada, "grain", 3);
    t.world.give_item(ada, "moonshine", 1);
    for (verb, args, code) in [
        // The depot does not trade contraband.
        ("buy", json!({ "item": "moonshine", "qty": 1 }), ErrorCode::NotFound),
        ("sell", json!({ "item": "moonshine", "qty": 1 }), ErrorCode::NotFound),
        ("buy", json!({ "item": "gold", "qty": 1 }), ErrorCode::NotFound),
        ("buy", json!({ "item": "grain", "qty": 3 }), ErrorCode::Insufficient),
        ("sell", json!({ "item": "grain", "qty": 4 }), ErrorCode::Insufficient),
        ("buy", json!({ "item": "grain", "qty": 0 }), ErrorCode::InvalidArgs),
        ("sell", json!({ "item": "grain", "qty": 1_001 }), ErrorCode::InvalidArgs),
        ("buy", json!({ "item": "grain", "qty": 1, "max_price": 499 }), ErrorCode::Conflict),
        ("sell", json!({ "item": "grain", "qty": 1, "min_price": 81 }), ErrorCode::Conflict),
    ] {
        assert_eq!(refused(&mut t.world, ada, verb, args.clone()), code, "{verb} {args}");
    }
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", 1_000)]));
    assert_eq!(flows.of_module(), Flows::default());
    assert_eq!(cash(&t.world, ada), 1_000);
}

#[test]
fn an_override_file_changes_a_depot_price_and_a_tax() {
    // The server gives override files to the world (`--data-dir`).
    let overrides = r#"
        [params."economy.depot_floor.grain"]
        value = 95

        [params."economy.sales_tax_bp"]
        value = 0
    "#;
    let config = unsup_kernel::WorldConfig {
        data: vec![overrides.to_string()],
        ..unsup_kernel::WorldConfig::default()
    };
    let mut world = unsup_kernel::testing::TestWorld::with_config(&[&ECONOMY], config);
    assert_eq!(world.param("economy.sales_tax_bp"), 0);
    let depot = add_site(&mut world, "depot", (8, 10), 1_000_000, true);
    let ada = resident_at(&mut world, "Ada", 0, depot);
    world.give_item(ada, "grain", 2);
    let sold = ok(&mut world, ada, "sell", json!({ "item": "grain", "qty": 2 }));
    assert_eq!((sold["unit_price"].as_i64(), sold["total"].as_i64()), (Some(95), Some(190)));
    // The ceiling keeps the number of `depot.toml`.
    assert_eq!(world.param("economy.depot_ceiling.grain"), 500);
    assert_eq!(
        flows(&world).of_module(),
        module_flows([("export.sale", 190)], [], [], [("export.sale:grain", 2)])
    );
}
