//! The whole chain: gather, sell, produce, stock, buy. Every cent and every
//! unit is accounted for at the end.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{Holder, Money};

#[test]
fn grain_becomes_bread_and_reaches_a_buyer() {
    let mut t = Town::new();

    // Cy owns a restaurant that buys grain and sells bread.
    let cy = t.world.spawn("Cy");
    open_site(&mut t.world, cy, t.at.restaurant);
    t.world.give_money(cy, Money(6_000));
    ok(&mut t.world, cy, "till_fund", json!({ "amount": 6_000 }));
    ok(&mut t.world, cy, "set_price", json!({ "item": "grain", "buy_price": 200 }));

    // Fay starts with nothing. She gathers grain on land of the city for
    // four hours and sells it to the restaurant.
    let fay = resident_at(&mut t.world, "Fay", 0, t.at.farm);
    assert_eq!(ok(&mut t.world, fay, "gather", json!({ "minutes": 240 }))["qty"], 24);
    go(&mut t.world, fay, t.at.restaurant);
    assert_eq!(
        ok(&mut t.world, fay, "sell", json!({ "item": "grain", "qty": 24 }))["total"],
        4_800
    );

    // Cy bakes ten loaves out of the stock and puts a price on them.
    let baked = ok(&mut t.world, cy, "produce", json!({ "recipe": "bread", "batches": 10 }));
    assert_eq!(baked["outputs"], json!([{ "item": "bread", "qty": 10 }]));
    ok(&mut t.world, cy, "set_price", json!({ "item": "bread", "price": 800 }));

    // Eve starts with nothing too. She works one starter shift and buys two
    // loaves with the wage.
    let eve = resident_at(&mut t.world, "Eve", 0, t.at.city_hall);
    assert_eq!(ok(&mut t.world, eve, "work", json!({}))["net"], 2_520);
    go(&mut t.world, eve, t.at.restaurant);
    let bought = ok(&mut t.world, eve, "buy", json!({ "item": "bread", "qty": 2 }));
    assert_eq!((bought["total"].as_i64(), bought["tax"].as_i64()), (Some(1_600), Some(80)));
    t.world.step(2);

    // The goods reached the buyer. Another module lets her eat them: bread
    // has the tag `food` and the attribute `nutrition`.
    assert_eq!(t.world.qty(Holder::Resident(eve), "bread"), 2);
    let catalogue = t.world.world.catalogue();
    let bread = catalogue.items.iter().find(|item| item.name == "bread").expect("bread");
    assert_eq!(bread.tags, ["food"]);
    assert_eq!(bread.attrs["nutrition"], 300);

    // Who holds what.
    assert_eq!(cash(&t.world, fay), 4_800);
    assert_eq!(cash(&t.world, eve), 2_520 - 1_600);
    assert_eq!(cash(&t.world, cy), 0);
    assert_eq!(till(&t.world, t.at.restaurant), 6_000 - 4_800 + 1_600 - 80);
    let place = Holder::Site(t.at.restaurant);
    assert_eq!((t.world.qty(place, "grain"), t.world.qty(place, "bread")), (4, 8));
    // The city: the price of the restaurant, the permit, the income tax of
    // the shift and the sales tax of the bread.
    assert_eq!(treasury(&t.world), 500_000 + 5_000 + 280 + 80);

    // Every faucet and sink, exactly.
    let flows = flows(&t.world);
    assert_eq!(
        flows.minted,
        map([("gov.public_works", 2_800), ("kernel.grant", 500_000 + 5_000 + 6_000)])
    );
    assert_eq!(flows.burned, map([]));
    assert_eq!(
        flows.produced,
        map([("nature.harvest:grain", 24), ("production.output:bread", 10)])
    );
    assert_eq!(flows.consumed, map([("production.input:grain", 20)]));
    // All the money that exists is with these five holders.
    let held = cash(&t.world, fay)
        + cash(&t.world, eve)
        + cash(&t.world, cy)
        + till(&t.world, t.at.restaurant)
        + treasury(&t.world);
    assert_eq!(held, 2_800 + 500_000 + 5_000 + 6_000);
}

#[test]
fn a_firm_with_a_worker_turns_ore_into_tools_and_sells_them_to_the_depot() {
    let mut t = Town::new();
    // The owner of a factory and a workshop.
    let boss = t.world.spawn("Boss");
    open_site(&mut t.world, boss, t.at.factory);
    // Ore from the depot at the ceiling price, wood from the forest.
    t.world.give_money(boss, Money(3 * 1_250 + 4_000));
    go(&mut t.world, boss, t.at.depot);
    ok(&mut t.world, boss, "buy", json!({ "item": "ore", "qty": 3 }));
    go(&mut t.world, boss, t.at.forest);
    assert_eq!(ok(&mut t.world, boss, "gather", json!({ "minutes": 15 }))["qty"], 1);

    go(&mut t.world, boss, t.at.factory);
    for item in ["ore", "wood"] {
        let qty = t.world.qty(Holder::Resident(boss), item);
        ok(&mut t.world, boss, "stock", json!({ "item": item, "qty": qty }));
    }
    ok(&mut t.world, boss, "till_fund", json!({ "amount": 4_000 }));
    // A worker makes the metal in a shift. The boss, who has the skill,
    // makes the tools.
    let job = ok(
        &mut t.world,
        boss,
        "post_job",
        json!({ "wage": 4_000, "slots": 1, "task": "metal" }),
    )["job"]
        .clone();
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.factory);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    let shift = ok(&mut t.world, worker, "work", json!({}));
    assert_eq!(shift["produced"], json!([{ "item": "metal", "qty": 1 }]));
    t.world.set_attr(boss, "skill.crafting", 100);
    ok(&mut t.world, boss, "produce", json!({ "recipe": "tools" }));
    ok(&mut t.world, boss, "unstock", json!({ "item": "tools", "qty": 1 }));
    go(&mut t.world, boss, t.at.depot);
    assert_eq!(
        ok(&mut t.world, boss, "sell", json!({ "item": "tools", "qty": 1 }))["total"],
        1_400
    );
    t.world.step(2);

    assert_eq!(cash(&t.world, worker), 3_600);
    assert_eq!(cash(&t.world, boss), 1_400);
    let flows = flows(&t.world);
    assert_eq!(
        flows.minted,
        map([("export.sale", 1_400), ("kernel.grant", 900_000 + 5_000 + 3_750 + 4_000)])
    );
    assert_eq!(flows.burned, map([("import.purchase", 3_750)]));
    assert_eq!(
        flows.produced,
        map([
            ("import.purchase:ore", 3),
            ("nature.harvest:wood", 1),
            ("production.output:metal", 1),
            ("production.output:tools", 1),
        ])
    );
    assert_eq!(
        flows.consumed,
        map([
            ("export.sale:tools", 1),
            ("production.input:metal", 1),
            ("production.input:ore", 3),
            ("production.input:wood", 1),
        ])
    );
}
