//! The city boards: `list_jobs` and `list_property`.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::protocol::ActionStatus;
use unsup_kernel::{ErrorCode, Pos, ResidentId, SiteId};

/// Asks a board of the town.
fn ask(t: &mut Town, r: ResidentId, verb: &str, args: Value) -> Value {
    ask_board(&mut t.world, r, verb, args)
}

fn sites_of(list: &Value, key: &str) -> Vec<SiteId> {
    list[key]
        .as_array()
        .expect("a list")
        .iter()
        .map(|e| SiteId(e["site"].as_u64().expect("a site id")))
        .collect()
}

#[test]
fn the_property_board_lists_what_the_city_sells_and_lets() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    assert_eq!(t.world.pos(ada), Pos::new(32, 32));
    // The zones tell the global realm at their regular check.
    t.world.step(60);

    // Twelve sites are for sale; three of them also take tenants.
    let all = ask(&mut t, ada, "list_property", json!({}));
    assert_eq!(all["city"], 0);
    assert_eq!(all["listings"].as_array().expect("a list").len(), 10);
    assert_eq!(all["more"], 2);
    let wide = ask(&mut t, ada, "list_property", json!({ "limit": 100 }));
    assert_eq!(wide["listings"].as_array().expect("a list").len(), 12);
    assert_eq!(wide["more"], 0);
    // Without an offer the nearest site comes first: the office at 20
    // tiles. The sites of the north row are all 28 tiles away (a diagonal
    // step counts as one), so they follow in id order.
    assert_eq!(sites_of(&all, "listings")[..3], [t.at.office, t.at.farm, t.at.mine]);
    let public = [t.at.city_hall, t.at.depot, t.at.park, t.at.far_hall];
    assert!(sites_of(&wide, "listings").iter().all(|s| !public.contains(s)));

    // The cheapest rent first; equal rents by distance.
    let rent = ask(&mut t, ada, "list_property", json!({ "offer": "rent" }));
    assert_eq!(sites_of(&rent, "listings"), [t.at.home, t.at.far_home, t.at.apartment]);
    assert_eq!(
        rent["listings"][0],
        json!({
            "site": t.at.home,
            "kind": "home",
            "name": "Test home",
            "zone": 0,
            "pos": { "x": 46, "y": 4 },
            "rent": 400,
            "units_free": 1,
        })
    );
    assert_eq!(rent["listings"][2]["units_free"], 8);

    // The cheapest price first.
    let cheap = ask(&mut t, ada, "list_property", json!({ "offer": "sale", "limit": 3 }));
    assert_eq!(sites_of(&cheap, "listings"), [t.at.home, t.at.far_home, t.at.forest]);
    assert_eq!(cheap["listings"][0]["price"], HOME_VALUE);
    assert_eq!(cheap["listings"][0].get("rent"), None);
    assert_eq!(cheap["more"], 9);
    // Filters: kind and greatest price.
    let shops = ask(&mut t, ada, "list_property", json!({ "kind": "shop" }));
    assert_eq!(sites_of(&shops, "listings"), [t.at.shop, t.at.far_shop]);
    let small = ask(&mut t, ada, "list_property", json!({ "offer": "sale", "max_price": 250_000 }));
    assert_eq!(sites_of(&small, "listings"), [t.at.home, t.at.far_home, t.at.forest]);
    let none = ask(&mut t, ada, "list_property", json!({ "kind": "castle" }));
    assert_eq!(none, json!({ "city": 0, "listings": [], "more": 0 }));
    assert_only_transfers(&t.world);
}

#[test]
fn the_property_board_follows_sales_offers_and_leases() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    t.world.step(60);
    let homes = json!({ "kind": "home" });
    assert_eq!(
        sites_of(&ask(&mut t, ada, "list_property", homes.clone()), "listings"),
        [t.at.home, t.at.far_home]
    );

    // A home with an owner leaves the board.
    buy_site(&mut t.world, ada, t.at.home);
    assert_eq!(
        sites_of(&ask(&mut t, ada, "list_property", homes.clone()), "listings"),
        [t.at.far_home]
    );
    // The owner offers it: for sale and to let.
    ok(&mut t.world, ada, "sell_property", json!({ "price": 222_000 }));
    ok(&mut t.world, ada, "rent_out", json!({ "rent": 700 }));
    let list = ask(&mut t, ada, "list_property", homes.clone());
    assert_eq!(list["listings"][0]["site"], json!(t.at.home));
    assert_eq!(
        (list["listings"][0]["price"].as_i64(), list["listings"][0]["rent"].as_i64()),
        (Some(222_000), Some(700))
    );
    // A tenant takes the only unit: the home is only for sale now.
    let bob = resident_at(&mut t.world, "Bob", 5_000, t.at.home);
    ok(&mut t.world, bob, "rent", json!({}));
    let list = ask(&mut t, ada, "list_property", json!({ "kind": "home", "offer": "rent" }));
    assert_eq!(sites_of(&list, "listings"), [t.at.far_home]);
    let list = ask(&mut t, ada, "list_property", json!({ "kind": "home", "offer": "sale" }));
    assert_eq!(sites_of(&list, "listings"), [t.at.far_home, t.at.home]);
    assert_only_transfers(&t.world);
}

#[test]
fn the_job_board_lists_the_jobs_that_take_workers() {
    let mut t = Town::new();
    let boss = t.world.spawn("Boss");
    let post = |t: &mut Town, args: Value| {
        ok(&mut t.world, boss, "post_job", args)["job"].as_u64().expect("a job id")
    };
    buy_site(&mut t.world, boss, t.at.farm);
    let farm_job = post(&mut t, json!({ "wage": 3_000, "slots": 1 }));
    open_site(&mut t.world, boss, t.at.far_shop);
    let shop_job = post(&mut t, json!({ "wage": 5_000, "slots": 2, "min_level": 1 }));
    open_site(&mut t.world, boss, t.at.workshop);
    let cloth_job = post(&mut t, json!({ "wage": 4_000, "slots": 1, "task": "cloth" }));
    let ada = t.world.spawn("Ada");
    let pick = |list: &Value| -> Vec<u64> {
        let jobs = list["jobs"].as_array().expect("a list");
        jobs.iter().map(|j| j["job"].as_u64().expect("a job id")).collect()
    };

    // The best wage first.
    let list = ask(&mut t, ada, "list_jobs", json!({}));
    assert_eq!(pick(&list), [shop_job, cloth_job, farm_job]);
    assert_eq!(
        list["jobs"][0],
        json!({
            "job": shop_job,
            "site": t.at.far_shop,
            "kind": "shop",
            "name": "Test shop",
            "zone": 1,
            "pos": { "x": 74, "y": 4 },
            "wage": 5_000,
            "skill": "trading",
            "min_level": 1,
            "free_slots": 2,
        })
    );
    assert_eq!(list["jobs"][1]["task"], "cloth");
    assert_eq!(list["jobs"][1]["skill"], "crafting");
    assert_eq!(list["more"], 0);

    assert_eq!(pick(&ask(&mut t, ada, "list_jobs", json!({ "skill": "farming" }))), [farm_job]);
    assert_eq!(pick(&ask(&mut t, ada, "list_jobs", json!({ "kind": "workshop" }))), [cloth_job]);
    let paid = ask(&mut t, ada, "list_jobs", json!({ "min_wage": 4_000 }));
    assert_eq!(pick(&paid), [shop_job, cloth_job]);
    let one = ask(&mut t, ada, "list_jobs", json!({ "limit": 1 }));
    assert_eq!((pick(&one), one["more"].as_i64()), (vec![shop_job], Some(2)));

    // A job that is full leaves the board, and comes back when it has room.
    let farms = json!({ "kind": "farm" });
    go(&mut t.world, ada, t.at.farm);
    ok(&mut t.world, ada, "apply_job", json!({ "job": farm_job }));
    assert_eq!(pick(&ask(&mut t, ada, "list_jobs", farms.clone())), Vec::<u64>::new());
    ok(&mut t.world, ada, "quit_job", json!({}));
    assert_eq!(pick(&ask(&mut t, ada, "list_jobs", farms.clone())), [farm_job]);
    // A closed job leaves the board.
    go(&mut t.world, boss, t.at.farm);
    ok(&mut t.world, boss, "close_job", json!({ "job": farm_job }));
    assert_eq!(pick(&ask(&mut t, ada, "list_jobs", farms)), Vec::<u64>::new());
    assert_only_transfers(&t.world);
}

#[test]
fn a_site_that_another_module_adds_later_reaches_the_board_at_the_next_check() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    // The first tick puts the sites that exist on the board.
    t.world.step(5);
    let castles = json!({ "kind": "castle" });
    let listed =
        |t: &mut Town| sites_of(&ask(t, ada, "list_property", castles.clone()), "listings");
    assert_eq!(listed(&mut t), []);
    // Another module adds a site. No verb of this module ran.
    let castle = add_site(&mut t.world, "castle", (26, 10), 700_000, false);
    t.world.step(3);
    assert_eq!(listed(&mut t), [], "the board does not know the site yet");
    // Each zone checks its sites every `economy.board_refresh_minutes`.
    t.world.step(60);
    assert_eq!(listed(&mut t), [castle]);
    assert_only_transfers(&t.world);
}

#[test]
fn the_job_board_says_what_the_city_pays_for_a_starter_shift() {
    let mut t = Town::new();
    // Room for one starter shift a day.
    t.world.set_param("economy.public_works_budget", 2_800);
    t.world.step(1);
    let ada = resident_at(&mut t.world, "Ada", 0, t.at.city_hall);
    let bob = t.world.spawn("Bob");
    // No job is posted. The board still shows a way to earn.
    assert_eq!(
        ask(&mut t, bob, "list_jobs", json!({})),
        json!({
            "city": 0,
            "jobs": [],
            "more": 0,
            "public_works": {
                "wage": 2_800,
                "minutes": 240,
                "kinds": ["city_hall", "depot"],
                "open": true,
            },
        })
    );
    // Ada takes the only shift of the day.
    t.world.act_ok(ada, "work", json!({}));
    t.world.step(2);
    assert_eq!(ask(&mut t, bob, "list_jobs", json!({}))["public_works"]["open"], false);
    // A new day has a new budget, also before anybody asks for a shift.
    settle(&mut t.world, ada);
    next_day(&mut t.world);
    assert_eq!(ask(&mut t, bob, "list_jobs", json!({}))["public_works"]["open"], true);
    // A law ends public works: the board says nothing about them.
    t.world.set_param("economy.public_works_wage", 0);
    t.world.step(1);
    assert_eq!(
        ask(&mut t, bob, "list_jobs", json!({})),
        json!({ "city": 0, "jobs": [], "more": 0 })
    );
    assert_eq!(flows(&t.world), module_flows([("gov.public_works", 2_800)], [], [], []));
}

#[test]
fn a_resident_in_jail_can_still_read_the_boards() {
    // The crime module owns `jailed`. A stand-in declares it here.
    struct Jail;
    impl unsup_kernel::Plugin for Jail {
        fn name(&self) -> &'static str {
            "jail"
        }
        fn build(&self, b: &mut unsup_kernel::Builder) {
            b.condition(unsup_kernel::CondDef::new("jailed").blocking());
        }
    }
    static JAIL: Jail = Jail;
    let mut world = unsup_kernel::testing::TestWorld::new(&[&ECONOMY, &JAIL]);
    let at = build(&mut world);
    let ada = resident_at(&mut world, "Ada", HOME_VALUE, at.home);
    // A free resident could buy the home.
    world.act_ok(ada, "buy_property", json!({}));
    world.act_ok(ada, "cancel", json!({}));
    world.set_condition(ada, "jailed", None);
    assert_eq!(world.act_err(ada, "buy_property", json!({})).code, ErrorCode::Blocked);
    assert_eq!(world.act(ada, "list_property", json!({})).status, ActionStatus::Pending);
    assert_eq!(world.act(ada, "list_jobs", json!({})).status, ActionStatus::Pending);
    assert!(matches!(world.act(ada, "list_recipes", json!({})).status, ActionStatus::Done { .. }));
    world.assert_conserved();
}
