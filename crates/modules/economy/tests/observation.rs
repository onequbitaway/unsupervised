//! What the module shows: the catalogue and the observation sections.

mod common;

use std::collections::BTreeMap;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::protocol::{Detail, VerbScope};
use unsup_kernel::{Money, ObserveOpts};

/// Every verb of the module: name, free, global.
const VERBS: [(&str, bool, bool); 25] = [
    ("apply_job", false, false),
    ("buy", false, false),
    ("buy_property", false, false),
    ("close_business", false, false),
    ("close_job", false, false),
    ("end_lease", false, false),
    ("fire", false, false),
    ("gather", false, false),
    ("list_jobs", true, true),
    ("list_property", true, true),
    ("list_recipes", true, false),
    ("open_business", false, false),
    ("post_job", false, false),
    ("produce", false, false),
    ("quit_job", false, false),
    ("rent", false, false),
    ("rent_out", false, false),
    ("sell", false, false),
    ("sell_property", false, false),
    ("set_price", false, false),
    ("stock", false, false),
    ("till_collect", false, false),
    ("till_fund", false, false),
    ("unstock", false, false),
    ("work", false, false),
];

#[test]
fn the_catalogue_has_every_verb_with_a_schema_that_agents_can_read() {
    let t = Town::new();
    let catalogue = t.world.world.catalogue();
    let mine: Vec<_> = catalogue.verbs.iter().filter(|v| v.module == "economy").collect();
    let found: Vec<(&str, bool, bool)> =
        mine.iter().map(|v| (v.verb.as_str(), v.free, v.scope == VerbScope::Global)).collect();
    assert_eq!(found, VERBS);
    for verb in &mine {
        assert!(verb.summary.ends_with('.'), "`{}` has no summary sentence", verb.verb);
        // Every argument has a description from its doc comment.
        let empty = serde_json::Map::new();
        let properties = verb.args_schema["properties"].as_object().unwrap_or(&empty);
        for (name, schema) in properties {
            let text = schema["description"].as_str().unwrap_or("");
            assert!(text.len() > 10, "`{}`: argument `{name}` has no description", verb.verb);
        }
    }
    let buy = mine.iter().find(|v| v.verb == "buy").expect("buy");
    assert_eq!(buy.args_schema["required"], json!(["item", "qty"]));
}

#[test]
fn the_catalogue_has_the_goods_with_the_tags_that_other_modules_read() {
    let t = Town::new();
    let catalogue = t.world.world.catalogue();
    assert_eq!(catalogue.items.len(), 16);
    let with_tag = |tag: &str| -> Vec<&str> {
        catalogue
            .items
            .iter()
            .filter(|item| item.tags.iter().any(|t| t == tag))
            .map(|item| item.name.as_str())
            .collect()
    };
    assert_eq!(with_tag("raw"), ["grain", "vegetables", "cotton", "herbs", "wood", "ore"]);
    assert_eq!(with_tag("food"), ["vegetables", "bread", "meal"]);
    assert_eq!(with_tag("medicine"), ["medicine"]);
    assert_eq!(with_tag("contraband"), ["moonshine", "stims"]);
    for item in &catalogue.items {
        assert!(item.weight > 0, "{} has no weight", item.name);
        assert!(item.attrs["base_price"] > 0, "{} has no base price", item.name);
        assert!(!item.summary.is_empty(), "{} has no summary", item.name);
        let food = item.tags.iter().any(|t| t == "food");
        assert_eq!(item.attrs.contains_key("nutrition"), food, "{}", item.name);
        let medicine = item.tags.iter().any(|t| t == "medicine");
        assert_eq!(item.attrs.contains_key("healing"), medicine, "{}", item.name);
    }
}

#[test]
fn every_number_is_a_parameter_with_a_summary() {
    let t = Town::new();
    let catalogue = t.world.world.catalogue();
    let mine: BTreeMap<&str, _> = catalogue
        .params
        .iter()
        .filter(|p| p.name.starts_with("economy."))
        .map(|p| (p.name.as_str(), p))
        .collect();
    // Each of the 14 depot goods has a floor and a ceiling parameter.
    let depot = mine.keys().filter(|name| name.starts_with("economy.depot_")).count();
    assert_eq!(depot, 2 * 14);
    assert!(mine.len() > depot, "the parameters of `balance.toml` are in the catalogue");
    for (name, param) in &mine {
        assert!(param.summary.ends_with('.'), "`{name}` has no summary sentence");
        assert!(param.min.is_some(), "`{name}` has no least value");
    }
    // A law can change the policy parameters.
    for name in [
        "economy.income_tax_bp",
        "economy.sales_tax_bp",
        "economy.property_tax_bp",
        "economy.public_works_wage",
        "economy.public_works_budget",
        "economy.min_wage",
        "economy.permit_fee",
        "economy.depot_floor.grain",
        "economy.depot_ceiling.grain",
    ] {
        assert!(mine[name].policy, "`{name}` is a policy parameter");
    }
    assert!(!mine["economy.xp_per_hour"].policy);
    assert_eq!(mine["economy.depot_floor.bread"].value, 320);
    assert_eq!(mine["economy.depot_ceiling.bread"].value, 2_000);
    // The depot never pays more than it charges.
    for (name, floor) in mine.iter().filter(|(name, _)| name.starts_with("economy.depot_floor.")) {
        let item = name.trim_start_matches("economy.depot_floor.");
        let ceiling = mine[format!("economy.depot_ceiling.{item}").as_str()];
        assert!(floor.value < ceiling.value, "{item}");
    }
}

#[test]
fn the_shared_attributes_have_the_definition_of_the_game_design() {
    let t = Town::new();
    let bp = t.world.world.blueprint();
    for name in ["need.mood", "need.energy"] {
        let def = bp.attr(bp.attr_id(name).expect(name));
        assert_eq!((def.min, def.max, def.default), (0, 1_000, 1_000), "{name}");
    }
    for skill in ["labor", "farming", "mining", "crafting", "cooking", "trading", "management"] {
        let name = format!("skill.{skill}");
        let def = bp.attr(bp.attr_id(&name).unwrap_or_else(|| panic!("{name} is declared")));
        assert_eq!((def.min, def.default), (0, 0), "{name}");
    }
}

#[test]
fn a_section_with_nothing_to_say_is_absent() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    // In the street, without a job or property.
    let seen = t.world.observe(ada);
    for name in ["job", "business", "property"] {
        assert_eq!(seen.sections.get(name), None, "{name}");
    }
    // A park offers nothing.
    go(&mut t.world, ada, t.at.park);
    assert_eq!(section(&t.world, ada, "business"), None);
    // A home of the city is for sale and to let.
    go(&mut t.world, ada, t.at.home);
    assert_eq!(
        section(&t.world, ada, "business"),
        Some(json!({ "for_sale": HOME_VALUE, "for_rent": 400, "site": t.at.home, "kind": "home" }))
    );
    // A resident can ask for one section.
    let only = ObserveOpts { sections: Some(vec!["business".into()]), ..Default::default() };
    let seen = t.world.observe_with(ada, &only);
    assert_eq!(seen.sections.keys().collect::<Vec<_>>(), ["business"]);
}

#[test]
fn the_business_section_stays_small_and_full_detail_shows_everything() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    open_site(&mut t.world, owner, t.at.shop);
    t.world.give_money(owner, Money(1_234_567));
    ok(&mut t.world, owner, "till_fund", json!({ "amount": 1_234_567 }));
    // Twelve goods with both prices, and two goods without a price.
    let priced = [
        "grain",
        "vegetables",
        "cotton",
        "herbs",
        "wood",
        "ore",
        "bread",
        "meal",
        "cloth",
        "metal",
        "tools",
        "furniture",
    ];
    for (n, item) in priced.iter().enumerate() {
        t.world.give_item(owner, item, 200);
        ok(&mut t.world, owner, "stock", json!({ "item": item, "qty": 200 }));
        let prices = json!({ "item": item, "price": 10_000 + n, "buy_price": 5_000 + n });
        ok(&mut t.world, owner, "set_price", prices);
    }
    for item in ["clothes", "medicine"] {
        t.world.give_item(owner, item, 3);
        ok(&mut t.world, owner, "stock", json!({ "item": item, "qty": 3 }));
    }
    // Four jobs. The second one pays best.
    let mut jobs = Vec::new();
    for wage in [120_000, 150_000, 110_000, 130_000] {
        let job = ok(&mut t.world, owner, "post_job", json!({ "wage": wage, "slots": 3 }));
        jobs.push(job["job"].clone());
    }
    ok(&mut t.world, owner, "sell_property", json!({ "price": 12_345_678 }));
    let guest = resident_at(&mut t.world, "Guest", 0, t.at.shop);
    let len = |list: &Value| list.as_array().expect("a list").len();

    // Brief: six prices, the three jobs with the best wage, and a count of
    // the rest. The whole section of this full shop stays small.
    let brief = section(&t.world, guest, "business").expect("a business section");
    assert_eq!((len(&brief["prices"]), brief["more_prices"].as_i64()), (6, Some(6)));
    assert_eq!(
        brief["prices"][0],
        json!({ "item": "grain", "qty": 200, "price": 10_000, "buy_price": 5_000 })
    );
    let shown: Vec<&Value> =
        brief["jobs"].as_array().expect("a list").iter().map(|j| &j["job"]).collect();
    assert_eq!(shown, [&jobs[1], &jobs[3], &jobs[0]]);
    assert_eq!(brief["more_jobs"], 1);
    assert_eq!(brief["for_sale"], 12_345_678);
    assert_eq!(brief.get("till"), None);
    assert_eq!(brief.get("stock"), None);
    let size = serde_json::to_string(&brief).expect("json").len();
    assert!(size < 700, "the brief section has {size} bytes");

    // Full detail: every price and every job.
    let full_opts = ObserveOpts { detail: Detail::Full, ..Default::default() };
    let full: Value = t.world.observe_with(guest, &full_opts).sections["business"].clone();
    assert_eq!((len(&full["prices"]), len(&full["jobs"])), (12, 4));
    assert_eq!((full.get("more_prices"), full.get("more_jobs")), (None, None));

    // The owner also sees the till, and the goods in the stock that the
    // price list does not show: six with a price and two without.
    let mine = section(&t.world, owner, "business").expect("a business section");
    assert_eq!(mine["till"], 1_234_567);
    assert_eq!(mine["prices"], brief["prices"]);
    assert_eq!((len(&mine["stock"]), mine["more_stock"].as_i64()), (6, Some(2)));
    assert_eq!(mine["stock"][0], json!({ "item": "bread", "qty": 200 }));
    let size = serde_json::to_string(&mine).expect("json").len();
    assert!(size < 900, "the brief section of the owner has {size} bytes");
    // With full detail the price list shows every priced good, so the
    // stock list has only the two goods without a price.
    let all: Value = t.world.observe_with(owner, &full_opts).sections["business"].clone();
    assert_eq!(
        all["stock"],
        json!([{ "item": "clothes", "qty": 3 }, { "item": "medicine", "qty": 3 }])
    );
    assert_eq!(all.get("more_stock"), None);

    // A worker of the shop sees the stock, but not the till.
    ok(&mut t.world, guest, "apply_job", json!({ "job": jobs[2] }));
    let seen = section(&t.world, guest, "business").expect("a business section");
    assert_eq!(seen["stock"], mine["stock"]);
    assert_eq!(seen["more_stock"], 2);
    assert_eq!(seen.get("till"), None);
    assert_only_transfers(&t.world);
}

#[test]
fn the_property_section_of_a_landlord_stays_small() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    // Eight sites: two more than `economy.observe_sites_max`.
    let places = [
        t.at.farm,
        t.at.mine,
        t.at.forest,
        t.at.workshop,
        t.at.factory,
        t.at.restaurant,
        t.at.shop,
        t.at.office,
    ];
    for place in places {
        buy_site(&mut t.world, ada, place);
    }
    let sites_in = |list: &Value| -> Vec<Value> {
        list.as_array().expect("a list").iter().map(|entry| entry["site"].clone()).collect()
    };
    // Brief: the six oldest and a count of the rest.
    let brief = section(&t.world, ada, "property").expect("a property section");
    assert_eq!(sites_in(&brief["owned"]), places[..6].iter().map(|s| json!(s)).collect::<Vec<_>>());
    assert_eq!(brief["more_owned"], 2);
    let size = serde_json::to_string(&brief).expect("json").len();
    assert!(size < 500, "the brief section has {size} bytes");
    // Full detail: every site.
    let full_opts = ObserveOpts { detail: Detail::Full, ..Default::default() };
    let full = t.world.observe_with(ada, &full_opts).sections["property"].clone();
    assert_eq!(sites_in(&full["owned"]).len(), 8);
    assert_eq!(full.get("more_owned"), None);

    // The same limit holds for the homes that a resident rents.
    t.world.set_param("economy.observe_sites_max", 2);
    t.world.step(1);
    t.world.give_money(ada, Money(10_000));
    for home in [t.at.home, t.at.apartment, t.at.far_home] {
        go(&mut t.world, ada, home);
        ok(&mut t.world, ada, "rent", json!({}));
    }
    let brief = section(&t.world, ada, "property").expect("a property section");
    assert_eq!(sites_in(&brief["leases"]), [json!(t.at.home), json!(t.at.apartment)]);
    assert_eq!(brief["more_leases"], 1);
    assert_eq!(sites_in(&brief["owned"]).len(), 2);
    assert_eq!(brief["more_owned"], 6);
    assert_only_transfers(&t.world);
}

#[test]
fn the_property_section_follows_the_resident_to_other_zones() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    t.world.give_money(ada, Money(2_000));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 600 }));
    go(&mut t.world, ada, t.at.far_home);
    ok(&mut t.world, ada, "rent", json!({}));

    // In the zone of the lease: the home it owns is far, so it has no detail.
    assert_eq!(
        section(&t.world, ada, "property"),
        Some(json!({
            "owned": [{ "site": t.at.home, "zone": 0, "kind": "home" }],
            "leases": [{ "site": t.at.far_home, "zone": 1, "rent": 400 }],
        }))
    );
    // In the zone of the home: the till is visible.
    go(&mut t.world, ada, t.at.park);
    assert_eq!(
        section(&t.world, ada, "property"),
        Some(json!({
            "owned": [{ "site": t.at.home, "zone": 0, "kind": "home", "till": 600 }],
            "leases": [{ "site": t.at.far_home, "zone": 1, "rent": 400 }],
        }))
    );
    assert_only_transfers(&t.world);
}
