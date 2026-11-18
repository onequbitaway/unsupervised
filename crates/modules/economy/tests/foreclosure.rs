//! Foreclosure after a loan default, and what happens to the ties of a
//! resident that leaves the world.

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::{ErrorCode, Holder, Money, ResidentId};

/// Ada owns a home, an open shop with stock, prices, a till and a worker,
/// and a home in the second zone. Returns Ada and the worker.
fn landlady(t: &mut Town) -> (ResidentId, ResidentId) {
    let ada = t.world.spawn("Ada");
    buy_site(&mut t.world, ada, t.at.home);
    buy_site(&mut t.world, ada, t.at.far_home);
    open_site(&mut t.world, ada, t.at.shop);
    t.world.give_money(ada, Money(1_000));
    ok(&mut t.world, ada, "till_fund", json!({ "amount": 1_000 }));
    t.world.give_item(ada, "bread", 5);
    ok(&mut t.world, ada, "stock", json!({ "item": "bread", "qty": 5 }));
    ok(&mut t.world, ada, "set_price", json!({ "item": "bread", "price": 800 }));
    let job = ok(&mut t.world, ada, "post_job", json!({ "wage": 100, "slots": 1 }))["job"].clone();
    let worker = resident_at(&mut t.world, "Worker", 0, t.at.shop);
    ok(&mut t.world, worker, "apply_job", json!({ "job": job }));
    (ada, worker)
}

#[test]
fn a_loan_default_gives_every_site_of_the_resident_to_the_city() {
    let mut t = Town::new();
    let (ada, worker) = landlady(&mut t);
    // Another owner keeps what it has.
    let bob = t.world.spawn("Bob");
    buy_site(&mut t.world, bob, t.at.farm);
    let owned = section(&t.world, ada, "property").expect("a property section");
    assert_eq!(owned["owned"].as_array().expect("a list").len(), 3);

    // The finance module emits this event in the global realm.
    emit_global(
        &mut t.world,
        "finance.loan_defaulted",
        json!({ "resident": ada, "outstanding": 50_000 }),
    );
    let events = events_of(t.world.step(3));

    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(city()), "{place}");
    }
    assert_eq!(site(&t.world, t.at.farm).owner, Some(Holder::Resident(bob)));
    let mut foreclosed = of_kind(&events, "economy.property_foreclosed");
    foreclosed.sort_by_key(|e| e["site"].as_u64());
    let mut expected: Vec<Value> = [t.at.home, t.at.far_home, t.at.shop]
        .iter()
        .map(|place| {
            json!({
                "site": place, "city": 0, "from": { "Resident": ada }, "reason": "loan_default",
            })
        })
        .collect();
    expected.sort_by_key(|e| e["site"].as_u64());
    assert_eq!(foreclosed, expected);
    assert_eq!(drama_of(&events, "economy.property_foreclosed"), [55, 55, 55]);

    // The shop lost its permit, its prices and its jobs.
    assert!(!site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(
        of_kind(&events, "economy.business_closed"),
        [json!({
            "site": t.at.shop, "owner": { "Resident": ada }, "kind": "shop",
            "reason": "foreclosure",
        })]
    );
    assert_eq!(section(&t.world, worker, "job"), None);
    assert_eq!(t.world.notices(worker).pop().expect("a notice").kind, "economy.job_ended");
    // Ada owns nothing and knows it.
    assert_eq!(section(&t.world, ada, "property"), None);
    let lost: Vec<_> =
        t.world.notices(ada).into_iter().filter(|n| n.kind == "economy.property_lost").collect();
    assert_eq!(lost.len(), 3);
    assert_eq!(lost[0].data["reason"], "loan_default");
    assert_eq!(refused(&mut t.world, ada, "till_collect", json!({})), ErrorCode::NotAllowed);

    // The stock stays with the site. The till goes to the city at midnight.
    assert_eq!(t.world.qty(Holder::Site(t.at.shop), "bread"), 5);
    assert_eq!(till(&t.world, t.at.shop), 1_000);
    let treasury_before = treasury(&t.world);
    next_day(&mut t.world);
    assert_eq!(till(&t.world, t.at.shop), 0);
    assert_eq!(treasury(&t.world), treasury_before + 1_000);
    // The city sells the sites again at their value.
    let carol = resident_at(&mut t.world, "Carol", HOME_VALUE, t.at.home);
    ok(&mut t.world, carol, "buy_property", json!({}));
    assert_only_transfers(&t.world);
}

#[test]
fn a_default_of_a_resident_without_sites_changes_nothing() {
    let mut t = Town::new();
    let (ada, _) = landlady(&mut t);
    let bob = t.world.spawn("Bob");
    let before = section(&t.world, ada, "property");
    emit_global(
        &mut t.world,
        "finance.loan_defaulted",
        json!({ "resident": bob, "outstanding": 1 }),
    );
    let events = events_of(t.world.step(3));
    assert_eq!(of_kind(&events, "economy.property_foreclosed"), Vec::<Value>::new());
    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(Holder::Resident(ada)), "{place}");
    }
    assert!(site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(section(&t.world, ada, "property"), before);
    assert_eq!(before.expect("a property section")["owned"].as_array().map(Vec::len), Some(3));
    assert_only_transfers(&t.world);
}

#[test]
fn the_sites_of_a_dead_resident_go_to_its_heir() {
    let mut t = Town::new();
    let (ada, worker) = landlady(&mut t);
    let heir = t.world.spawn("Heir");

    // The life module emits the death with the heir, then removes the
    // resident.
    emit_global(&mut t.world, "life.resident_died", death(ada, json!({ "Resident": heir })));
    t.world.despawn(ada);
    let events = events_of(t.world.step(4));

    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(Holder::Resident(heir)), "{place}");
    }
    let inherited = of_kind(&events, "economy.property_inherited");
    assert_eq!(inherited.len(), 3);
    assert!(inherited.contains(&json!({
        "site": t.at.shop, "city": 0, "from": ada, "to": { "Resident": heir },
    })));
    // The shop stays open with its prices. The jobs of the dead owner end.
    assert!(site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(section(&t.world, worker, "job"), None);
    go(&mut t.world, heir, t.at.shop);
    let seen = section(&t.world, heir, "business").expect("a business section");
    assert_eq!(seen["prices"][0]["price"], 800);
    assert_eq!(seen["till"], 1_000);
    let owned = section(&t.world, heir, "property").expect("a property section");
    assert_eq!(owned["owned"].as_array().expect("a list").len(), 3);
    ok(&mut t.world, heir, "till_collect", json!({}));
    assert_eq!(cash(&t.world, heir), 1_000);
    assert_only_transfers(&t.world);
}

#[test]
fn the_sites_of_a_resident_that_leaves_without_an_heir_go_to_the_city() {
    let mut t = Town::new();
    let (ada, _) = landlady(&mut t);
    t.world.despawn(ada);
    let events = events_of(t.world.step(4));
    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(city()), "{place}");
    }
    // Nobody runs the shop of the city: the permit ends.
    assert!(!site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(
        of_kind(&events, "economy.business_closed"),
        [json!({
            "site": t.at.shop, "owner": { "Resident": ada }, "kind": "shop", "reason": "estate",
        })]
    );
    let inherited = of_kind(&events, "economy.property_inherited");
    assert_eq!(inherited.len(), 3);
    assert!(inherited.contains(&json!({
        "site": t.at.home, "city": 0, "from": ada, "to": { "Org": 100 },
    })));
    assert_eq!(drama_of(&events, "economy.property_inherited"), [30, 30, 30]);
    assert_only_transfers(&t.world);
}

/// The death of a resident as the life module reports it.
fn death(resident: ResidentId, heir: Value) -> Value {
    json!({ "resident": resident, "cause": "age", "age": 81, "heir": heir, "estate": 0 })
}

#[test]
fn a_firm_inherits_an_open_business_and_a_city_inherits_a_closed_one() {
    // The will names a firm: the shop stays open.
    let mut t = Town::new();
    let (ada, _) = landlady(&mut t);
    emit_global(&mut t.world, "life.resident_died", death(ada, json!({ "Org": 5_000_000 })));
    t.world.despawn(ada);
    let events = events_of(t.world.step(4));
    let firm = Holder::Org(unsup_kernel::OrgId(5_000_000));
    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(firm), "{place}");
    }
    assert!(site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(of_kind(&events, "economy.business_closed"), Vec::<Value>::new());
    assert_only_transfers(&t.world);

    // No heir and no household: the life module names the city. The city
    // runs no business, as after a foreclosure.
    let mut t = Town::new();
    let (ada, _) = landlady(&mut t);
    emit_global(&mut t.world, "life.resident_died", death(ada, json!({ "Org": 100 })));
    t.world.despawn(ada);
    let events = events_of(t.world.step(4));
    for place in [t.at.home, t.at.far_home, t.at.shop] {
        assert_eq!(site(&t.world, place).owner, Some(city()), "{place}");
    }
    assert!(!site(&t.world, t.at.shop).has_tag("open"));
    assert_eq!(of_kind(&events, "economy.business_closed")[0]["reason"], "estate");
    let inherited = of_kind(&events, "economy.property_inherited");
    assert_eq!(inherited.len(), 3);
    assert!(inherited.iter().all(|e| e["to"] == json!({ "Org": 100 })));
    assert_only_transfers(&t.world);
}

/// A stand-in for the life module that reports a death only after the
/// kernel removed the resident: `life.resident_died` follows
/// `kernel.despawned`. The cause of the removal carries the id of the heir.
struct LateLife;

impl unsup_kernel::Plugin for LateLife {
    fn name(&self) -> &'static str {
        "life"
    }

    fn build(&self, b: &mut unsup_kernel::Builder) {
        if b.scope() != unsup_kernel::Scope::Global {
            return;
        }
        b.on_event("kernel.despawned", |realm, event| {
            let resident = ResidentId(event.data["resident"].as_u64().expect("a resident id"));
            let heir: u64 = event.data["cause"].as_str().expect("a cause").parse().expect("an id");
            let meta = unsup_kernel::EventMeta::public("life.resident_died", 80);
            realm.emit(meta, &death(resident, json!({ "Resident": heir })));
        });
    }
}

#[test]
fn the_heir_gets_the_sites_whatever_the_order_of_the_death_and_the_removal() {
    static LATE_LIFE: LateLife = LateLife;
    let mut world = unsup_kernel::testing::TestWorld::new(&[&LATE_LIFE, &ECONOMY]);
    let at = build(&mut world);
    let heir = world.spawn("Heir");
    let [ada, bob] = ["Ada", "Bob"].map(|name| world.spawn(name));
    buy_site(&mut world, ada, at.home);
    buy_site(&mut world, bob, at.farm);
    let to_heir = Holder::Resident(heir);

    // Ada: the event of the death comes first, then the removal.
    world.with_resident(ada, |realm, idx| {
        let meta = unsup_kernel::EventMeta::public("life.resident_died", 80);
        realm.emit(meta, &death(ada, json!({ "Resident": heir })));
        realm.despawn(idx, to_heir, &heir.0.to_string());
    });
    // Bob: the removal comes first. The stand-in reports the death after.
    world.with_resident(bob, |realm, idx| realm.despawn(idx, to_heir, &heir.0.to_string()));
    world.world.settle();
    let events = events_of(world.step(4));

    assert_eq!(site(&world, at.home).owner, Some(to_heir));
    assert_eq!(site(&world, at.farm).owner, Some(to_heir));
    let mut inherited = of_kind(&events, "economy.property_inherited");
    inherited.sort_by_key(|e| e["site"].as_u64());
    assert_eq!(
        inherited,
        [
            json!({ "site": at.farm, "city": 0, "from": bob, "to": { "Resident": heir } }),
            json!({ "site": at.home, "city": 0, "from": ada, "to": { "Resident": heir } }),
        ]
    );
    let owned = section(&world, heir, "property").expect("a property section");
    assert_eq!(owned["owned"].as_array().map(Vec::len), Some(2));
    assert_only_transfers(&world);
}

#[test]
fn a_resident_that_leaves_the_world_frees_its_home_and_its_job() {
    let mut t = Town::new();
    let (ada, worker) = landlady(&mut t);
    ok(&mut t.world, ada, "sell_property", json!({}));
    go(&mut t.world, ada, t.at.home);
    ok(&mut t.world, ada, "rent_out", json!({ "rent": 900 }));
    let tenant = resident_at(&mut t.world, "Tenant", 5_000, t.at.home);
    ok(&mut t.world, tenant, "rent", json!({}));
    assert_eq!(site(&t.world, t.at.home).tenants, [tenant]);

    t.world.despawn(tenant);
    t.world.despawn(worker);
    let events = events_of(t.world.step(4));
    assert!(site(&t.world, t.at.home).tenants.is_empty());
    assert_eq!(
        of_kind(&events, "economy.lease_ended"),
        [json!({ "site": t.at.home, "tenant": tenant, "by": "gone" })]
    );
    assert_eq!(
        of_kind(&events, "economy.worker_left")
            .iter()
            .map(|e| (e["resident"].clone(), e["reason"].clone()))
            .collect::<Vec<_>>(),
        [(json!(worker), json!("gone"))]
    );
    // The home takes a tenant again and the job a worker.
    let next = resident_at(&mut t.world, "Next", 5_000, t.at.home);
    ok(&mut t.world, next, "rent", json!({}));
    go(&mut t.world, next, t.at.shop);
    let seen = section(&t.world, next, "business").expect("a business section");
    assert_eq!(seen["jobs"][0]["free_slots"], 1);
    // Nobody is asked for the rent of the tenant that is gone.
    next_day(&mut t.world);
    assert_eq!(site(&t.world, t.at.home).tenants, [next]);
    assert_only_transfers(&t.world);
}
