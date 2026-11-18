//! Every event of the module has the drama score of its own parameter
//! (`docs/design/game-design.md`, section 6).

mod common;

use common::*;
use serde_json::{Value, json};
use unsup_kernel::protocol::Event;
use unsup_kernel::{Money, ResidentId};

/// What an event is scored as, and the parameter with its score. A worker
/// that is fired has a score of its own; the event is `economy.worker_left`.
const SCORES: [(&str, &str); 16] = [
    ("economy.shift_completed", "economy.drama.shift_completed"),
    ("economy.goods_sold", "economy.drama.goods_sold"),
    ("economy.goods_produced", "economy.drama.goods_produced"),
    ("economy.property_sold", "economy.drama.property_sold"),
    ("economy.lease_started", "economy.drama.lease_started"),
    ("economy.tenant_evicted", "economy.drama.tenant_evicted"),
    ("economy.business_opened", "economy.drama.business_opened"),
    ("economy.business_closed", "economy.drama.business_closed"),
    ("economy.job_posted", "economy.drama.job_posted"),
    ("economy.worker_hired", "economy.drama.worker_hired"),
    ("economy.worker_left", "economy.drama.worker_left"),
    ("economy.worker_fired", "economy.drama.worker_fired"),
    ("economy.shift_unpaid", "economy.drama.shift_unpaid"),
    ("economy.lease_ended", "economy.drama.lease_ended"),
    ("economy.property_foreclosed", "economy.drama.property_foreclosed"),
    ("economy.property_inherited", "economy.drama.property_inherited"),
];

fn scored_as(event: &Event) -> &str {
    if event.kind == "economy.worker_left" && event.data["reason"] == "fired" {
        "economy.worker_fired"
    } else {
        &event.kind
    }
}

/// A town in which things happen, with a record of the public events.
struct Story {
    t: Town,
    events: Vec<Event>,
}

impl Story {
    /// Runs an action that must succeed and keeps its events.
    fn does(&mut self, r: ResidentId, verb: &str, args: Value) -> Value {
        let (result, events) = ok_with_events(&mut self.t.world, r, verb, args);
        self.events.extend(events);
        result
    }

    /// Lets a resident buy a site of the city with money that it gets.
    fn buys(&mut self, r: ResidentId, place: unsup_kernel::SiteId) {
        let price = site(&self.t.world, place).attr("value");
        self.t.world.give_money(r, Money(price));
        go(&mut self.t.world, r, place);
        self.does(r, "buy_property", json!({}));
    }
}

#[test]
fn every_event_has_the_drama_score_of_its_own_parameter() {
    let mut s = Story { t: Town::new(), events: Vec::new() };
    // Every score gets a value of its own, so that no event can pass with
    // the score of another event.
    let score = |n: usize| 40 + i64::try_from(n).expect("a small number");
    for (n, (_, param)) in SCORES.iter().enumerate() {
        s.t.world.set_param(param, score(n));
    }
    s.t.world.step(1);
    let at = s.t.at;

    // A farm with a job: a sale of property, a job, a hire, a paid shift.
    let owner = s.t.world.spawn("Owner");
    s.buys(owner, at.farm);
    s.t.world.give_money(owner, Money(6_000));
    s.does(owner, "till_fund", json!({ "amount": 3_000 }));
    let job = s.does(owner, "post_job", json!({ "wage": 3_000, "slots": 2 }))["job"].clone();
    let worker = resident_at(&mut s.t.world, "Worker", 0, at.farm);
    s.does(worker, "apply_job", json!({ "job": job }));
    s.does(worker, "work", json!({}));
    // A shift that the till cannot pay at its end.
    s.does(owner, "till_fund", json!({ "amount": 3_000 }));
    s.t.world.act_ok(worker, "work", json!({}));
    s.does(owner, "till_collect", json!({}));
    let unpaid = settle(&mut s.t.world, worker);
    s.events.extend(unpaid);
    // One worker is fired, another one quits.
    s.does(owner, "fire", json!({ "resident": worker }));
    let other = resident_at(&mut s.t.world, "Other", 0, at.farm);
    s.does(other, "apply_job", json!({ "job": job }));
    s.does(other, "quit_job", json!({}));

    // A shop: a permit, a sale of goods, the end of the permit.
    s.buys(owner, at.shop);
    s.t.world.give_money(owner, Money(5_000));
    s.does(owner, "open_business", json!({}));
    s.t.world.give_item(owner, "bread", 1);
    s.does(owner, "stock", json!({ "item": "bread", "qty": 1 }));
    s.does(owner, "set_price", json!({ "item": "bread", "price": 1_000 }));
    go(&mut s.t.world, other, at.shop);
    s.t.world.give_money(other, Money(1_000));
    s.does(other, "buy", json!({ "item": "bread", "qty": 1 }));
    s.does(owner, "close_business", json!({}));

    // A home: a lease that the tenant ends, and a lease that ends with an
    // eviction on the third missed day.
    s.buys(owner, at.home);
    s.does(owner, "rent_out", json!({ "rent": 1_500 }));
    go(&mut s.t.world, other, at.home);
    s.t.world.give_money(other, Money(1_500));
    s.does(other, "rent", json!({}));
    s.does(other, "end_lease", json!({}));
    let tenant = resident_at(&mut s.t.world, "Tenant", 1_500, at.home);
    s.does(tenant, "rent", json!({}));

    // An owner that leaves the world without an heir.
    let gone = s.t.world.spawn("Gone");
    s.buys(gone, at.mine);
    s.t.world.despawn(gone);
    let frames = s.t.world.step(4);
    s.events.extend(events_of(frames));

    // Four midnights: the eviction on the third, and on the fourth the city
    // takes the farm, whose till never paid the property tax.
    for _ in 0..4 {
        let day = next_day(&mut s.t.world);
        s.events.extend(day);
    }
    assert_eq!(site(&s.t.world, at.farm).owner, Some(city()));

    let mine: Vec<&Event> = s.events.iter().filter(|e| e.kind.starts_with("economy.")).collect();
    for (n, (name, param)) in SCORES.iter().enumerate() {
        let scores: Vec<i64> =
            mine.iter().filter(|e| scored_as(e) == *name).map(|e| i64::from(e.drama)).collect();
        assert!(!scores.is_empty(), "the story has no event `{name}`");
        assert!(scores.iter().all(|s| *s == score(n)), "`{name}` must have `{param}`: {scores:?}");
    }
    // No event of the module is without a score parameter.
    for event in &mine {
        assert!(SCORES.iter().any(|(name, _)| *name == scored_as(event)), "{}", event.kind);
    }
    // One paid shift gathered grain. Everything else was a transfer.
    assert_eq!(
        flows(&s.t.world).of_module(),
        module_flows([], [], [("nature.harvest:grain", 24)], [])
    );
}
