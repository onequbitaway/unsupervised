//! Residents and messages that cross the border between two processes of
//! a cluster.

mod common;

use serde_json::{Value, json};
use unsup_kernel::{Holder, Money, Plugin, RealmId, World};
use unsup_protocol::{ActionStatus, ObserveOpts, ResidentId, ZoneId};
use unsup_runtime::cluster::{ClusterTotals, ValueTotals, WorkerStats};
use unsup_runtime::{RuntimeConfig, SimHandle, start};

use common::cluster::{Setup, do_act, look, notices, residents_per_zone};
use common::roads::{BANK, Route, plugins, world_config};
use common::{Tally, spec};

/// Nine zones of 16 by 16 tiles. Worker 0 hosts the zones 0 to 2, worker 1
/// the zones 3 to 5, worker 2 the zones 6 to 8. Zone 0 is the tiles from
/// (0, 0) to (15, 15); zone 3 is below it, zone 1 is on its right.
fn setup() -> Setup {
    Setup::new(plugins(), world_config(5, 3, 16), 3)
}

fn external_money(world: &World) -> i64 {
    world.audit().expect("the world balances").money.external
}

#[tokio::test]
async fn a_resident_walks_through_three_processes_with_all_it_holds() {
    let cluster = setup().start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    // Ada appears in the middle of zone 0 with the start money of the
    // town. She earns three cents and puts ten in the bank of the global
    // realm.
    for _ in 0..3 {
        do_act(&handle, ada, "chop", json!({})).await;
    }
    do_act(&handle, ada, "deposit", json!({ "amount": 10 })).await;
    let seen = look(&handle, ada).await;
    assert_eq!(seen["self"]["pos"], json!({ "x": 8, "y": 8 }));
    assert_eq!((&seen["self"]["zone"], &seen["self"]["cash"]), (&json!(0), &json!(93)));
    assert_eq!(residents_per_zone(&handle).await, [1, 0, 0, 0, 0, 0, 0, 0, 0]);

    // She walks to (24, 40) in zone 7: 16 tiles to the south-east through
    // zone 4, then 16 tiles to the south. Zone 4 is in the second process,
    // zone 7 in the third.
    do_act(&handle, ada, "walk", json!({ "x": 24, "y": 40 })).await;
    handle.step(8).await.expect("step");
    let seen = look(&handle, ada).await;
    assert_eq!(seen["self"]["pos"], json!({ "x": 16, "y": 16 }));
    assert_eq!(seen["self"]["zone"], json!(4));
    // The walk goes on in the new process: the action moved with her.
    assert_eq!(seen["self"]["activity"]["verb"], json!("walk"));
    assert_eq!(seen["reports"], json!([]));
    assert_eq!(residents_per_zone(&handle).await, [0, 0, 0, 0, 1, 0, 0, 0, 0]);
    let stats = cluster.handle.cluster_stats().await.expect("stats");
    assert_eq!(stats.migrations, 1);
    let per_worker: Vec<u64> = stats.workers.iter().map(|worker| worker.residents).collect();
    assert_eq!(per_worker, [0, 1, 0]);

    handle.step(24).await.expect("step");
    let seen = look(&handle, ada).await;
    assert_eq!(seen["self"]["pos"], json!({ "x": 24, "y": 40 }));
    assert_eq!((&seen["self"]["zone"], &seen["self"]["cash"]), (&json!(7), &json!(93)));
    assert_eq!(seen["self"]["activity"], Value::Null);
    // The town takes one point of energy in every tick. She lost exactly
    // one per frame: no tick was lost or done twice at a border.
    assert_eq!(seen["self"]["attrs"]["energy"], json!(1_000 - 32));
    let reports = seen["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        (&reports[0]["verb"], &reports[0]["ok"], &reports[0]["result"]),
        (&json!("walk"), &json!(true), &json!({ "pos": { "x": 24, "y": 40 } }))
    );
    assert_eq!(residents_per_zone(&handle).await, [0, 0, 0, 0, 0, 0, 0, 1, 0]);

    // The directory of the global realm followed her.
    let stats = cluster.handle.cluster_stats().await.expect("stats");
    assert_eq!((stats.frame, stats.migrations, stats.dropped_messages), (32, 2, 0));
    assert_eq!(
        stats.workers[2],
        WorkerStats {
            slot: 2,
            zones: vec![ZoneId(6), ZoneId(7), ZoneId(8)],
            residents: 1,
            ..stats.workers[2].clone()
        }
    );

    // The union of all processes balances: 103 cents were made, Ada holds
    // 93 and the bank 10, and nothing is on the way.
    let totals = cluster.handle.audit().await.expect("the union balances");
    let money = ValueTotals { held: 103, created: 103, destroyed: 0, in_flight: 0, external: 0 };
    assert_eq!(totals, ClusterTotals { money, items: Default::default() });

    // Each world alone counts what crossed its border.
    let (global, zones) = cluster.stop();
    assert_eq!(zones.len(), 9);
    assert_eq!(external_money(&global), -10, "the deposit came in");
    let external: Vec<i64> = zones.iter().map(external_money).collect();
    assert_eq!(external, [103, 0, 0, 0, 0, 0, 0, -93, 0], "zone 4 passed on what it got");
    let bank = global.realm(RealmId::Global).expect("the global realm").ledger().money(BANK);
    assert_eq!(bank, Money(10));
    assert_eq!(
        global.realm(RealmId::Global).expect("the global realm").directory().zone_of(ada),
        Some(ZoneId(7))
    );

    // Her components came with her.
    let zone = zones[7].realm(RealmId::Zone(ZoneId(7))).expect("zone 7");
    let idx = zone.idx_of(ada).expect("Ada is in zone 7");
    assert_eq!(zone.col::<Tally>()[idx], Tally { chops: 3 });
    assert_eq!(zone.col::<Route>()[idx], Route { steps: 32, ..zone.col::<Route>()[idx].clone() });
    assert_eq!(zone.col::<Route>()[idx].to, None);
    assert_eq!(zone.ledger().money(Holder::Resident(ada)), Money(93));
    for world in zones.iter().take(7) {
        assert_eq!(world.residents(), 0);
    }
}

#[tokio::test]
async fn notices_wires_and_messages_find_a_resident_in_another_process() {
    let cluster = setup().start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    // Ada goes to zone 8 in the third process. Bob stays in zone 0 in the
    // first.
    do_act(&handle, ada, "travel", json!({ "zone": 8 })).await;
    assert_eq!(look(&handle, ada).await["self"]["zone"], json!(8));

    // A notice, a wire and a module message from Bob. They are in the
    // inbox of zone 8 at once and Ada has them after the next tick.
    do_act(&handle, bob, "tell", json!({ "to": ada, "kind": "town.greeting" })).await;
    do_act(&handle, bob, "pay", json!({ "to": ada, "amount": 7 })).await;
    do_act(&handle, bob, "poke", json!({ "to": ada, "n": 5 })).await;
    let seen = look(&handle, ada).await;
    assert_eq!((notices(&seen), &seen["self"]["cash"]), (vec![], &json!(100)));
    assert_eq!(look(&handle, bob).await["self"]["cash"], json!(93), "the money is on its way");
    handle.step(1).await.expect("step");
    let seen = look(&handle, ada).await;
    assert_eq!(notices(&seen), [("town.greeting".to_string(), Some(bob.0))]);
    assert_eq!(seen["self"]["cash"], json!(107));

    // The global realm, in the coordinator, answers a resident in the
    // third process: a verb of the global realm wires money back and
    // sends a receipt.
    do_act(&handle, ada, "deposit", json!({ "amount": 20 })).await;
    let status = do_act(&handle, ada, "withdraw", json!({ "amount": 5 })).await;
    assert_eq!(status, ActionStatus::Pending);
    handle.step(2).await.expect("step");
    let seen = look(&handle, ada).await;
    assert_eq!(seen["self"]["cash"], json!(107 - 20 + 5));
    assert_eq!(
        notices(&seen),
        [("town.greeting".to_string(), Some(bob.0)), ("roads.receipt".to_string(), None)]
    );
    let reports = seen["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_eq!(
        (&reports[0]["verb"], &reports[0]["result"]),
        (&json!("withdraw"), &json!({ "withdrawn": 5 }))
    );

    let stats = cluster.handle.cluster_stats().await.expect("stats");
    assert_eq!(stats.dropped_messages, 0, "the kernel dropped nothing");
    let totals = cluster.handle.audit().await.expect("the union balances");
    assert_eq!((totals.money.held, totals.money.created, totals.money.in_flight), (200, 200, 0));
    let (global, zones) = cluster.stop();
    let bank = global.realm(RealmId::Global).expect("the global realm").ledger().money(BANK);
    assert_eq!(bank, Money(15));
    let zone = zones[8].realm(RealmId::Zone(ZoneId(8))).expect("zone 8");
    assert_eq!(zone.col::<Route>()[zone.idx_of(ada).expect("Ada")].pokes, 5);
}

#[tokio::test]
async fn a_shout_is_heard_across_the_border_of_two_processes() {
    let cluster = setup().start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    // Bob goes to the south edge of zone 0. Ada goes two tiles further,
    // into zone 3 in the second process.
    do_act(&handle, bob, "walk", json!({ "x": 8, "y": 15 })).await;
    do_act(&handle, ada, "walk", json!({ "x": 8, "y": 17 })).await;
    handle.step(10).await.expect("step");
    assert_eq!(look(&handle, ada).await["self"]["zone"], json!(3));
    assert_eq!(look(&handle, bob).await["self"]["zone"], json!(0));
    // Each sees the other across the border.
    let near = handle.observe(bob, ObserveOpts::default()).await.expect("observe");
    assert_eq!(near.sections["nearby"]["residents"][0]["id"], json!(ada));
    assert_eq!(near.sections["nearby"]["residents"][0]["zone"], json!(3));

    do_act(&handle, bob, "shout", json!({})).await;
    handle.step(1).await.expect("step");
    assert_eq!(notices(&look(&handle, ada).await), [("roads.shout".to_string(), Some(bob.0))]);
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").dropped_messages, 0);
    cluster.stop();
}

/// Three realms send Ada something in the same frame: the global realm a
/// receipt, zone 0 and zone 3 a call each. Returns her notices.
async fn three_senders(handle: &SimHandle) -> Vec<(String, Option<u64>)> {
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    let cyd = handle.spawn(spec("Cyd")).await.expect("spawn");
    // Ada was in zone 0 and in zone 3, so both zones know her. She waits
    // in zone 8. Cyd calls from zone 3, Bob from zone 0.
    do_act(handle, ada, "travel", json!({ "zone": 3 })).await;
    do_act(handle, ada, "travel", json!({ "zone": 8 })).await;
    do_act(handle, cyd, "travel", json!({ "zone": 3 })).await;
    do_act(handle, ada, "deposit", json!({ "amount": 10 })).await;
    handle.step(1).await.expect("step");
    // Cyd is first to act, and zone 3 has the higher number. Both calls
    // end in the second frame from now. The global realm answers a
    // `withdraw` in the frame in which it gets it: that is the second
    // frame too.
    do_act(handle, cyd, "call", json!({ "to": ada, "ticks": 2 })).await;
    do_act(handle, bob, "call", json!({ "to": ada, "ticks": 2 })).await;
    handle.step(1).await.expect("step");
    do_act(handle, ada, "withdraw", json!({ "amount": 4 })).await;
    handle.step(1).await.expect("step");
    assert_eq!(notices(&look(handle, ada).await), [], "all three are on the way");
    handle.step(1).await.expect("step");
    let seen = look(handle, ada).await;
    assert_eq!(seen["self"]["cash"], json!(94));
    notices(&seen)
}

#[tokio::test]
async fn the_messages_of_one_frame_arrive_in_the_order_of_one_process() {
    let plugins_of_world = plugins();
    let refs: Vec<&dyn Plugin> = plugins_of_world.iter().map(AsRef::as_ref).collect();
    let world = World::new(&refs, world_config(5, 3, 16));
    let (handle, join) = start(world, RuntimeConfig::manual());
    let single = three_senders(&handle).await;
    join.shutdown().expect("a clean stop");

    let cluster = setup().start().expect("the cluster starts");
    let multi = three_senders(&cluster.handle).await;
    cluster.stop();

    // The router gives a zone the messages of the global realm first, then
    // those of the zones by zone number. The time at which a process
    // answers and the order of the calls do not count.
    let (bob, cyd) = (2, 3);
    let expected = [
        ("roads.receipt".to_string(), None),
        ("roads.call".to_string(), Some(bob)),
        ("roads.call".to_string(), Some(cyd)),
    ];
    assert_eq!(single, expected);
    assert_eq!(multi, expected);
}

/// Bob and Cyd walk into zone 4 in the same frame: Cyd from zone 3, Bob
/// from zone 1.
async fn two_arrivals(handle: &SimHandle) {
    handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    let cyd = handle.spawn(spec("Cyd")).await.expect("spawn");
    // Bob stands on the last column of zone 1 above zone 4, Cyd on the
    // last row of zone 3 left of zone 4. One step takes each into zone 4.
    do_act(handle, cyd, "travel", json!({ "zone": 3 })).await;
    do_act(handle, bob, "travel", json!({ "zone": 1 })).await;
    do_act(handle, cyd, "walk", json!({ "x": 15, "y": 20 })).await;
    do_act(handle, bob, "walk", json!({ "x": 20, "y": 15 })).await;
    handle.step(40).await.expect("step");
    assert_eq!(look(handle, cyd).await["self"]["pos"], json!({ "x": 15, "y": 20 }));
    assert_eq!(look(handle, bob).await["self"]["pos"], json!({ "x": 20, "y": 15 }));
    do_act(handle, cyd, "walk", json!({ "x": 16, "y": 20 })).await;
    do_act(handle, bob, "walk", json!({ "x": 20, "y": 16 })).await;
    handle.step(2).await.expect("step");
    assert_eq!(look(handle, cyd).await["self"]["zone"], json!(4));
    assert_eq!(look(handle, bob).await["self"]["zone"], json!(4));
}

#[tokio::test]
async fn residents_that_arrive_in_one_frame_take_their_rows_in_the_order_of_one_process() {
    let plugins_of_world = plugins();
    let refs: Vec<&dyn Plugin> = plugins_of_world.iter().map(AsRef::as_ref).collect();
    let world = World::new(&refs, world_config(5, 3, 16));
    let (handle, join) = start(world, RuntimeConfig::manual());
    two_arrivals(&handle).await;
    let single = join.shutdown().expect("a clean stop");
    let rows =
        |world: &World| world.realm(RealmId::Zone(ZoneId(4))).expect("zone 4").ids().to_vec();

    let cluster = setup().start().expect("the cluster starts");
    two_arrivals(&cluster.handle).await;
    let (_, zones) = cluster.stop();

    // Zone 1 comes before zone 3: Bob gets the first row, also when Cyd
    // acted first and the worker of zone 3 answered first.
    let (bob, cyd) = (ResidentId(2), ResidentId(3));
    assert_eq!(rows(&single), [bob, cyd]);
    assert_eq!(rows(&zones[4]), [bob, cyd]);
}
