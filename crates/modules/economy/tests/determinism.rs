//! The same seed and the same inputs give the same state, also across a
//! snapshot.

mod common;

use common::*;
use serde_json::json;
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Money, ResidentId, WorldConfig};

/// The residents of the scenario, in the order in which `setup` spawns
/// them. Resident ids are the same in every new world.
#[derive(Clone, Copy)]
struct Cast {
    owner: ResidentId,
    worker: ResidentId,
    tenant: ResidentId,
    idler: ResidentId,
}

/// Builds the town and brings it into a state with every kind of economy
/// state: an owner with sites, an open shop, a job with a worker, a lease,
/// a price list and a node that was used.
fn setup(w: &mut TestWorld<'static>) -> (Sites, Cast) {
    let at = build(w);
    let owner = w.spawn("Owner");
    buy_site(w, owner, at.farm);
    buy_site(w, owner, at.home);
    open_site(w, owner, at.shop);
    w.give_money(owner, Money(20_000));
    ok(w, owner, "till_fund", json!({ "amount": 12_000 }));
    w.give_item(owner, "bread", 9);
    ok(w, owner, "stock", json!({ "item": "bread", "qty": 9 }));
    ok(w, owner, "set_price", json!({ "item": "bread", "price": 900, "buy_price": 300 }));
    go(w, owner, at.farm);
    ok(w, owner, "till_fund", json!({ "amount": 8_000 }));
    let job = ok(w, owner, "post_job", json!({ "wage": 2_500, "slots": 2 }))["job"].clone();
    go(w, owner, at.home);
    ok(w, owner, "rent_out", json!({ "rent": 600 }));

    let worker = resident_at(w, "Worker", 0, at.farm);
    ok(w, worker, "apply_job", json!({ "job": job }));
    let tenant = resident_at(w, "Tenant", 3_000, at.home);
    ok(w, tenant, "rent", json!({}));
    let idler = resident_at(w, "Idler", 0, at.city_hall);
    (at, Cast { owner, worker, tenant, idler })
}

/// What the residents do after the setup: shifts in progress, trade, a
/// day change with rent and tax, and a foreclosure.
fn run(w: &mut TestWorld<'static>, at: Sites, cast: Cast) {
    let Cast { owner, worker, tenant, idler } = cast;
    w.act_ok(worker, "work", json!({}));
    w.act_ok(idler, "work", json!({}));
    go(w, tenant, at.shop);
    ok(w, tenant, "buy", json!({ "item": "bread", "qty": 2 }));
    go(w, tenant, at.forest);
    ok(w, tenant, "gather", json!({ "item": "herbs", "minutes": 120 }));
    settle(w, worker);
    settle(w, idler);
    go(w, idler, at.depot);
    ok(w, idler, "buy", json!({ "item": "grain", "qty": 2 }));
    next_day(w);
    emit_global(w, "finance.loan_defaulted", json!({ "resident": owner, "outstanding": 1 }));
    w.step(5);
}

#[test]
fn the_same_inputs_give_the_same_state_also_after_a_snapshot() {
    // The ids of the sites and of the residents do not depend on the seed.
    let (at, cast) = setup(&mut TestWorld::with_seed(&[&ECONOMY], 3));
    let hash = assert_deterministic(
        &[&ECONOMY],
        &WorldConfig::with_seed(3),
        |w| {
            setup(w);
        },
        |w| run(w, at, cast),
    );
    assert_eq!(hash.len(), 64);
}

#[test]
fn a_snapshot_in_the_middle_of_a_shift_changes_nothing() {
    let play = |restore_at: Option<u32>| {
        let mut w = TestWorld::with_seed(&[&ECONOMY], 11);
        let (at, cast) = setup(&mut w);
        w.act_ok(cast.worker, "work", json!({}));
        w.act_ok(cast.idler, "work", json!({}));
        go(&mut w, cast.tenant, at.farm);
        for frame in 0..300 {
            if restore_at == Some(frame) {
                // Both shifts run, and the public works claim is on its way.
                w.snapshot_restore();
            }
            w.step(1);
        }
        next_day(&mut w);
        let flows = flows(&w);
        (w.hash(), cash(&w, cast.worker), cash(&w, cast.idler), till(&w, at.home), flows)
    };
    let plain = play(None);
    // The worker and the idler were paid; the home got a day of rent and
    // paid a day of tax.
    assert_eq!((plain.1, plain.2), (2_250, 1_800));
    assert_eq!(plain.3, 600 + 600 - 100);
    // One starter shift was paid and one shift at the farm gathered grain.
    assert_eq!(
        plain.4.clone().of_module(),
        module_flows([("gov.public_works", 2_000)], [], [("nature.harvest:grain", 24)], [])
    );
    for frame in [0, 1, 2, 100, 239] {
        assert_eq!(play(Some(frame)), plain, "a restore at frame {frame} changed the outcome");
    }
}

#[test]
fn a_world_with_other_inputs_has_another_state() {
    let play = |wage: i64| {
        let mut w = TestWorld::with_seed(&[&ECONOMY], 5);
        let at = build(&mut w);
        let owner = w.spawn("Owner");
        buy_site(&mut w, owner, at.farm);
        ok(&mut w, owner, "post_job", json!({ "wage": wage, "slots": 1 }));
        w.hash()
    };
    assert_eq!(play(1_000), play(1_000));
    assert_ne!(play(1_000), play(1_001));
}
