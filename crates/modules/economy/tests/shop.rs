//! Shops: `stock`, `unstock`, `set_price`, `buy`, `sell`, the sales tax and
//! the till.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::{ErrorCode, Holder, Money, OrgId, RealmId, ResidentId};

/// A shop with a permit, 20 loaves of bread in its stock, and bread for
/// sale at 1,000 cents. The shop pays 400 cents for a loaf.
fn bakery(t: &mut Town) -> ResidentId {
    let owner = t.world.spawn("Owner");
    open_site(&mut t.world, owner, t.at.shop);
    t.world.give_item(owner, "bread", 20);
    ok(&mut t.world, owner, "stock", json!({ "item": "bread", "qty": 20 }));
    ok(
        &mut t.world,
        owner,
        "set_price",
        json!({ "item": "bread", "price": 1_000, "buy_price": 400 }),
    );
    owner
}

#[test]
fn a_customer_buys_and_the_shop_pays_the_sales_tax() {
    let mut t = Town::new();
    let owner = bakery(&mut t);
    let treasury_before = treasury(&t.world);
    let ada = resident_at(&mut t.world, "Ada", 5_000, t.at.shop);

    // A customer sees the price list.
    let seen = section(&t.world, ada, "business").expect("a business section");
    assert_eq!(
        seen["prices"],
        json!([{ "item": "bread", "qty": 20, "price": 1_000, "buy_price": 400 }])
    );
    assert_eq!(seen["open"], true);
    assert_eq!(seen.get("till"), None, "only the owner sees the till");

    let (result, events) =
        ok_with_events(&mut t.world, ada, "buy", json!({ "item": "bread", "qty": 3 }));
    assert_eq!(
        result,
        json!({
            "site": t.at.shop, "item": "bread", "qty": 3, "unit_price": 1_000, "total": 3_000,
            "tax": 150,
        })
    );
    assert_eq!(cash(&t.world, ada), 2_000);
    assert_eq!(t.world.qty(Holder::Resident(ada), "bread"), 3);
    assert_eq!(t.world.qty(Holder::Site(t.at.shop), "bread"), 17);
    // Five percent of the price goes to the city. The till keeps the rest.
    assert_eq!(till(&t.world, t.at.shop), 2_850);
    t.world.step(1);
    assert_eq!(treasury(&t.world), treasury_before + 150);
    assert_eq!(
        of_kind(&events, "economy.goods_sold"),
        [json!({
            "seller": { "Resident": owner },
            "buyer": ada,
            "site": t.at.shop,
            "city": 0,
            "item": "bread",
            "qty": 3,
            "unit_price": 1_000,
            "tax": 150,
        })]
    );
    assert_eq!(section(&t.world, owner, "business").expect("a section")["till"], 2_850);

    let flows = flows(&t.world);
    assert_eq!(flows.minted, map([("kernel.grant", SHOP_VALUE + 5_000 + 5_000)]));
    assert_eq!(flows.burned, map([]));
    assert_eq!(flows.produced, map([("kernel.grant:bread", 20)]));
    assert_eq!(flows.consumed, map([]));
}

#[test]
fn a_resident_sells_to_a_shop_that_buys() {
    let mut t = Town::new();
    let owner = bakery(&mut t);
    t.world.give_money(owner, Money(3_000));
    ok(&mut t.world, owner, "till_fund", json!({ "amount": 3_000 }));
    let baker = resident_at(&mut t.world, "Baker", 0, t.at.shop);
    t.world.give_item(baker, "bread", 10);

    let (result, events) =
        ok_with_events(&mut t.world, baker, "sell", json!({ "item": "bread", "qty": 5 }));
    assert_eq!(
        result,
        json!({ "site": t.at.shop, "item": "bread", "qty": 5, "unit_price": 400, "total": 2_000 })
    );
    assert_eq!(cash(&t.world, baker), 2_000);
    assert_eq!(till(&t.world, t.at.shop), 1_000);
    assert_eq!(t.world.qty(Holder::Site(t.at.shop), "bread"), 25);
    assert_eq!(t.world.attr(baker, "skill.trading"), 1);
    // The shop is the buyer: the event names its owner, and no tax is due.
    assert_eq!(
        of_kind(&events, "economy.goods_sold"),
        [json!({
            "seller": { "Resident": baker },
            "buyer": owner,
            "site": t.at.shop,
            "city": 0,
            "item": "bread",
            "qty": 5,
            "unit_price": 400,
            "tax": 0,
        })]
    );

    // The till has 1,000 cents left: not enough for three more loaves.
    let error = t.world.act_err(baker, "sell", json!({ "item": "bread", "qty": 3 }));
    assert_eq!(error.code, ErrorCode::Insufficient);
    assert_eq!(error.details, json!({ "needed": 1_200, "available": 1_000 }));
    for (args, code) in [
        (json!({ "item": "bread", "qty": 6 }), ErrorCode::Insufficient),
        (json!({ "item": "grain", "qty": 1 }), ErrorCode::NotFound),
        (json!({ "item": "gold", "qty": 1 }), ErrorCode::NotFound),
        (json!({ "item": "bread", "qty": 0 }), ErrorCode::InvalidArgs),
        (json!({ "item": "bread", "qty": 1, "min_price": 401 }), ErrorCode::Conflict),
    ] {
        assert_eq!(refused(&mut t.world, baker, "sell", args), code);
    }
    ok(&mut t.world, baker, "sell", json!({ "item": "bread", "qty": 1, "min_price": 400 }));
    // Only a shop with a permit buys.
    let one = json!({ "item": "bread", "qty": 1 });
    go(&mut t.world, baker, t.at.far_shop);
    assert_eq!(refused(&mut t.world, baker, "sell", one.clone()), ErrorCode::NotAllowed);
    go(&mut t.world, baker, t.at.farm);
    assert_eq!(refused(&mut t.world, baker, "sell", one.clone()), ErrorCode::NotHere);
    t.world.set_pos(baker, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, baker, "sell", one), ErrorCode::NotHere);
    assert_only_transfers(&t.world);
}

#[test]
fn buy_refuses_what_it_must() {
    let mut t = Town::new();
    bakery(&mut t);
    let ada = t.world.spawn("Ada");
    t.world.give_money(ada, Money(2_500));
    let bread = json!({ "item": "bread", "qty": 1 });

    assert_eq!(refused(&mut t.world, ada, "buy", bread.clone()), ErrorCode::NotHere);
    // A farm does not trade with residents; a shop without a permit neither.
    go(&mut t.world, ada, t.at.farm);
    assert_eq!(refused(&mut t.world, ada, "buy", bread.clone()), ErrorCode::NotHere);
    go(&mut t.world, ada, t.at.far_shop);
    assert_eq!(refused(&mut t.world, ada, "buy", bread.clone()), ErrorCode::NotAllowed);

    go(&mut t.world, ada, t.at.shop);
    for (args, code) in [
        (json!({ "item": "gold", "qty": 1 }), ErrorCode::NotFound),
        (json!({ "item": "meal", "qty": 1 }), ErrorCode::NotFound),
        (json!({ "item": "bread", "qty": 0 }), ErrorCode::InvalidArgs),
        (json!({ "item": "bread", "qty": 1_001 }), ErrorCode::InvalidArgs),
        (json!({ "item": "bread", "qty": 21 }), ErrorCode::Insufficient),
        (json!({ "item": "bread", "qty": 3 }), ErrorCode::Insufficient),
        (json!({ "item": "bread", "qty": 1, "max_price": 999 }), ErrorCode::Conflict),
    ] {
        assert_eq!(refused(&mut t.world, ada, "buy", args.clone()), code, "{args}");
    }
    let error = t.world.act_err(ada, "buy", json!({ "item": "bread", "qty": 3 }));
    assert_eq!(error.details, json!({ "needed": 3_000, "available": 2_500 }));
    ok(&mut t.world, ada, "buy", json!({ "item": "bread", "qty": 2, "max_price": 1_000 }));
    assert_eq!(cash(&t.world, ada), 500);
    assert_only_transfers(&t.world);
}

#[test]
fn two_customers_cannot_both_buy_the_last_unit() {
    let mut t = Town::new();
    let owner = bakery(&mut t);
    ok(&mut t.world, owner, "unstock", json!({ "item": "bread", "qty": 19 }));
    let a = resident_at(&mut t.world, "A", 1_000, t.at.shop);
    let b = resident_at(&mut t.world, "B", 1_000, t.at.shop);
    // Both start in the same tick; the second one finds the shelf empty.
    t.world.act_ok(a, "buy", json!({ "item": "bread", "qty": 1 }));
    t.world.act_ok(b, "buy", json!({ "item": "bread", "qty": 1 }));
    settle(&mut t.world, a);
    settle(&mut t.world, b);
    assert_eq!(t.world.qty(Holder::Resident(a), "bread"), 1);
    assert_eq!(t.world.qty(Holder::Resident(b), "bread"), 0);
    let report = t.world.last_report(b).expect("a report");
    assert_eq!(report.error.expect("an error").code, ErrorCode::Insufficient);
    assert_eq!(cash(&t.world, b), 1_000);
    assert_only_transfers(&t.world);
}

#[test]
fn the_owner_stocks_unstocks_and_sets_prices() {
    let mut t = Town::new();
    let owner = bakery(&mut t);
    let guest = resident_at(&mut t.world, "Guest", 0, t.at.shop);
    t.world.give_item(guest, "bread", 2);

    let result = ok(&mut t.world, owner, "unstock", json!({ "item": "bread", "qty": 5 }));
    assert_eq!(result, json!({ "site": t.at.shop, "item": "bread", "qty": 5, "in_stock": 15 }));
    assert_eq!(t.world.qty(Holder::Resident(owner), "bread"), 5);
    let result = ok(&mut t.world, owner, "stock", json!({ "item": "bread", "qty": 2 }));
    assert_eq!(result["in_stock"], 17);

    // A stranger moves nothing and sets no price.
    for (verb, args) in [
        ("stock", json!({ "item": "bread", "qty": 1 })),
        ("unstock", json!({ "item": "bread", "qty": 1 })),
        ("set_price", json!({ "item": "bread", "price": 1 })),
    ] {
        assert_eq!(refused(&mut t.world, guest, verb, args), ErrorCode::NotAllowed, "{verb}");
    }
    for (verb, args, code) in [
        ("stock", json!({ "item": "gold", "qty": 1 }), ErrorCode::NotFound),
        ("stock", json!({ "item": "bread", "qty": 0 }), ErrorCode::InvalidArgs),
        ("stock", json!({ "item": "bread", "qty": 4 }), ErrorCode::Insufficient),
        ("unstock", json!({ "item": "gold", "qty": 1 }), ErrorCode::NotFound),
        ("unstock", json!({ "item": "bread", "qty": -1 }), ErrorCode::InvalidArgs),
        ("unstock", json!({ "item": "bread", "qty": 18 }), ErrorCode::Insufficient),
        ("set_price", json!({ "item": "gold", "price": 5 }), ErrorCode::NotFound),
        ("set_price", json!({ "item": "bread", "price": 0 }), ErrorCode::InvalidArgs),
        ("set_price", json!({ "item": "bread", "buy_price": 100_000_001 }), ErrorCode::InvalidArgs),
    ] {
        assert_eq!(refused(&mut t.world, owner, verb, args.clone()), code, "{verb} {args}");
    }
    t.world.set_pos(owner, unsup_kernel::Pos::new(60, 60));
    for verb in ["stock", "unstock"] {
        let args = json!({ "item": "bread", "qty": 1 });
        assert_eq!(refused(&mut t.world, owner, verb, args), ErrorCode::NotHere);
    }
    go(&mut t.world, owner, t.at.shop);

    // A worker of the shop can fill the shelves but cannot empty them.
    let job =
        ok(&mut t.world, owner, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    ok(&mut t.world, guest, "apply_job", json!({ "job": job }));
    ok(&mut t.world, guest, "stock", json!({ "item": "bread", "qty": 2 }));
    assert_eq!(
        refused(&mut t.world, guest, "unstock", json!({ "item": "bread", "qty": 1 })),
        ErrorCode::NotAllowed
    );

    // A price list entry without a price goes away.
    ok(&mut t.world, owner, "set_price", json!({ "item": "bread" }));
    assert_eq!(
        refused(&mut t.world, guest, "buy", json!({ "item": "bread", "qty": 1 })),
        ErrorCode::NotFound
    );
    assert_eq!(section(&t.world, guest, "business").expect("a section").get("prices"), None);
    assert_only_transfers(&t.world);
}

#[test]
fn set_price_needs_a_shop_with_a_permit() {
    let mut t = Town::new();
    let owner = t.world.spawn("Owner");
    let price = json!({ "item": "bread", "price": 900 });
    assert_eq!(refused(&mut t.world, owner, "set_price", price.clone()), ErrorCode::NotHere);
    // A farm is not a shop.
    buy_site(&mut t.world, owner, t.at.farm);
    assert_eq!(refused(&mut t.world, owner, "set_price", price.clone()), ErrorCode::NotHere);
    // A shop without a permit does not trade.
    buy_site(&mut t.world, owner, t.at.shop);
    assert_eq!(refused(&mut t.world, owner, "set_price", price.clone()), ErrorCode::NotAllowed);
    t.world.give_money(owner, Money(5_000));
    ok(&mut t.world, owner, "open_business", json!({}));
    let result = ok(&mut t.world, owner, "set_price", price);
    assert_eq!(
        result,
        json!({ "site": t.at.shop, "item": "bread", "price": 900, "buy_price": null })
    );
    assert_only_transfers(&t.world);
}

#[test]
fn the_owner_moves_cash_between_the_till_and_its_pocket() {
    let mut t = Town::new();
    let owner = bakery(&mut t);
    let guest = resident_at(&mut t.world, "Guest", 1_000, t.at.shop);
    t.world.give_money(owner, Money(4_000));

    let result = ok(&mut t.world, owner, "till_fund", json!({ "amount": 3_000 }));
    assert_eq!(result, json!({ "site": t.at.shop, "amount": 3_000, "till": 3_000 }));
    assert_eq!(cash(&t.world, owner), 1_000);
    let result = ok(&mut t.world, owner, "till_collect", json!({ "amount": 500 }));
    assert_eq!(
        result,
        json!({ "site": t.at.shop, "amount": 500, "to": { "Resident": owner }, "till": 2_500 })
    );
    assert_eq!((cash(&t.world, owner), till(&t.world, t.at.shop)), (1_500, 2_500));

    for (verb, args, code) in [
        ("till_collect", json!({ "amount": 2_501 }), ErrorCode::Insufficient),
        ("till_collect", json!({ "amount": 0 }), ErrorCode::InvalidArgs),
        ("till_fund", json!({ "amount": 1_501 }), ErrorCode::Insufficient),
        ("till_fund", json!({ "amount": 0 }), ErrorCode::InvalidArgs),
        ("till_fund", json!({ "amount": -5 }), ErrorCode::InvalidArgs),
    ] {
        assert_eq!(refused(&mut t.world, owner, verb, args.clone()), code, "{verb} {args}");
    }
    for (verb, args) in [("till_collect", json!({})), ("till_fund", json!({ "amount": 10 }))] {
        assert_eq!(refused(&mut t.world, guest, verb, args), ErrorCode::NotAllowed, "{verb}");
    }
    // Without an amount, everything.
    ok(&mut t.world, owner, "till_collect", json!({}));
    assert_eq!((cash(&t.world, owner), till(&t.world, t.at.shop)), (4_000, 0));
    // An empty till gives nothing.
    assert_eq!(refused(&mut t.world, owner, "till_collect", json!({})), ErrorCode::Insufficient);
    t.world.set_pos(owner, unsup_kernel::Pos::new(60, 60));
    assert_eq!(refused(&mut t.world, owner, "till_collect", json!({})), ErrorCode::NotHere);
    assert_eq!(
        refused(&mut t.world, owner, "till_fund", json!({ "amount": 1 })),
        ErrorCode::NotHere
    );
    assert_only_transfers(&t.world);
}

#[test]
fn the_till_of_a_firm_goes_to_its_treasury() {
    let mut t = Town::new();
    let firm = OrgId(5_000_000);
    let boss = t.world.spawn("Boss");
    t.world.with_resident(boss, |realm, _| realm.grant(boss, firm, "owner", &["site.manage"]));
    t.world.give_money(boss, Money(SHOP_VALUE + 2_000));
    go(&mut t.world, boss, t.at.shop);
    ok(&mut t.world, boss, "buy_property", json!({ "org": firm }));
    ok(&mut t.world, boss, "till_fund", json!({ "amount": 2_000 }));

    let result = ok(&mut t.world, boss, "till_collect", json!({ "amount": 1_500 }));
    assert_eq!(result["to"], json!({ "Org": firm }));
    t.world.step(1);
    assert_eq!(cash(&t.world, boss), 0);
    assert_eq!(till(&t.world, t.at.shop), 500);
    assert_eq!(money_in(&t.world, RealmId::Global, Holder::Org(firm)), 1_500);
    assert_only_transfers(&t.world);
}
