//! Permits: `open_business` and `close_business`.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money};

const FEE: i64 = 5_000;

#[test]
fn a_permit_costs_a_fee_and_opens_the_site() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.shop);
    t.world.give_money(ada, Money(FEE));
    assert_eq!(section(&t.world, ada, "business").expect("a section")["open"], false);

    let (result, events) = ok_with_events(&mut t.world, ada, "open_business", json!({}));
    assert_eq!(result, json!({ "site": t.at.shop, "kind": "shop", "fee": FEE }));
    assert!(site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(cash(&t.world, ada), 0);
    t.world.step(1);
    assert_eq!(treasury(&t.world), SHOP_VALUE + FEE);
    assert_eq!(
        of_kind(&events, "economy.business_opened"),
        [json!({ "site": t.at.shop, "owner": { "Resident": ada }, "kind": "shop" })]
    );
    assert_eq!(drama_of(&events, "economy.business_opened"), [25]);
    assert_eq!(section(&t.world, ada, "business").expect("a section")["open"], true);
    // The score is a parameter of the data file.
    t.world.set_param("economy.drama.business_closed", 77);
    t.world.step(1);
    let (_, events) = ok_with_events(&mut t.world, ada, "close_business", json!({}));
    assert_eq!(drama_of(&events, "economy.business_closed"), [77]);
    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", SHOP_VALUE + FEE)]));
    assert_eq!(flows.of_module(), Flows::default());
}

#[test]
fn a_permit_can_change_what_a_site_is() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.office);
    t.world.give_money(ada, Money(2 * FEE));
    // An office becomes a workshop: now recipes work there.
    let result = ok(&mut t.world, ada, "open_business", json!({ "kind": "workshop" }));
    assert_eq!(result["kind"], "workshop");
    assert_eq!(site(&t.world, t.at.office).kind, "workshop");
    t.world.give_item(ada, "cotton", 2);
    ok(&mut t.world, ada, "stock", json!({ "item": "cotton", "qty": 2 }));
    ok(&mut t.world, ada, "produce", json!({ "recipe": "cloth" }));
    let job = ok(
        &mut t.world,
        ada,
        "post_job",
        json!({ "wage": 100, "slots": 1, "task": "cloth" }),
    )["job"]
        .clone();
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.office);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));

    // The workshop becomes a shop: the old jobs end, the stock stays.
    let (_, events) = ok_with_events(&mut t.world, ada, "open_business", json!({ "kind": "shop" }));
    assert_eq!(site(&t.world, t.at.office).kind, "shop");
    assert!(site(&t.world, t.at.office).has_tag("open"));
    assert_eq!(
        of_kind(&events, "economy.business_closed"),
        [json!({
            "site": t.at.office, "owner": { "Resident": ada }, "kind": "workshop",
            "reason": "converted",
        })]
    );
    assert_eq!(
        of_kind(&events, "economy.business_opened"),
        [json!({ "site": t.at.office, "owner": { "Resident": ada }, "kind": "shop" })]
    );
    assert_eq!(section(&t.world, worker, "job"), None);
    assert_eq!(t.world.qty(Holder::Site(t.at.office), "cloth"), 1);
    ok(&mut t.world, ada, "set_price", json!({ "item": "cloth", "price": 900 }));
    assert_eq!(cash(&t.world, ada), 0);
    // One batch of cloth was made before the workshop became a shop.
    assert_eq!(
        flows(&t.world).of_module(),
        module_flows([], [], [("production.output:cloth", 1)], [("production.input:cotton", 2)])
    );
}

#[test]
fn open_business_refuses_what_it_must() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    assert_eq!(refused(&mut t.world, ada, "open_business", json!({})), ErrorCode::NotHere);
    go(&mut t.world, ada, t.at.shop);
    assert_eq!(refused(&mut t.world, ada, "open_business", json!({})), ErrorCode::NotAllowed);
    // A home and a farm cannot be a business.
    for place in [t.at.home, t.at.farm] {
        buy_site(&mut t.world, ada, place);
        assert_eq!(refused(&mut t.world, ada, "open_business", json!({})), ErrorCode::NotAllowed);
    }
    buy_site(&mut t.world, ada, t.at.shop);
    t.world.give_money(ada, Money(FEE - 1));
    let error = t.world.act_err(ada, "open_business", json!({}));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": FEE, "available": FEE - 1 }));
    t.world.give_money(ada, Money(1));
    for kind in ["farm", "home", "castle"] {
        let args = json!({ "kind": kind });
        assert_eq!(refused(&mut t.world, ada, "open_business", args), ErrorCode::InvalidArgs);
    }
    ok(&mut t.world, ada, "open_business", json!({}));
    t.world.give_money(ada, Money(FEE));
    for args in [json!({}), json!({ "kind": "shop" })] {
        assert_eq!(refused(&mut t.world, ada, "open_business", args), ErrorCode::Conflict);
    }
    assert_only_transfers(&t.world);
}

#[test]
fn close_business_ends_the_prices_and_the_jobs_and_keeps_the_stock() {
    let mut t = Town::new();
    let ada = t.world.spawn("Ada");
    let guest = resident_at(&mut t.world, "Guest", 5_000, t.at.shop);
    assert_eq!(refused(&mut t.world, guest, "close_business", json!({})), ErrorCode::NotAllowed);
    buy_site(&mut t.world, ada, t.at.shop);
    assert_eq!(refused(&mut t.world, ada, "close_business", json!({})), ErrorCode::Conflict);
    t.world.give_money(ada, Money(FEE + 700));
    ok(&mut t.world, ada, "open_business", json!({}));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 700 }));
    t.world.give_item(ada, "bread", 4);
    ok(&mut t.world, ada, "stock", json!({ "item": "bread", "qty": 4 }));
    ok(&mut t.world, ada, "set_price", json!({ "item": "bread", "price": 800 }));
    let job = ok(&mut t.world, ada, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    ok(&mut t.world, guest, "apply_job", json!({ "job": job }));

    let (result, events) = ok_with_events(&mut t.world, ada, "close_business", json!({}));
    assert_eq!(result, json!({ "site": t.at.shop, "kind": "shop" }));
    assert_eq!(
        of_kind(&events, "economy.business_closed"),
        [json!({
            "site": t.at.shop, "owner": { "Resident": ada }, "kind": "shop", "reason": "owner",
        })]
    );
    assert!(!site(&t.world, t.at.shop).has_tag("open"));
    // No trade, no job; the stock and the till are still there.
    let bread = json!({ "item": "bread", "qty": 1 });
    assert_eq!(refused(&mut t.world, guest, "buy", bread), ErrorCode::NotAllowed);
    assert_eq!(section(&t.world, guest, "job"), None);
    assert_eq!(t.world.qty(Holder::Site(t.at.shop), "bread"), 4);
    assert_eq!(till(&t.world, t.at.shop), 700);
    // A new permit starts with an empty price list.
    t.world.give_money(ada, Money(FEE));
    ok(&mut t.world, ada, "open_business", json!({}));
    let bread = json!({ "item": "bread", "qty": 1 });
    assert_eq!(refused(&mut t.world, guest, "buy", bread), ErrorCode::NotFound);
    t.world.set_pos(ada, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, ada, "close_business", json!({})), ErrorCode::NotHere);
    assert_only_transfers(&t.world);
}
