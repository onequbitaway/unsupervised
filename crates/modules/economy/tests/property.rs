//! Property: `buy_property`, `sell_property` and the daily property tax.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId};

#[test]
fn a_resident_buys_a_home_from_the_city() {
    let mut t = Town::new();
    let ada = resident_at(&mut t.world, "Ada", 250_000, t.at.home);
    let seen = section(&t.world, ada, "business").expect("a business section");
    assert_eq!(seen["for_sale"], HOME_VALUE);

    let (result, events) = ok_with_events(&mut t.world, ada, "buy_property", json!({}));
    assert_eq!(
        result,
        json!({
            "site": t.at.home, "kind": "home", "price": HOME_VALUE, "owner": { "Resident": ada },
        })
    );
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(ada)));
    assert_eq!(cash(&t.world, ada), 50_000);
    t.world.step(1);
    assert_eq!(treasury(&t.world), HOME_VALUE);
    assert_eq!(
        of_kind(&events, "economy.property_sold"),
        [json!({
            "site": t.at.home,
            "city": 0,
            "from": { "Org": 100 },
            "to": { "Resident": ada },
            "price": HOME_VALUE,
        })]
    );
    assert_eq!(drama_of(&events, "economy.property_sold"), [30]);
    assert_eq!(
        section(&t.world, ada, "property"),
        Some(json!({ "owned": [{ "site": t.at.home, "zone": 0, "kind": "home", "till": 0 }] }))
    );
    // A site with an owner is not for sale until the owner says so.
    let bob = resident_at(&mut t.world, "Bob", 250_000, t.at.home);
    assert_eq!(refused(&mut t.world, bob, "buy_property", json!({})), ErrorCode::NotAllowed);
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", 500_000)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn two_buyers_cannot_both_buy_the_same_site() {
    let mut t = Town::new();
    let [a, b] = ["A", "B"].map(|name| resident_at(&mut t.world, name, HOME_VALUE, t.at.home));
    // Both start in the same tick. The site is still for sale for both.
    t.world.act_ok(a, "buy_property", json!({}));
    t.world.act_ok(b, "buy_property", json!({}));
    settle(&mut t.world, a);
    settle(&mut t.world, b);
    // The first one owns the home. The second one keeps its money.
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(a)));
    let report = t.world.last_report(b).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::NotAllowed);
    assert_eq!((cash(&t.world, a), cash(&t.world, b)), (0, HOME_VALUE));
    t.world.step(1);
    assert_eq!(treasury(&t.world), HOME_VALUE);
    assert_eq!(section(&t.world, b, "property"), None);
    assert_only_transfers(&t.world);
}

#[test]
fn buy_property_refuses_what_it_must() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    t.world.give_money(ada, Money(HOME_VALUE - 1));
    assert_eq!(refused(&mut t.world, ada, "buy_property", json!({})), ErrorCode::NotHere);
    // A public building is never sold.
    for public in [t.at.city_hall, t.at.depot, t.at.park] {
        go(&mut t.world, ada, public);
        assert_eq!(refused(&mut t.world, ada, "buy_property", json!({})), ErrorCode::NotAllowed);
    }
    go(&mut t.world, ada, t.at.home);
    let error = t.world.act_err(ada, "buy_property", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": HOME_VALUE, "available": HOME_VALUE - 1 }));
    t.world.give_money(ada, Money(1));
    let limit = json!({ "max_price": HOME_VALUE - 1 });
    assert_eq!(refused(&mut t.world, ada, "buy_property", limit), ErrorCode::Conflict);
    // For an organization, the buyer needs `site.manage` there.
    let firm = json!({ "org": 5_000_000 });
    assert_eq!(refused(&mut t.world, ada, "buy_property", firm), ErrorCode::NotAllowed);

    ok(&mut t.world, ada, "buy_property", json!({ "max_price": HOME_VALUE }));
    assert_eq!(refused(&mut t.world, ada, "buy_property", json!({})), ErrorCode::Conflict);
    assert_only_transfers(&t.world);
}

#[test]
fn an_owner_sells_to_another_resident() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    let bob = resident_at(&mut t.world, "Bob", 300_000, t.at.home);

    let listed = ok(&mut t.world, ada, "sell_property", json!({ "price": 300_000 }));
    assert_eq!(listed, json!({ "site": t.at.home, "price": 300_000 }));
    assert_eq!(section(&t.world, bob, "business").expect("a section")["for_sale"], 300_000);
    let owned = section(&t.world, ada, "property").expect("a property section");
    assert_eq!(owned["owned"][0]["price"], 300_000);

    let (_, events) = ok_with_events(&mut t.world, bob, "buy_property", json!({}));
    // Cash from the buyer to the seller, in the same place.
    assert_eq!((cash(&t.world, ada), cash(&t.world, bob)), (300_000, 0));
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(bob)));
    assert_eq!(site(&t.world, t.at.home).attr("price"), 0, "the offer ended with the sale");
    assert_eq!(
        of_kind(&events, "economy.property_sold"),
        [json!({
            "site": t.at.home,
            "city": 0,
            "from": { "Resident": ada },
            "to": { "Resident": bob },
            "price": 300_000,
        })]
    );
    assert_eq!(section(&t.world, ada, "property"), None);
    assert_eq!(
        section(&t.world, bob, "property").expect("a section")["owned"][0]["site"],
        json!(t.at.home)
    );
    let notice = t.world.notices(ada).pop().expect("a notice");
    assert_eq!((notice.kind.as_str(), notice.from), ("economy.property_sold", Some(bob)));
    assert_eq!(notice.data, json!({ "site": t.at.home, "price": 300_000 }));
    assert_only_transfers(&t.world);
}

#[test]
fn a_firm_sells_a_site_and_its_treasury_gets_the_price() {
    let mut t = Town::new();
    let firm = OrgId(5_000_000);
    let boss = t.world.spawn("Boss");
    t.world.with_resident(boss, |realm, _| realm.grant(boss, firm, "owner", &["site.manage"]));
    t.world.give_money(boss, Money(FARM_VALUE));
    go(&mut t.world, boss, t.at.farm);
    ok(&mut t.world, boss, "buy_property", json!({ "org": firm }));
    // The firm owns the farm. The boss does not: it has no property section.
    assert_eq!(section(&t.world, boss, "property"), None);

    // A member with `site.manage` offers the farm of the firm.
    ok(&mut t.world, boss, "sell_property", json!({ "price": 77_000 }));
    let bob = resident_at(&mut t.world, "Bob", 77_000, t.at.farm);
    let (_, events) = ok_with_events(&mut t.world, bob, "buy_property", json!({}));
    assert_eq!(
        of_kind(&events, "economy.property_sold"),
        [json!({
            "site": t.at.farm,
            "city": 0,
            "from": { "Org": 5_000_000 },
            "to": { "Resident": bob },
            "price": 77_000,
        })]
    );
    // The price is in the treasury of the firm, not with the member.
    t.world.step(1);
    assert_eq!(money_in(&t.world, unsup_kernel::RealmId::Global, Holder::Org(firm)), 77_000);
    assert_eq!((cash(&t.world, boss), cash(&t.world, bob)), (0, 0));
    // The firm does not control the farm any more.
    assert_eq!(
        refused(&mut t.world, boss, "sell_property", json!({ "price": 1 })),
        ErrorCode::NotAllowed
    );
    assert_only_transfers(&t.world);
}

#[test]
fn a_seller_in_another_zone_gets_the_price_by_wire() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    ok(&mut t.world, ada, "sell_property", json!({ "price": 123_456 }));
    go(&mut t.world, ada, t.at.far_home);
    let bob = resident_at(&mut t.world, "Bob", 123_456, t.at.home);
    ok(&mut t.world, bob, "buy_property", json!({}));
    t.world.step(2);
    assert_eq!(cash(&t.world, ada), 123_456);
    // The seller learns that the site is gone, in whatever zone it is.
    assert_eq!(section(&t.world, ada, "property"), None);
    assert!(t.world.notices(ada).iter().any(|n| n.kind == "economy.property_sold"));
    assert_only_transfers(&t.world);
}

#[test]
fn sell_property_refuses_what_it_must_and_an_offer_can_end() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    let price = json!({ "price": 1_000 });
    assert_eq!(refused(&mut t.world, ada, "sell_property", price.clone()), ErrorCode::NotHere);
    go(&mut t.world, ada, t.at.home);
    assert_eq!(refused(&mut t.world, ada, "sell_property", price.clone()), ErrorCode::NotAllowed);
    buy_site(&mut t.world, ada, t.at.home);
    for bad in [json!({ "price": 0 }), json!({ "price": 100_000_001 })] {
        assert_eq!(refused(&mut t.world, ada, "sell_property", bad), ErrorCode::InvalidArgs);
    }
    // An officer of the city controls the city hall, but it is public.
    let mayor = resident_at(&mut t.world, "Mayor", 0, t.at.city_hall);
    let gov = OrgId::city(CITY);
    t.world.with_resident(mayor, |realm, _| realm.grant(mayor, gov, "mayor", &["site.manage"]));
    assert_eq!(refused(&mut t.world, mayor, "sell_property", price.clone()), ErrorCode::NotAllowed);

    // The owner ends the offer: nobody can buy.
    ok(&mut t.world, ada, "sell_property", price);
    let bob = resident_at(&mut t.world, "Bob", 5_000, t.at.home);
    assert_eq!(section(&t.world, bob, "business").expect("a section")["for_sale"], 1_000);
    ok(&mut t.world, ada, "sell_property", json!({}));
    assert_eq!(refused(&mut t.world, bob, "buy_property", json!({})), ErrorCode::NotAllowed);
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(ada)));
    assert_only_transfers(&t.world);
}

#[test]
fn a_sale_ends_the_jobs_but_the_business_goes_on() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    open_site(&mut t.world, ada, t.at.shop);
    t.world.give_item(ada, "bread", 3);
    ok(&mut t.world, ada, "stock", json!({ "item": "bread", "qty": 3 }));
    ok(&mut t.world, ada, "set_price", json!({ "item": "bread", "price": 700 }));
    let job = ok(&mut t.world, ada, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.shop);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    ok(&mut t.world, ada, "sell_property", json!({ "price": 10_000 }));

    let bob = resident_at(&mut t.world, "Bob", 10_700, t.at.shop);
    ok(&mut t.world, bob, "buy_property", json!({}));
    // The new owner has an open shop with its stock and its prices.
    assert!(site(&t.world, t.at.shop).has_tag("open"));
    ok(&mut t.world, bob, "buy", json!({ "item": "bread", "qty": 1 }));
    assert_eq!(t.world.qty(Holder::Site(t.at.shop), "bread"), 2);
    // The worker of the old owner lost the job.
    assert_eq!(section(&t.world, worker, "job"), None);
    assert_eq!(refused(&mut t.world, worker, "work", json!({})), ErrorCode::NotAllowed);
    assert_only_transfers(&t.world);
}

#[test]
fn a_site_pays_property_tax_each_day_from_its_till() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    t.world.give_money(ada, Money(250));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 250 }));
    // Five basis points of 200,000 cents: 100 cents a day.
    let paid_for_the_home = HOME_VALUE;

    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 150);
    assert_eq!(treasury(&t.world), paid_for_the_home + 100);
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 50);
    assert_eq!(site(&t.world, t.at.home).attr("tax_due"), 0);

    // The third day the till is short: it pays what it has and owes the rest.
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 0);
    assert_eq!(treasury(&t.world), paid_for_the_home + 250);
    assert_eq!(site(&t.world, t.at.home).attr("tax_due"), 50);
    let notice = t.world.notices(ada).pop().expect("a notice");
    assert_eq!(notice.kind, "economy.tax_due");
    assert_eq!(notice.data, json!({ "site": t.at.home, "owed": 50, "days_left": 2 }));
    let owned = section(&t.world, ada, "property").expect("a property section");
    assert_eq!(owned["owned"][0]["tax_due"], 50);

    // The owner pays the debt and the tax of the next day.
    t.world.give_money(ada, Money(150));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 150 }));
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 0);
    assert_eq!(site(&t.world, t.at.home).attr("tax_due"), 0);
    assert_eq!(site(&t.world, t.at.home).attr("tax_days"), 0);
    assert_eq!(treasury(&t.world), paid_for_the_home + 400);
    // The sites of the city pay no tax: only this home did.
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", HOME_VALUE + 400)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn the_city_takes_a_site_that_owes_tax_for_too_long() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    // Three days of grace: the site owes on day 1, 2 and 3.
    for day in 1..=3 {
        let events = next_day(&mut t.world);
        assert_eq!(site(&t.world, t.at.home).attr("tax_due"), 100 * day);
        assert_eq!(site(&t.world, t.at.home).attr("tax_days"), day);
        assert_eq!(
            of_kind(&events, "economy.property_foreclosed"),
            Vec::<serde_json::Value>::new()
        );
    }
    assert_eq!(site(&t.world, t.at.home).owner, Some(Holder::Resident(ada)));
    // On day 4 the city takes it.
    let events = next_day(&mut t.world);
    assert_eq!(
        of_kind(&events, "economy.property_foreclosed"),
        [json!({
            "site": t.at.home,
            "city": 0,
            "from": { "Resident": ada },
            "reason": "property_tax",
        })]
    );
    let home = site(&t.world, t.at.home);
    assert_eq!(home.owner, Some(city()));
    assert_eq!((home.attr("tax_due"), home.attr("tax_days")), (0, 0));
    assert_eq!(section(&t.world, ada, "property"), None);
    let notice = t.world.notices(ada).pop().expect("a notice");
    assert_eq!(notice.kind, "economy.property_lost");
    assert_eq!(notice.data, json!({ "site": t.at.home, "reason": "property_tax" }));
    // The home is for sale again at its value.
    let bob = resident_at(&mut t.world, "Bob", HOME_VALUE, t.at.home);
    ok(&mut t.world, bob, "buy_property", json!({}));
    assert_only_transfers(&t.world);
}

/// Ada owns a funded home in each of the two zones. Returns the frames at
/// which the first property tax left the till of the near home and of the
/// far home.
fn first_tax_frames(t: &mut Town) -> (u64, u64) {
    let ada = t.world.spawn("Ada");
    for home in [t.at.home, t.at.far_home] {
        buy_site(&mut t.world, ada, home);
        t.world.give_money(ada, Money(500));
        ok(&mut t.world, ada, "till_fund", json!({ "amount": 500 }));
    }
    let (mut near, mut far) = (None, None);
    for _ in 0..DAY + 10 {
        t.world.step(1);
        let frame = t.world.world.frame();
        if till(&t.world, t.at.home) < 500 {
            near.get_or_insert(frame);
        }
        if till(&t.world, t.at.far_home) < 500 {
            far.get_or_insert(frame);
        }
    }
    // One day of tax each, and no more.
    assert_eq!((till(&t.world, t.at.home), till(&t.world, t.at.far_home)), (400, 400));
    (near.expect("the near home paid"), far.expect("the far home paid"))
}

#[test]
fn the_zones_spread_their_daily_work_over_the_first_minutes_of_the_day() {
    // The second zone starts its day one minute after the first zone.
    let mut t = Town::new();
    assert_eq!(zone_of_site(&t.world, t.at.far_home), unsup_kernel::ZoneId(1));
    let (near, far) = first_tax_frames(&mut t);
    assert_eq!(far, near + 1);
    assert_only_transfers(&t.world);

    // Without a spread, every zone works at midnight.
    let mut t = Town::new();
    t.world.set_param("economy.daily_spread_minutes", 1);
    let (near, far) = first_tax_frames(&mut t);
    assert_eq!(far, near);
    assert_only_transfers(&t.world);
}

#[test]
fn a_law_changes_the_property_tax_rate() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    t.world.give_money(ada, Money(1_000));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 1_000 }));
    t.world.set_param("economy.property_tax_bp", 20);
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 600);
    t.world.set_param("economy.property_tax_bp", 0);
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.home), 600);
    assert_only_transfers(&t.world);
}
