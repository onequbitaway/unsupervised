//! The verb `gather`: raw goods from the node of a farm, a mine or a
//! forest.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder};

#[test]
fn a_resident_gathers_on_land_of_the_city() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.farm);
    let seen = section(&t.world, ada, "business").expect("a business section");
    assert_eq!(seen["gather"], json!({ "goods": ["grain", "vegetables", "cotton"], "left": 600 }));

    // One hour by default: six units of grain.
    let ack = t.world.act_ok(ada, "gather", json!({}));
    assert_eq!(serde_json::to_value(&ack).expect("json")["ticks"], 60);
    let events = settle(&mut t.world, ada);
    let report = t.world.last_report(ada).expect("a report");
    assert_eq!(
        report.result,
        json!({
            "site": t.at.farm, "item": "grain", "qty": 6, "skill": "farming", "xp": 10, "node": 594,
        })
    );
    assert_eq!(t.world.qty(Holder::Resident(ada), "grain"), 6);
    assert_eq!(t.world.attr(ada, "skill.farming"), 10);
    assert_eq!(t.world.attr(ada, "need.energy"), 980);
    assert_eq!(
        of_kind(&events, "economy.goods_produced"),
        [json!({ "resident": ada, "site": t.at.farm, "item": "grain", "qty": 6 })]
    );

    // Another good and another duration: five vegetables an hour for 90 minutes.
    let result = ok(&mut t.world, ada, "gather", json!({ "item": "vegetables", "minutes": 90 }));
    assert_eq!(result["qty"], 7);
    // Wood and herbs come from the forest, ore from the mine.
    go(&mut t.world, ada, t.at.forest);
    assert_eq!(ok(&mut t.world, ada, "gather", json!({ "item": "herbs" }))["qty"], 3);
    assert_eq!(ok(&mut t.world, ada, "gather", json!({}))["item"], "wood");
    go(&mut t.world, ada, t.at.mine);
    let result = ok(&mut t.world, ada, "gather", json!({}));
    assert_eq!((result["item"].as_str(), result["qty"].as_i64()), (Some("ore"), Some(3)));

    // Nothing was given to Ada: the harvest is all that exists.
    assert_eq!(
        flows(&t.world),
        module_flows(
            [],
            [],
            [
                ("nature.harvest:grain", 6),
                ("nature.harvest:herbs", 3),
                ("nature.harvest:ore", 3),
                ("nature.harvest:vegetables", 7),
                ("nature.harvest:wood", 4),
            ],
            []
        )
    );
}

#[test]
fn skill_and_mood_change_the_harvest() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.farm);
    // Level 5 adds 50 percent: nine units an hour.
    t.world.set_attr(ada, "skill.farming", 2_500);
    assert_eq!(ok(&mut t.world, ada, "gather", json!({}))["qty"], 9);
    // Mood 500 gives 75 percent of that: 6.75, rounded down.
    t.world.set_attr(ada, "need.mood", 500);
    let result = ok(&mut t.world, ada, "gather", json!({}));
    assert_eq!(result["qty"], 6);
    assert_eq!(result["xp"], 7);
    assert_eq!(flows(&t.world), module_flows([], [], [("nature.harvest:grain", 15)], []));
}

#[test]
fn a_node_runs_empty_and_regenerates() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.mine);
    // A miner of level 1,000 would take 1,212 units in four hours. The node
    // has 400.
    t.world.set_attr(ada, "skill.mining", 100_000_000);
    let result = ok(&mut t.world, ada, "gather", json!({ "minutes": 240 }));
    assert_eq!(result["qty"], 400);
    assert_eq!(result["node"], 0);
    // Nothing is left now.
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::Insufficient);
    // The mine gets 15 units back each hour: 15 after one hour, and 15 more
    // during the hour of work.
    t.world.step(60);
    let seen = section(&t.world, ada, "business").expect("a business section");
    assert_eq!(seen["gather"]["left"], 15);
    let result = ok(&mut t.world, ada, "gather", json!({}));
    assert_eq!(result["qty"], 30);
    assert_eq!(flows(&t.world), module_flows([], [], [("nature.harvest:ore", 430)], []));
}

#[test]
fn gather_refuses_what_it_must() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    // Not at a site, and at a site without a node.
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::NotHere);
    go(&mut t.world, ada, t.at.shop);
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::NotHere);

    go(&mut t.world, ada, t.at.farm);
    for (args, code) in [
        (json!({ "item": "ore" }), ErrorCode::InvalidArgs),
        (json!({ "item": "gold" }), ErrorCode::NotFound),
        (json!({ "minutes": 9 }), ErrorCode::InvalidArgs),
        (json!({ "minutes": 241 }), ErrorCode::InvalidArgs),
    ] {
        assert_eq!(refused(&mut t.world, ada, "gather", args), code);
    }
    t.world.set_attr(ada, "need.energy", 10);
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::Insufficient);
    t.world.set_attr(ada, "need.energy", 1_000);

    // Land with an owner: only the owner gathers, and a worker uses `work`.
    let owner = t.world.spawn("Owner");
    buy_site(&mut t.world, owner, t.at.farm);
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::NotAllowed);
    let job =
        ok(&mut t.world, owner, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    ok(&mut t.world, ada, "apply_job", json!({ "job": job }));
    assert_eq!(refused(&mut t.world, ada, "gather", json!({})), ErrorCode::NotAllowed);
    // The owner gathers for itself.
    assert_eq!(ok(&mut t.world, owner, "gather", json!({}))["qty"], 6);
    assert_eq!(t.world.qty(Holder::Resident(owner), "grain"), 6);
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("nature.harvest:grain", 6)], [])
    );
}

#[test]
fn a_worker_of_a_city_farm_uses_work_and_not_gather() {
    let mut t = Town::new();
    // An officer of the city posts a job at the farm of the city.
    let gov = unsup_kernel::OrgId::city(CITY);
    let mayor = resident_at(&mut t.world, "Mayor", 0, t.at.farm);
    t.world.with_resident(mayor, |realm, _| realm.grant(mayor, gov, "mayor", &["site.manage"]));
    let job =
        ok(&mut t.world, mayor, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.farm);
    // Everybody gathers on land of the city.
    assert_eq!(ok(&mut t.world, ada, "gather", json!({ "minutes": 10 }))["qty"], 1);
    // With a job at the site, the harvest is for the site: `work`.
    ok(&mut t.world, ada, "apply_job", json!({ "job": job }));
    let error = t.world.act_err(ada, "gather", json!({ "minutes": 10 }));
    assert_eq!(error.code, ErrorCode::NotAllowed);
    assert!(error.message.contains("`work`"), "{}", error.message);
    // After the job, the land of the city is open to it again.
    ok(&mut t.world, ada, "quit_job", json!({}));
    assert_eq!(ok(&mut t.world, ada, "gather", json!({ "minutes": 10 }))["qty"], 1);
    assert_eq!(flows(&t.world), module_flows([], [], [("nature.harvest:grain", 2)], []));
}

#[test]
fn a_gatherer_that_leaves_the_site_gets_nothing() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.farm);
    t.world.act_ok(ada, "gather", json!({}));
    t.world.step(10);
    t.world.set_pos(ada, unsup_kernel::Pos::new(60, 60));
    settle(&mut t.world, ada);
    let report = t.world.last_report(ada).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::NotHere);
    assert_eq!(flows(&t.world), Flows::default());
}
