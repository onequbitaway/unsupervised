//! The receipt of a completed mission holds the cents that the escrow
//! really paid. The escrow of a mission always holds the whole reward, so a
//! payment that is too small means a fault somewhere else. These tests
//! make the fault with the ledger and check that every answer shows it.

mod common;

use common::{bake, bread_mission, escrow, join, notice, ok, one_event, post, rich, world};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Holder, Money};

/// The organization of the bakers.
const BAKERS: u64 = 5_000;

/// Takes cents out of the escrow of a mission. No rule of the game does
/// that. The test moves the money with the ledger, as a fault would.
fn drain(w: &TestWorld<'_>, mission: u64, cents: i64) {
    w.global()
        .ledger_mut()
        .transfer_money(Holder::Escrow(mission), Holder::Account(77), Money(cents), "test.fault")
        .expect("the escrow holds the cents");
}

#[test]
fn a_receipt_holds_what_the_escrow_really_paid() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 5_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    // Two places of 900 cents: the mission stays open after the first
    // payment, so that Bob can ask what it paid.
    let mut definition = bread_mission(900, 1);
    definition["takers"] = json!(2);
    let mission = post(&mut w, ann, definition);
    ok(&mut w, bob, "mission_accept", json!({ "mission": mission }));
    assert_eq!(escrow(&w, mission), 1_800);

    // The escrow loses all but 400 cents. Bob then meets the objective.
    drain(&w, mission, 1_400);
    bake(&mut w, bob);
    let frames = w.step(2);
    let paid_in = w.world.frame() - 1;
    assert_eq!((w.cash(bob), escrow(&w, mission)), (Money(400), 0));

    // The event, the notices and the answers name the 400 cents that
    // moved, and the 500 cents that are missing.
    let completed = json!({
        "mission": mission,
        "resident": bob,
        "paid": 400,
        "shares": [{ "resident": bob, "amount": 400 }],
        "short": 500,
    });
    assert_eq!(one_event(&frames, "missions.mission_completed"), completed);
    assert_eq!(notice(&w, ann, "missions.mission_paid").data, completed);
    assert_eq!(
        notice(&w, bob, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 400, "points": 3, "short": 500 })
    );
    assert_eq!(
        ok(&mut w, bob, "mission_claim", json!({ "mission": mission })),
        json!({ "mission": mission, "status": "paid", "paid": 400, "frame": paid_in, "short": 500 })
    );
    let listed = ok(&mut w, bob, "mission_list", json!({ "mission": mission }));
    assert_eq!(
        listed["mission"]["you"],
        json!({ "status": "paid", "paid": 400, "frame": paid_in, "short": 500 })
    );
    w.assert_conserved();
}

#[test]
fn a_receipt_of_a_team_holds_what_each_member_really_got() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let bob = rich(&mut w, "Bob", 2, 0);
    let cid = rich(&mut w, "Cid", 3, 0);
    for member in [bob, cid] {
        join(&mut w, member, BAKERS);
    }
    let definition = json!({
        "title": "Bread for the fair",
        "objectives": [
            { "event": "economy.goods_produced", "fields": { "item": "bread" }, "count": 4, "actor": "team" },
        ],
        "mode": "team",
        "reward": 1_000,
        "deadline_in": 600,
        "takers": 2,
    });
    let mission = post(&mut w, ann, definition);
    for member in [bob, cid] {
        ok(&mut w, member, "mission_accept", json!({ "mission": mission, "org": BAKERS }));
    }

    // Bob adds 3 events and Cid 1: their shares are 750 and 250 cents. The
    // escrow has only 800 cents. Bob, the lower id, is paid first.
    drain(&w, mission, 1_200);
    for _ in 0..3 {
        bake(&mut w, bob);
    }
    bake(&mut w, cid);
    let frames = w.step(2);
    let paid_in = w.world.frame() - 1;
    assert_eq!((w.cash(bob), w.cash(cid), escrow(&w, mission)), (Money(750), Money(50), 0));
    assert_eq!(
        one_event(&frames, "missions.mission_completed"),
        json!({
            "mission": mission,
            "resident": bob,
            "org": BAKERS,
            "paid": 800,
            "shares": [{ "resident": bob, "amount": 750 }, { "resident": cid, "amount": 50 }],
            "short": 200,
        })
    );
    // Bob got his whole share. Cid did not, and still has the points.
    assert_eq!(
        notice(&w, bob, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 750, "points": 3 })
    );
    assert_eq!(
        notice(&w, cid, "missions.mission_completed").data,
        json!({ "mission": mission, "paid": 50, "points": 3, "short": 200 })
    );
    // A snapshot keeps the receipt with the cents that are missing.
    w.snapshot_restore();
    let claim = |w: &mut TestWorld<'_>, member| -> Value {
        ok(w, member, "mission_claim", json!({ "mission": mission }))
    };
    assert_eq!(
        claim(&mut w, bob),
        json!({ "mission": mission, "status": "paid", "paid": 750, "frame": paid_in })
    );
    assert_eq!(
        claim(&mut w, cid),
        json!({ "mission": mission, "status": "paid", "paid": 50, "frame": paid_in, "short": 200 })
    );
    w.assert_conserved();
}
