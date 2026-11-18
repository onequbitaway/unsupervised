//! The verbs `produce` and `list_recipes`.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::{ErrorCode, Holder, ResidentId};

/// A restaurant with a permit and grain in its stock.
fn kitchen(t: &mut Town, grain: i64) -> ResidentId {
    let cook = t.world.spawn("Cook");
    open_site(&mut t.world, cook, t.at.restaurant);
    t.world.give_item(cook, "grain", grain);
    ok(&mut t.world, cook, "stock", json!({ "item": "grain", "qty": grain }));
    cook
}

#[test]
fn a_recipe_turns_the_inputs_in_the_stock_into_outputs_in_the_stock() {
    let mut t = Town::new();
    let cook = kitchen(&mut t, 10);
    let place = Holder::Site(t.at.restaurant);

    // Three batches of bread: 20 minutes and two units of grain each.
    let ack = t.world.act_ok(cook, "produce", json!({ "recipe": "bread", "batches": 3 }));
    assert_eq!(serde_json::to_value(&ack).expect("json")["ticks"], 60);
    // Nothing moves before the end.
    assert_eq!(t.world.qty(place, "grain"), 10);
    let events = settle(&mut t.world, cook);
    let report = t.world.last_report(cook).expect("a report");
    assert_eq!(
        report.result,
        json!({
            "site": t.at.restaurant,
            "recipe": "bread",
            "batches": 3,
            "outputs": [{ "item": "bread", "qty": 3 }],
            "skill": "cooking",
            "xp": 10,
        })
    );
    assert_eq!(t.world.qty(place, "grain"), 4);
    assert_eq!(t.world.qty(place, "bread"), 3);
    assert_eq!(t.world.qty(Holder::Resident(cook), "bread"), 0);
    assert_eq!(t.world.attr(cook, "skill.cooking"), 10);
    assert_eq!(t.world.attr(cook, "need.energy"), 980);
    assert_eq!(
        of_kind(&events, "economy.goods_produced"),
        [json!({ "resident": cook, "site": t.at.restaurant, "item": "bread", "qty": 3 })]
    );

    // One batch when the resident gives no number.
    let result = ok(&mut t.world, cook, "produce", json!({ "recipe": "bread" }));
    assert_eq!(result["batches"], 1);

    let flows = flows(&t.world);
    assert_eq!(flows.produced, map([("kernel.grant:grain", 10), ("production.output:bread", 4)]));
    assert_eq!(
        flows.of_module(),
        module_flows([], [], [("production.output:bread", 4)], [("production.input:grain", 8)])
    );
}

#[test]
fn a_recipe_with_two_inputs_and_two_units_of_output() {
    let mut t = Town::new();
    let maker = t.world.spawn("Maker");
    open_site(&mut t.world, maker, t.at.workshop);
    t.world.set_attr(maker, "skill.medicine", 400);
    for (item, qty) in [("herbs", 5), ("medicine", 2)] {
        t.world.give_item(maker, item, qty);
        ok(&mut t.world, maker, "stock", json!({ "item": item, "qty": qty }));
    }
    // Stims: two herbs and one medicine give two units. They are contraband.
    let result = ok(&mut t.world, maker, "produce", json!({ "recipe": "stims", "batches": 2 }));
    assert_eq!(result["outputs"], json!([{ "item": "stims", "qty": 4 }]));
    let place = Holder::Site(t.at.workshop);
    assert_eq!(
        (t.world.qty(place, "herbs"), t.world.qty(place, "medicine"), t.world.qty(place, "stims")),
        (1, 0, 4)
    );
    let stims = t.world.world.catalogue().items.into_iter().find(|i| i.name == "stims");
    assert_eq!(stims.expect("stims are in the catalogue").tags, ["contraband"]);
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows(
            [],
            [],
            [("production.output:stims", 4)],
            [("production.input:herbs", 4), ("production.input:medicine", 2)]
        )
    );
}

#[test]
fn skill_above_the_least_level_makes_production_faster() {
    let mut t = Town::new();
    let cook = kitchen(&mut t, 10);
    // Level 2 is two levels above the least level of bread: 60 / 1.1 = 54.5.
    t.world.set_attr(cook, "skill.cooking", 400);
    let ack = t.world.act_ok(cook, "produce", json!({ "recipe": "bread", "batches": 3 }));
    assert_eq!(serde_json::to_value(&ack).expect("json")["ticks"], 55);
    settle(&mut t.world, cook);
    assert_eq!(t.world.qty(Holder::Site(t.at.restaurant), "bread"), 3);
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("production.output:bread", 3)], [("production.input:grain", 6)])
    );
}

#[test]
fn produce_refuses_what_it_must() {
    let mut t = Town::new();
    let cook = t.world.spawn("Cook");
    let bread = json!({ "recipe": "bread" });
    // Not at a site, and at a restaurant without a permit.
    assert_eq!(refused(&mut t.world, cook, "produce", bread.clone()), ErrorCode::NotHere);
    buy_site(&mut t.world, cook, t.at.restaurant);
    assert_eq!(refused(&mut t.world, cook, "produce", bread.clone()), ErrorCode::NotAllowed);
    t.world.give_money(cook, unsup_kernel::Money(5_000));
    ok(&mut t.world, cook, "open_business", json!({}));

    // No inputs in the stock.
    let error = t.world.act_err(cook, "produce", bread.clone());
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "missing": { "grain": { "needed": 2, "available": 0 } } }));
    t.world.give_item(cook, "grain", 4);
    ok(&mut t.world, cook, "stock", json!({ "item": "grain", "qty": 4 }));
    let three = json!({ "recipe": "bread", "batches": 3 });
    assert_eq!(refused(&mut t.world, cook, "produce", three), ErrorCode::Insufficient);

    for (args, code) in [
        (json!({ "recipe": "soup" }), ErrorCode::NotFound),
        // Tools are made at a workshop or a factory.
        (json!({ "recipe": "tools" }), ErrorCode::NotHere),
        (json!({ "recipe": "bread", "batches": 0 }), ErrorCode::InvalidArgs),
        (json!({ "recipe": "bread", "batches": 21 }), ErrorCode::InvalidArgs),
        // A meal needs level 1 in cooking.
        (json!({ "recipe": "meal" }), ErrorCode::NotAllowed),
    ] {
        assert_eq!(refused(&mut t.world, cook, "produce", args), code);
    }
    // Too tired to work.
    t.world.set_attr(cook, "need.energy", 49);
    assert_eq!(refused(&mut t.world, cook, "produce", bread.clone()), ErrorCode::Insufficient);
    t.world.set_attr(cook, "need.energy", 1_000);
    // A stranger cannot use the kitchen. A worker of the site can.
    let guest = resident_at(&mut t.world, "Guest", 0, t.at.restaurant);
    assert_eq!(refused(&mut t.world, guest, "produce", bread.clone()), ErrorCode::NotAllowed);
    let job = ok(&mut t.world, cook, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    ok(&mut t.world, guest, "apply_job", json!({ "job": job }));
    ok(&mut t.world, guest, "produce", bread);
    assert_eq!(t.world.qty(Holder::Site(t.at.restaurant), "bread"), 1);
    // Of all the requests, only the last one made bread.
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("production.output:bread", 1)], [("production.input:grain", 2)])
    );
}

#[test]
fn inputs_that_go_away_during_the_work_fail_the_action_and_change_nothing() {
    let mut t = Town::new();
    let cook = kitchen(&mut t, 4);
    let helper = resident_at(&mut t.world, "Helper", 0, t.at.restaurant);
    let job = ok(&mut t.world, cook, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    ok(&mut t.world, helper, "apply_job", json!({ "job": job }));

    t.world.act_ok(helper, "produce", json!({ "recipe": "bread", "batches": 2 }));
    // The owner takes the grain back while the helper works.
    ok(&mut t.world, cook, "unstock", json!({ "item": "grain", "qty": 3 }));
    settle(&mut t.world, helper);
    let report = t.world.last_report(helper).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::Insufficient);
    assert_eq!(t.world.qty(Holder::Site(t.at.restaurant), "grain"), 1);
    assert_eq!(t.world.qty(Holder::Site(t.at.restaurant), "bread"), 0);
    assert_eq!(t.world.attr(helper, "skill.cooking"), 0);
    let flows = flows(&t.world);
    assert_eq!(flows.produced, map([("kernel.grant:grain", 4)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn list_recipes_is_free_and_filters() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    let list = |t: &mut Town, args| {
        // A resident has a few free actions in each tick.
        t.world.step(1);
        match t.world.act_ok(ada, "list_recipes", args) {
            ActionStatus::Done { result } => result["recipes"].as_array().expect("a list").clone(),
            other => panic!("a free verb ends at once: {other:?}"),
        }
    };
    let all = list(&mut t, json!({}));
    assert_eq!(all.len(), 10);
    assert_eq!(
        all[0],
        json!({
            "recipe": "bread",
            "sites": ["restaurant", "factory"],
            "skill": "cooking",
            "min_level": 0,
            "minutes": 20,
            "inputs": { "grain": 2 },
            "outputs": { "bread": 1 },
        })
    );
    let names = |list: &[serde_json::Value]| -> Vec<String> {
        list.iter().map(|r| r["recipe"].as_str().expect("a name").to_string()).collect()
    };
    assert_eq!(names(&list(&mut t, json!({ "site": "restaurant" }))), ["bread", "meal"]);
    assert_eq!(names(&list(&mut t, json!({ "output": "moonshine" }))), ["moonshine"]);
    assert_eq!(names(&list(&mut t, json!({ "site": "factory", "output": "tools" }))), ["tools"]);
    t.world.step(1);
    let gold = json!({ "output": "gold" });
    assert_eq!(refused(&mut t.world, ada, "list_recipes", gold), ErrorCode::NotFound);
    // The contraband recipes make goods with the tag `contraband`.
    let catalogue = t.world.world.catalogue();
    let contraband: Vec<&str> = catalogue
        .items
        .iter()
        .filter(|item| item.tags.iter().any(|tag| tag == "contraband"))
        .map(|item| item.name.as_str())
        .collect();
    assert_eq!(contraband, ["moonshine", "stims"]);
    for name in contraband {
        assert_eq!(list(&mut t, json!({ "output": name })).len(), 1, "{name} has a recipe");
    }
}
