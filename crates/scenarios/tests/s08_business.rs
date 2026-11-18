//! Story 8: a business.
//!
//! Ann buys a restaurant from the city, pays for the permit, hires Ben,
//! and Ben bakes bread out of the grain that Ann gathered. Cal buys the
//! bread. The city gets the price of the site, the permit fee, the income
//! tax of the wage, the sales tax of the bread and the property tax.
//!
//! The story starts in the republic (`unsup_scenarios::republic`). The
//! president pays the capital of the business from the national treasury.

use serde_json::json;
use unsup_kernel::{CityId, ErrorCode, Holder};
use unsup_scenarios::republic::grant;
use unsup_scenarios::{Game, Republic, Setup, assert_deterministic_in_republic};

const CITY: CityId = CityId(0);
const WAGE: i64 = 3_000;
const PRICE_OF_BREAD: i64 = 1_000;

fn story(g: &mut Game, republic: &Republic) {
    let [ann, ben, cal, _] = republic.citizens;
    let treasury = g.city_treasury(CITY);
    let (ann_cash, ben_cash, cal_cash) = (g.cash(ann), g.cash(ben), g.cash(cal));

    // The property board lists the restaurants that the city sells, the
    // cheapest first. Ann gets the price, the permit fee and 20,000 for
    // the till from the nation.
    let offers = g.act(ann, "list_property", json!({ "kind": "restaurant", "offer": "sale" }));
    let offer = offers["listings"][0].clone();
    let price = offer["price"].as_i64().expect("a price");
    assert_eq!(price, 4_675_000);
    let place = g.site_now(unsup_kernel::SiteId(offer["site"].as_u64().expect("a site")));
    assert_eq!((place.kind.as_str(), place.attr("value")), ("restaurant", price));
    grant(g, republic, ann, price + 5_000 + 20_000);

    // The raw goods: four hours at the farm of the city give 22 units of
    // grain.
    let farm = g.site("farm");
    g.walk(ann, &farm);
    let harvest = g.act(ann, "gather", json!({ "item": "grain", "minutes": 240 }));
    assert_eq!((harvest["qty"].as_i64(), harvest["skill"].as_str()), (Some(22), Some("farming")));

    // Before the purchase the site is closed to her.
    g.walk(ann, &place);
    assert_eq!(g.refused(ann, "open_business", json!({})), ErrorCode::NotAllowed);
    let bought = g.act(ann, "buy_property", json!({ "max_price": price }));
    assert_eq!(
        bought,
        json!({ "site": place.id, "kind": "restaurant", "owner": { "Resident": ann }, "price": price })
    );
    // Without a permit a business does not hire.
    let job = json!({ "wage": WAGE, "slots": 1, "task": "bread" });
    assert_eq!(g.refused(ann, "post_job", job.clone()), ErrorCode::NotAllowed);
    assert_eq!(
        g.act(ann, "open_business", json!({})),
        json!({ "site": place.id, "kind": "restaurant", "fee": 5_000 })
    );
    assert!(g.site_now(place.id).has_tag("open"));
    assert_eq!(g.act(ann, "till_fund", json!({ "amount": 20_000 }))["till"], 20_000);
    assert_eq!(g.cash(ann), ann_cash);
    assert_eq!(g.act(ann, "stock", json!({ "item": "grain", "qty": 22 }))["in_stock"], 22);

    // The job: 3,000 for a shift that bakes bread. It is on the board of
    // the city, above the starter shifts of 2,800.
    let posted = g.act(ann, "post_job", job);
    let job = posted["job"].as_u64().expect("a job id");
    g.step(1);
    let board = g.act(ben, "list_jobs", json!({}));
    assert_eq!(board["jobs"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        (
            board["jobs"][0]["job"].as_u64(),
            board["jobs"][0]["wage"].as_i64(),
            board["jobs"][0]["task"].as_str()
        ),
        (Some(job), Some(WAGE), Some("bread"))
    );
    assert_eq!(board["public_works"]["wage"], 2_800);
    // Ben takes the job at the site and works one shift. The till pays the
    // wage and withholds 10 percent for the city. Twenty minutes for a
    // loaf and two units of grain: eleven loaves, and the grain is used.
    g.walk(ben, &place);
    assert_eq!(g.refused(ben, "work", json!({})), ErrorCode::NotAllowed);
    assert_eq!(g.act(ben, "apply_job", json!({ "job": job }))["wage"], WAGE);
    assert_eq!(g.notice(ann, "economy.worker_hired").data, json!({ "job": job, "site": place.id }));
    let shift = g.act(ben, "work", json!({}));
    assert_eq!(
        shift,
        json!({
            "site": place.id, "job": job, "wage": WAGE, "tax": 300, "net": 2_700,
            "skill": "cooking", "xp": 36, "produced": [{ "item": "bread", "qty": 11 }],
        })
    );
    assert_eq!(g.cash(ben), ben_cash + 2_700);
    let stock = Holder::Site(place.id);
    assert_eq!(
        (g.qty(stock, "bread"), g.qty(stock, "grain"), g.money(stock)),
        (11, 0, 20_000 - WAGE)
    );
    // The worker can put goods in the stock. It cannot take them out.
    assert_eq!(
        g.refused(ben, "unstock", json!({ "item": "bread", "qty": 1 })),
        ErrorCode::NotAllowed
    );

    // Nothing is for sale before the owner sets a price.
    g.walk(cal, &place);
    assert_eq!(g.refused(cal, "buy", json!({ "item": "bread", "qty": 1 })), ErrorCode::NotFound);
    g.act(ann, "set_price", json!({ "item": "bread", "price": PRICE_OF_BREAD }));
    assert_eq!(
        g.section(cal, "business").expect("a business section")["prices"],
        json!([{ "item": "bread", "price": PRICE_OF_BREAD, "qty": 11 }])
    );
    // Cal buys three loaves. The shop pays 5 percent sales tax out of the
    // price. A price above his limit is refused.
    let too_dear = json!({ "item": "bread", "qty": 3, "max_price": 999 });
    assert_eq!(g.refused(cal, "buy", too_dear), ErrorCode::Conflict);
    let sale = g.act(cal, "buy", json!({ "item": "bread", "qty": 3, "max_price": PRICE_OF_BREAD }));
    assert_eq!(
        sale,
        json!({ "site": place.id, "item": "bread", "qty": 3, "unit_price": PRICE_OF_BREAD, "total": 3_000, "tax": 150 })
    );
    assert_eq!((g.cash(cal), g.carried(cal, "bread")), (cal_cash - 3_000, 3));
    assert_eq!(
        g.last_event("economy.goods_sold").data,
        json!({
            "seller": { "Resident": ann }, "buyer": cal, "site": place.id, "city": 0,
            "item": "bread", "qty": 3, "unit_price": PRICE_OF_BREAD, "tax": 150,
        })
    );
    // The till after one day of business: the wage went out, the sale came
    // in, and the sales tax left for the city.
    let till = 20_000 - WAGE + 3_000 - 150;
    assert_eq!(g.money(stock), till);
    // Bread is food.
    assert_eq!(g.act(cal, "eat", json!({ "item": "bread" }))["nutrition"], 300);

    // The start of day 7. The till pays the property tax: 5 basis points
    // of the value. The city pays its five officials at midnight.
    g.run_to(7, 0, 10);
    let tax = price * 5 / 10_000;
    assert_eq!(tax, 2_337);
    assert_eq!(g.money(stock), till - tax);
    let salaries = 8_000 + 2 * 3_000 + 5_000 + 6_000;
    // All that the city got from this business: the price of the site, the
    // permit, the income tax of the wage, the sales tax and the property
    // tax.
    assert_eq!(g.city_treasury(CITY), treasury + price + 5_000 + 300 + 150 + tax - salaries);
    // The owner takes the profit of the bread out of the till.
    let collected = g.act(ann, "till_collect", json!({ "amount": 2_850 }));
    assert_eq!(collected["till"], till - tax - 2_850);
    assert_eq!(g.cash(ann), ann_cash + 2_850);

    // The owner closes the job. The worker loses it.
    g.walk(ann, &place);
    g.act(ann, "close_job", json!({ "job": job }));
    g.step(1);
    assert_eq!(g.section(ben, "job"), None);
    assert_eq!(g.notice(ben, "economy.job_ended").data["reason"], "closed");

    g.audit();
}

#[test]
fn a_restaurant_is_bought_opened_staffed_and_its_taxes_reach_the_city() {
    assert_deterministic_in_republic(&Setup::town(1), story);
}
