//! Two cities: each has its own treasury, its own boards, its own
//! parameters and its own public works budget.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::protocol::AdminCommand;
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{CityConfig, CityId, ErrorCode, Holder, OrgId, Pos, SiteId, WorldConfig};

const WEST: CityId = CityId(0);
const EAST: CityId = CityId(1);
const WAGE: i64 = 2_800;

/// A country of two cities with one district each.
struct Country {
    world: TestWorld<'static>,
    west_hall: SiteId,
    west_shop: SiteId,
    east_hall: SiteId,
    east_shop: SiteId,
    east_home: SiteId,
}

fn country() -> Country {
    let config = WorldConfig {
        cities: vec![
            CityConfig::new("Westham", Pos::new(0, 0), 1),
            CityConfig::new("Eastham", Pos::new(200, 0), 1),
        ],
        ..WorldConfig::default()
    };
    let mut world = TestWorld::with_config(&[&ECONOMY], config);
    let west = Holder::Org(OrgId::city(WEST));
    let east = Holder::Org(OrgId::city(EAST));
    Country {
        west_hall: add_site_of(&mut world, west, "city_hall", (2, 2), 1_000_000, true),
        west_shop: add_site_of(&mut world, west, "shop", (8, 2), SHOP_VALUE, false),
        east_hall: add_site_of(&mut world, east, "city_hall", (202, 2), 1_000_000, true),
        east_shop: add_site_of(&mut world, east, "shop", (208, 2), SHOP_VALUE, false),
        east_home: add_site_of(&mut world, east, "home", (214, 2), HOME_VALUE, false),
        world,
    }
}

/// Changes a parameter for one city.
fn set_city_param(world: &mut TestWorld<'static>, name: &str, value: i64, city: CityId) {
    let command = AdminCommand::SetParam { name: name.to_string(), value, city: Some(city) };
    world.world.admin(command);
    world.step(1);
}

fn sites_in(list: &Value) -> Vec<Value> {
    list["listings"].as_array().expect("a list").iter().map(|e| e["site"].clone()).collect()
}

#[test]
fn the_taxes_and_the_price_of_a_site_go_to_the_city_of_the_site() {
    let mut c = country();
    assert_eq!(zone_of_site(&c.world, c.east_shop), unsup_kernel::ZoneId(1));
    let eve = c.world.spawn("Eve");
    open_site(&mut c.world, eve, c.east_shop);
    // The price of the shop and the permit fee.
    c.world.step(1);
    assert_eq!(treasury_of(&c.world, EAST), SHOP_VALUE + 5_000);
    assert_eq!(site(&c.world, c.east_shop).owner, Some(Holder::Resident(eve)));

    // The sales tax of a sale in the east.
    c.world.give_item(eve, "bread", 2);
    ok(&mut c.world, eve, "stock", json!({ "item": "bread", "qty": 2 }));
    ok(&mut c.world, eve, "set_price", json!({ "item": "bread", "price": 1_000 }));
    let ada = resident_at(&mut c.world, "Ada", 2_000, c.east_shop);
    let (bought, events) =
        ok_with_events(&mut c.world, ada, "buy", json!({ "item": "bread", "qty": 2 }));
    assert_eq!(bought["tax"], 100);
    assert_eq!(of_kind(&events, "economy.goods_sold")[0]["city"], 1);
    // The income tax of a starter shift in the east.
    go(&mut c.world, ada, c.east_hall);
    assert_eq!(ok(&mut c.world, ada, "work", json!({}))["tax"], 280);
    c.world.step(1);
    assert_eq!(treasury_of(&c.world, EAST), SHOP_VALUE + 5_000 + 100 + 280);
    assert_eq!(treasury_of(&c.world, WEST), 0);

    // The property tax of the shop, the next day: five basis points.
    go(&mut c.world, eve, c.east_shop);
    next_day(&mut c.world);
    assert_eq!(treasury_of(&c.world, EAST), SHOP_VALUE + 5_000 + 100 + 280 + 250);
    assert_eq!(treasury_of(&c.world, WEST), 0);
    assert_eq!(flows(&c.world).of_module(), module_flows([("gov.public_works", WAGE)], [], [], []));
}

#[test]
fn each_city_has_its_own_public_works_budget() {
    let mut c = country();
    // Room for one starter shift a day in each city.
    c.world.set_param("economy.public_works_budget", WAGE);
    c.world.step(1);
    let [a, b] = ["A", "B"].map(|name| resident_at(&mut c.world, name, 0, c.west_hall));
    let [x, y] = ["X", "Y"].map(|name| resident_at(&mut c.world, name, 0, c.east_hall));

    c.world.act_ok(a, "work", json!({}));
    c.world.step(2);
    // The west has spent its budget. The east has not.
    assert_eq!(refused(&mut c.world, b, "work", json!({})), ErrorCode::Insufficient);
    c.world.act_ok(x, "work", json!({}));
    c.world.step(2);
    assert_eq!(refused(&mut c.world, y, "work", json!({})), ErrorCode::Insufficient);
    settle(&mut c.world, a);
    settle(&mut c.world, x);
    assert_eq!((cash(&c.world, a), cash(&c.world, x)), (2_520, 2_520));
    c.world.step(1);
    assert_eq!((treasury_of(&c.world, WEST), treasury_of(&c.world, EAST)), (280, 280));
    assert_eq!(flows(&c.world), module_flows([("gov.public_works", 2 * WAGE)], [], [], []));
}

#[test]
fn a_city_can_have_its_own_wage_tax_and_budget() {
    let mut c = country();
    // A law of the east: a higher wage, a higher income tax, and a budget
    // for one shift at that wage.
    set_city_param(&mut c.world, "economy.public_works_wage", 3_000, EAST);
    set_city_param(&mut c.world, "economy.income_tax_bp", 2_000, EAST);
    set_city_param(&mut c.world, "economy.public_works_budget", 3_000, EAST);
    let a = resident_at(&mut c.world, "A", 0, c.west_hall);
    let [x, y] = ["X", "Y"].map(|name| resident_at(&mut c.world, name, 0, c.east_hall));

    // The job board of each city shows the wage of that city.
    let west = ask_board(&mut c.world, a, "list_jobs", json!({}));
    let east = ask_board(&mut c.world, x, "list_jobs", json!({}));
    assert_eq!(
        (west["city"].as_i64(), west["public_works"]["wage"].as_i64()),
        (Some(0), Some(WAGE))
    );
    assert_eq!(
        (east["city"].as_i64(), east["public_works"]["wage"].as_i64()),
        (Some(1), Some(3_000))
    );

    // X and Y start in the same frame. The budget of the east has room for
    // one shift at the wage of the east: the shift of Y stops.
    c.world.act_ok(x, "work", json!({}));
    c.world.act_ok(y, "work", json!({}));
    let west_shift = ok(&mut c.world, a, "work", json!({}));
    settle(&mut c.world, x);
    let east_shift = c.world.last_report(x).expect("a report").result;
    assert_eq!((west_shift["wage"].as_i64(), west_shift["tax"].as_i64()), (Some(WAGE), Some(280)));
    assert_eq!((east_shift["wage"].as_i64(), east_shift["tax"].as_i64()), (Some(3_000), Some(600)));
    let stopped = c.world.last_report(y).expect("a report").error.expect("an error");
    assert_eq!(stopped.code, ErrorCode::Interrupted);
    assert_eq!(cash(&c.world, y), 0);
    assert_eq!(refused(&mut c.world, y, "work", json!({})), ErrorCode::Insufficient);
    // The budget of the west is the national one: it has room for more.
    ok(&mut c.world, a, "work", json!({}));
    c.world.step(1);
    assert_eq!((treasury_of(&c.world, WEST), treasury_of(&c.world, EAST)), (560, 600));
    assert_eq!(flows(&c.world), module_flows([("gov.public_works", 2 * WAGE + 3_000)], [], [], []));
}

#[test]
fn the_boards_of_a_city_list_only_that_city() {
    let mut c = country();
    let wes = resident_at(&mut c.world, "Wes", 0, c.west_hall);
    let eve = c.world.spawn("Eve");
    open_site(&mut c.world, eve, c.east_shop);
    let job =
        ok(&mut c.world, eve, "post_job", json!({ "wage": 4_000, "slots": 1 }))["job"].clone();
    c.world.step(60);

    // Property: the shop of the west; the home of the east. The shop of the
    // east has an owner now.
    let west = ask_board(&mut c.world, wes, "list_property", json!({}));
    let east = ask_board(&mut c.world, eve, "list_property", json!({}));
    assert_eq!((west["city"].as_i64(), sites_in(&west)), (Some(0), vec![json!(c.west_shop)]));
    assert_eq!((east["city"].as_i64(), sites_in(&east)), (Some(1), vec![json!(c.east_home)]));

    // Jobs: the job of the east is not on the board of the west.
    let west = ask_board(&mut c.world, wes, "list_jobs", json!({}));
    let east = ask_board(&mut c.world, eve, "list_jobs", json!({}));
    assert_eq!(west["jobs"], json!([]));
    assert_eq!(east["jobs"].as_array().map(Vec::len), Some(1));
    assert_eq!(east["jobs"][0]["job"], job);
    assert_eq!(east["jobs"][0]["zone"], 1);
    assert_only_transfers(&c.world);
}
