//! A restaurant: the purchase, the workers, the goods and the customers.

use serde_json::json;
use unsup_kernel::{Holder, Site};
use unsup_scenarios::republic::grant;
use unsup_scenarios::{Game, Republic};

/// Ann buys the cheapest restaurant of the city, opens it, bakes, hires
/// and fires. Returns the site.
pub fn open(g: &mut Game, cast: &Republic) -> Site {
    let [ann, ben, _, dee] = cast.citizens;
    let board = g.act(ann, "list_property", json!({ "kind": "restaurant", "offer": "sale" }));
    let place =
        g.site_now(unsup_kernel::SiteId(board["listings"][0]["site"].as_u64().expect("a site")));
    let price = place.attr("value");
    // The council raised the permit fee to 6,000 this morning.
    grant(g, cast, ann, price + 6_000 + 30_000);
    let farm = g.site("farm");
    g.walk(ann, &farm);
    // Four hours at the farm give 22 units of grain.
    let grain = g.act(ann, "gather", json!({ "item": "grain", "minutes": 240 }))["qty"].clone();
    assert_eq!(grain, 22);

    g.walk(ann, &place);
    g.act(ann, "buy_property", json!({}));
    assert_eq!(g.act(ann, "open_business", json!({}))["fee"], 6_000);
    g.act(ann, "till_fund", json!({ "amount": 30_000 }));
    g.act(ann, "stock", json!({ "item": "grain", "qty": grain }));
    let recipes = g.act(ann, "list_recipes", json!({ "output": "bread" }));
    assert_eq!(recipes["recipes"][0]["inputs"], json!({ "grain": 2 }));
    // The owner bakes two loaves herself.
    let baked = g.act(ann, "produce", json!({ "recipe": "bread", "batches": 2 }));
    assert_eq!(baked["outputs"], json!([{ "item": "bread", "qty": 2 }]));

    // A job for two. Ben and Dee take it. Dee is fired at once.
    let job = g.act(ann, "post_job", json!({ "wage": 3_000, "slots": 2, "task": "bread" }))["job"]
        .as_u64()
        .expect("a job");
    g.step(1);
    assert_eq!(g.act(ben, "list_jobs", json!({ "kind": "restaurant" }))["jobs"][0]["job"], job);
    g.walk_all(&[ben, dee], &place);
    g.act(ben, "apply_job", json!({ "job": job }));
    g.act(dee, "apply_job", json!({ "job": job }));
    assert_eq!(g.act(ann, "fire", json!({ "resident": dee }))["resident"], json!(dee));
    assert_eq!(g.notice(dee, "economy.job_ended").data["reason"], "fired");
    // Ben works a shift: nine loaves out of the 18 units of grain that are
    // left.
    let shift = g.act(ben, "work", json!({}));
    assert_eq!(shift["produced"], json!([{ "item": "bread", "qty": 9 }]));
    let stock = Holder::Site(place.id);
    assert_eq!((g.qty(stock, "bread"), g.qty(stock, "grain")), (11, 0));
    place
}

/// The second day of the restaurant: prices, customers, the till, and the
/// end of the job.
pub fn trade(g: &mut Game, cast: &Republic, place: &Site) {
    let [ann, ben, cal, _] = cast.citizens;
    g.walk_all(&[ann, ben, cal], place);
    // Bread for 1,000. The restaurant also buys vegetables for 150.
    g.act(ann, "set_price", json!({ "item": "bread", "price": 1_000 }));
    g.act(ann, "set_price", json!({ "item": "vegetables", "buy_price": 150 }));
    assert_eq!(g.act(cal, "buy", json!({ "item": "bread", "qty": 2 }))["total"], 2_000);
    assert_eq!(g.act(cal, "sell", json!({ "item": "vegetables", "qty": 4 }))["total"], 600);
    // The owner takes goods out of the stock, leaves them on the floor and
    // picks them up again.
    g.act(ann, "unstock", json!({ "item": "vegetables", "qty": 4 }));
    g.act(ann, "drop", json!({ "item": "vegetables", "qty": 4 }));
    assert_eq!(g.qty(Holder::Site(place.id), "vegetables"), 4);
    g.act(ann, "pick_up", json!({ "item": "vegetables", "qty": 4 }));
    assert_eq!(g.qty(Holder::Site(place.id), "vegetables"), 0);
    g.act(ann, "till_collect", json!({ "amount": 1_000 }));
    // Ben leaves the job, and Ann closes it.
    let job = g.section(ben, "job").expect("Ben has a job")["job"].as_u64().expect("a job id");
    g.act(ben, "quit_job", json!({}));
    assert_eq!(g.section(ben, "job"), None);
    g.act(ann, "close_job", json!({ "job": job }));
}

/// The last day: the restaurant is for sale, and the owner gives up the
/// permit.
pub fn close(g: &mut Game, cast: &Republic, place: &Site) {
    let ann = cast.citizens[0];
    g.walk(ann, place);
    let offer = g.act(ann, "sell_property", json!({ "price": 5_000_000 }));
    assert_eq!(offer["price"], 5_000_000);
    g.act(ann, "close_business", json!({}));
    assert!(!g.site_now(place.id).has_tag("open"));
}
