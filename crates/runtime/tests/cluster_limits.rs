//! What a cluster cannot do with the kernel as it is. Each test shows one
//! limit of "Limits" in `README.md`: what one process does, and what the
//! cluster does instead.
//!
//! These tests hold the limits in place. When the kernel gets the change
//! that a limit waits for, its test fails. Change the test then to expect
//! what one process does, and take the limit out of the README.

mod common;

use std::sync::Arc;

use serde_json::json;
use unsup_kernel::{Builder, Money, Plugin, RealmId, UNCLAIMED, World};
use unsup_protocol::{CityId, ObserveOpts, OwnerId, ResidentId, SimError, SpawnSpec, ZoneId};
use unsup_runtime::{Plugins, RuntimeConfig, SimHandle, start};

use common::cluster::{
    Setup, cluster_digests, do_act, look, notices, residents_per_zone, world_digests,
};
use common::roads::{BANK, Roads, Signals, plugins, world_config};
use common::{Town, spec};

/// Nine zones of 16 by 16 tiles. Worker 0 hosts the zones 0 to 2, worker 1
/// the zones 3 to 5, worker 2 the zones 6 to 8.
fn setup() -> Setup {
    Setup::new(plugins(), world_config(5, 3, 16), 3)
}

/// Makes the same world for one process.
fn one_process() -> World {
    let plugins = plugins();
    let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    World::new(&refs, world_config(5, 3, 16))
}

#[tokio::test]
async fn the_kernel_drops_a_message_for_a_resident_that_the_world_of_the_sender_never_saw() {
    let cluster = setup().start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    do_act(&handle, ada, "travel", json!({ "zone": 3 })).await;
    // Bob never was in zone 3. The world of zone 3 has never seen him, so
    // the kernel there takes him for a resident that is gone and drops
    // the notice. One process delivers it.
    do_act(&handle, ada, "tell", json!({ "to": bob, "kind": "town.greeting" })).await;
    handle.step(2).await.expect("step");
    assert_eq!(notices(&look(&handle, bob).await), []);
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").dropped_messages, 1);

    // Bob never left the zone in which he was made, so the world of the
    // global realm has never seen him either: the answer to his verb of
    // the global realm is dropped, and his action waits for its timeout.
    do_act(&handle, bob, "deposit", json!({ "amount": 20 })).await;
    do_act(&handle, bob, "withdraw", json!({ "amount": 5 })).await;
    handle.step(3).await.expect("step");
    let seen = look(&handle, bob).await;
    assert_eq!(seen["self"]["activity"]["verb"], json!("withdraw"), "no answer came");
    assert_eq!(seen["self"]["cash"], json!(80));
    // The receipt and the answer: two more. The wire is not dropped: the
    // kernel puts a wire for a resident that is gone on the unclaimed
    // account of the global realm. So no money is lost, but Bob does not
    // get his.
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").dropped_messages, 3);
    let totals = cluster.handle.audit().await.expect("the union balances");
    assert_eq!((totals.money.held, totals.money.created), (200, 200));

    // After Bob crossed a border once, both worlds know him.
    do_act(&handle, bob, "cancel", json!({})).await;
    do_act(&handle, bob, "travel", json!({ "zone": 3 })).await;
    do_act(&handle, bob, "travel", json!({ "zone": 0 })).await;
    do_act(&handle, ada, "tell", json!({ "to": bob, "kind": "town.greeting" })).await;
    do_act(&handle, bob, "withdraw", json!({ "amount": 5 })).await;
    handle.step(2).await.expect("step");
    let seen = look(&handle, bob).await;
    assert_eq!(notices(&seen)[0], ("town.greeting".to_string(), Some(ada.0)));
    assert_eq!(seen["self"]["cash"], json!(85));
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").dropped_messages, 3);
    let (global, _) = cluster.stop();
    let ledger = global.realm(RealmId::Global).expect("the global realm").ledger();
    assert_eq!((ledger.money(BANK), ledger.money(UNCLAIMED)), (Money(10), Money(5)));
}

/// Two residents send a number to zone 1 in the same tick: Ada from zone
/// 0, Bob from zone 1 itself.
async fn two_signals(handle: &SimHandle) {
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    do_act(handle, bob, "travel", json!({ "zone": 1 })).await;
    do_act(handle, ada, "signal", json!({ "zone": 1, "n": 10, "ticks": 2 })).await;
    do_act(handle, bob, "signal", json!({ "zone": 1, "n": 11, "ticks": 2 })).await;
    handle.step(4).await.expect("step");
}

fn heard(zone_1: &World) -> Vec<u64> {
    zone_1.realm(RealmId::Zone(ZoneId(1))).expect("zone 1").res::<Signals>().heard.clone()
}

#[tokio::test]
async fn a_message_of_a_zone_to_itself_comes_before_the_messages_of_lower_zones() {
    let (handle, join) = start(one_process(), RuntimeConfig::manual());
    two_signals(&handle).await;
    let single = join.shutdown().expect("a clean stop");
    // One process routes by zone number: zone 0 first, then zone 1.
    assert_eq!(heard(&single), [10, 11]);

    let cluster = setup().start().expect("the cluster starts");
    two_signals(&cluster.handle).await;
    let (_, zones) = cluster.stop();
    // The world of zone 1 gives its own message to its zone by itself,
    // before the coordinator brings the message of zone 0.
    assert_eq!(heard(&zones[1]), [11, 10]);
}

#[tokio::test]
async fn a_realm_differs_from_one_process_while_it_holds_a_message_of_another_world() {
    let (handle, join) = start(one_process(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    do_act(&handle, ada, "signal", json!({ "zone": 4, "n": 7, "ticks": 1 })).await;
    handle.step(1).await.expect("step");
    let single = join.shutdown().expect("a clean stop");
    let with_message = world_digests(&single);
    let (handle, join) = start(single, RuntimeConfig::manual());
    handle.step(1).await.expect("step");
    let after_tick = world_digests(&join.shutdown().expect("a clean stop"));

    let cluster = setup().start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    do_act(&handle, ada, "signal", json!({ "zone": 4, "n": 7, "ticks": 1 })).await;
    handle.step(1).await.expect("step");
    // Messages of zone 0 wait in three inboxes: the zones 1, 3 and 4 got
    // the news that Ada is near their border, and zone 4 got the number.
    // The kernel marks a message that came from another world, and the
    // mark is in the state of the realm. So these three realms differ from
    // one process. Every other realm is as in one process.
    let waiting = cluster_digests(&handle).await;
    let differ: Vec<RealmId> = waiting
        .iter()
        .filter(|(realm, digest)| with_message[*realm] != **digest)
        .map(|(realm, _)| *realm)
        .collect();
    let zone = |zone| RealmId::Zone(ZoneId(zone));
    assert_eq!(differ, [zone(1), zone(3), zone(4)]);
    // The zones take the messages in their next tick. Then all realms are
    // as in one process again.
    handle.step(1).await.expect("step");
    assert_eq!(cluster_digests(&handle).await, after_tick);
    let (_, zones) = cluster.stop();
    assert_eq!(
        zones[4].realm(RealmId::Zone(ZoneId(4))).expect("zone 4").res::<Signals>().heard,
        [7]
    );
}

#[tokio::test]
async fn a_viewer_sees_a_resident_enter_a_zone_one_frame_after_it_left_the_other() {
    /// Returns, for each of the first two frames, the zones that a
    /// resident left and the zones that one entered.
    async fn crossing(handle: &SimHandle) -> Vec<(Vec<u32>, Vec<u32>)> {
        let mut frames = handle.frames();
        let ada = handle.spawn(spec("Ada")).await.expect("spawn");
        // The spawn is news for viewers too. A first frame takes it.
        handle.step(1).await.expect("step");
        frames.recv().await.expect("frame 1");
        // One step to the south-east from (8, 8) is still zone 0. Eight
        // steps reach (16, 16), the first tile of zone 4.
        do_act(handle, ada, "walk", json!({ "x": 16, "y": 16 })).await;
        handle.step(9).await.expect("step");
        let mut seen = Vec::new();
        for _ in 0..9 {
            let frame = frames.recv().await.expect("a frame");
            let zones = |pick: fn(&unsup_protocol::ZoneDelta) -> bool| -> Vec<u32> {
                frame.zones.iter().filter(|delta| pick(delta)).map(|delta| delta.zone.0).collect()
            };
            let moved =
                (zones(|delta| !delta.left.is_empty()), zones(|delta| !delta.entered.is_empty()));
            if moved != (vec![], vec![]) {
                seen.push(moved);
            }
        }
        seen
    }

    let (handle, join) = start(one_process(), RuntimeConfig::manual());
    let single = crossing(&handle).await;
    join.shutdown().expect("a clean stop");
    // One process: she leaves zone 0 and enters zone 4 in the same frame.
    assert_eq!(single, [(vec![0], vec![4])]);

    let cluster = setup().start().expect("the cluster starts");
    let multi = crossing(&cluster.handle).await;
    cluster.stop();
    // The cluster: the world of zone 4 gives its news for viewers only
    // when it runs a frame, and she arrives after its frame.
    assert_eq!(multi, [(vec![0], vec![]), (vec![], vec![4])]);
}

#[tokio::test]
async fn every_resident_is_made_in_the_spawn_zone() {
    // Two cities. The spawn zone is zone 0, in the first city.
    let mut world = world_config(5, 2, 16);
    world.cities.push(unsup_kernel::CityConfig::new("Farside", unsup_kernel::Pos::new(64, 0), 1));
    let cluster = Setup::new(plugins(), world, 2).start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let spec = |city| SpawnSpec { owner: OwnerId(1), name: "Ada".into(), model: None, city };
    // The ids come from one world, in the order of the calls.
    assert_eq!(handle.spawn(spec(Some(CityId(0)))).await, Ok(ResidentId(1)));
    // Without a city, the kernel of the spawn zone takes its own city.
    assert_eq!(handle.spawn(spec(None)).await, Ok(ResidentId(2)));
    // The world of the spawn zone hosts no zone of the second city.
    assert_eq!(handle.spawn(spec(Some(CityId(1)))).await, Err(SimError::NotHosted));
    assert_eq!(handle.spawn(spec(Some(CityId(9)))).await, Err(SimError::Invalid));
    assert_eq!(handle.spawn(spec(None)).await, Ok(ResidentId(3)));
    assert_eq!(residents_per_zone(&handle).await, [3, 0, 0, 0, 0]);
    // A resident can go to the other city.
    do_act(&handle, ResidentId(3), "travel", json!({ "zone": 4 })).await;
    assert_eq!(residents_per_zone(&handle).await, [2, 0, 0, 0, 1]);
    cluster.stop();
}

/// A plugin whose only part is a section of the observation that the
/// global realm writes.
struct Vista;

impl Plugin for Vista {
    fn name(&self) -> &'static str {
        "vista"
    }

    fn build(&self, b: &mut Builder) {
        b.observe_global("vista", 0, |realm, resident, _| {
            Some(json!({ "in_directory": realm.directory().is_alive(resident) }))
        });
    }
}

#[tokio::test]
async fn an_observation_has_no_section_of_the_global_realm() {
    let with_vista = || -> Plugins {
        Arc::new(vec![Box::new(Town::default()), Box::new(Roads), Box::new(Vista)])
    };
    /// Returns the names of the sections that a new resident sees after
    /// one frame, and the section of the global realm.
    async fn sections(handle: &SimHandle) -> (Vec<String>, Option<serde_json::Value>) {
        let ada = handle.spawn(spec("Ada")).await.expect("spawn");
        handle.step(1).await.expect("step");
        let seen = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
        (seen.sections.keys().cloned().collect(), seen.sections.get("vista").cloned())
    }

    let plugins = with_vista();
    let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    let world = World::new(&refs, world_config(5, 3, 16));
    let (handle, join) = start(world, RuntimeConfig::manual());
    let single = sections(&handle).await;
    join.shutdown().expect("a clean stop");
    assert_eq!(single.0, ["nearby", "reports", "self", "vista"]);
    assert_eq!(single.1, Some(json!({ "in_directory": true })));

    // In the cluster the zone of the resident and the global realm are in
    // two worlds. The kernel has no call that gives the sections of the
    // global realm for a resident of another world.
    let cluster =
        Setup::new(with_vista(), world_config(5, 3, 16), 3).start().expect("the cluster starts");
    let multi = sections(&cluster.handle).await;
    cluster.stop();
    assert_eq!(multi.0, ["nearby", "reports", "self"]);
    assert_eq!(multi.1, None);
}
