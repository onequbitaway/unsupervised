//! The input log and the snapshots of each process, and the recovery of
//! the whole cluster to one consistent frame.

mod common;

use serde_json::json;
use unsup_kernel::RealmId;
use unsup_protocol::{ObserveOpts, ResidentId, SimError, ZoneId};
use unsup_runtime::cluster::{ClusterError, ClusterHandle};
use unsup_runtime::{MemStorage, Storage};

use common::cluster::{Disks, Running, Setup};
use common::roads::{plugins, world_config};
use common::script::Script;
use common::{act, hex, spec};

/// Seed of the world and of the script.
const SEED: u64 = 23;

/// A cluster of three workers that records. Every realm takes a snapshot
/// every four frames, so a recovery restores a snapshot and replays the
/// log after it.
fn setup(disks: &Disks) -> Setup {
    Setup {
        disks: Some(disks.clone()),
        snapshot_every: 4,
        ..Setup::new(plugins(), world_config(SEED, 3, 16), 3)
    }
}

/// Returns the state hash of the world of every realm, as text.
async fn hashes(handle: &ClusterHandle) -> Vec<(RealmId, String)> {
    let hashes = handle.hashes().await.expect("the hashes of the cluster");
    assert_eq!(hashes.len(), 10, "the global realm and nine zones");
    hashes.into_iter().map(|(realm, hash)| (realm, hex(&hash))).collect()
}

/// Ends a cluster the hard way: one worker dies. Nothing is written in
/// order.
async fn kill(cluster: Running) {
    cluster.aborts[0].abort();
    let Running { handle, join, workers, .. } = cluster;
    // The coordinator finds the dead worker and stops the others.
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
    assert!(matches!(join.shutdown(), Err(ClusterError::WorkerLost { slot: 0, .. })));
    for worker in workers {
        worker.join().expect("the worker thread ran").expect_err("no stop in order");
    }
}

#[tokio::test]
async fn the_cluster_recovers_to_the_newest_frame_that_every_process_has() {
    let disks = Disks::new(3);
    let cluster = setup(&disks).start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let mut script = Script::new(SEED, &setup(&disks).world);
    script.spawn(&handle, 8).await;
    for round in 0..10 {
        script.round(&handle, round).await;
    }
    // Frame 10. Worker 1 dies here: this is what its disk holds.
    assert_eq!(handle.stats().await.expect("stats").frame, 10);
    let disk_of_worker_1 = disks.workers[1].fork();
    let at_frame_10 = hashes(&handle).await;
    let script_at_frame_10 = script.clone();
    handle.audit().await.expect("the union balances at frame 10");

    // The other processes go on for six frames, and then take inputs that
    // no frame follows. (In a real cluster they stop with worker 1. Their
    // disks then hold a part of a frame; here they hold whole frames more.)
    let mut later = vec![at_frame_10.clone()];
    for round in 10..16 {
        script.round(&handle, round).await;
        later.push(hashes(&handle).await);
    }
    let extra = handle.spawn(spec("Too late")).await.expect("spawn");
    handle.act(extra, act("travel", json!({ "zone": 5 }))).await.expect("travel");
    assert_ne!(hashes(&handle).await, later[6]);
    assert_ne!(later[6], at_frame_10);
    let crashed = Disks {
        coordinator: disks.coordinator.fork(),
        workers: vec![disks.workers[0].fork(), disk_of_worker_1, disks.workers[2].fork()],
    };
    kill(cluster).await;

    // The cluster starts again on what the disks hold. The newest seal
    // that all realms have is the one of frame 10, so every realm goes
    // back to it: the coordinator and two workers drop six frames.
    let cluster = setup(&crashed).start().expect("the cluster recovers");
    let handle = cluster.handle.clone();
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.frame, stats.residents), (10, script_at_frame_10.living().len() as u64));
    // Seal 0 is the start of the world, seal `n` the end of frame `n`.
    assert_eq!(handle.cluster_stats().await.expect("stats").epoch, 10);
    assert_eq!(hashes(&handle).await, at_frame_10, "every realm is as it was at frame 10");
    handle.audit().await.expect("the union balances after the recovery");
    // The resident that came after frame 10 never existed.
    assert_eq!(
        handle.observe(extra, ObserveOpts::default()).await.map(|_| ()),
        Err(SimError::UnknownResident)
    );

    // The recovered cluster goes on like the first one did: the same
    // script gives the same state after each of the next six frames.
    let mut script = script_at_frame_10;
    for (round, expected) in (10..16).zip(&later[1..]) {
        script.round(&handle, round).await;
        assert_eq!(&hashes(&handle).await, expected, "after round {round}");
    }
    cluster.stop();
}

#[tokio::test]
async fn a_stop_in_order_and_a_new_start_lose_nothing() {
    let disks = Disks::new(3);
    let cluster = setup(&disks).start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let mut script = Script::new(SEED, &setup(&disks).world);
    script.spawn(&handle, 6).await;
    for round in 0..6 {
        script.round(&handle, round).await;
    }
    // Inputs after the last frame: a new resident that goes to zone 7 in
    // the third process at once. The directory of the global realm learns
    // of both only in its next tick.
    let late = handle.spawn(spec("Late")).await.expect("spawn");
    handle.act(late, act("travel", json!({ "zone": 7 }))).await.expect("travel");
    let before = hashes(&handle).await;
    let epoch = handle.cluster_stats().await.expect("stats").epoch;
    assert_eq!(epoch, 6);
    drop(handle);
    cluster.stop();

    // The same disks, a new start. The stop wrote one more seal, after
    // the inputs.
    let cluster = setup(&disks).start().expect("the cluster starts again");
    let handle = cluster.handle.clone();
    assert_eq!(handle.stats().await.expect("stats").frame, 6);
    assert_eq!(handle.cluster_stats().await.expect("stats").epoch, 7);
    assert_eq!(hashes(&handle).await, before);
    // The coordinator finds the resident in zone 7, also before the
    // global realm ticked.
    let seen = handle.observe(late, ObserveOpts::default()).await.expect("observe");
    assert_eq!(seen.sections["self"]["zone"], json!(7));
    assert_eq!(handle.spawn(spec("Next")).await, Ok(ResidentId(late.0 + 1)));
    handle.step(1).await.expect("step");
    handle.audit().await.expect("the union balances");
    cluster.stop();

    // A third start goes on at the last seal, the one of frame 7. The
    // second stop wrote no seal of its own: no input came after the frame.
    let cluster = setup(&disks).start().expect("the cluster starts a third time");
    assert_eq!(cluster.handle.stats().await.expect("stats").frame, 7);
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").epoch, 8);
    cluster.stop();
}

#[tokio::test]
async fn a_loss_of_power_leaves_a_cluster_that_recovers() {
    let disks = Disks::new(3);
    let cluster = setup(&disks).start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let mut script = Script::new(SEED, &setup(&disks).world);
    script.spawn(&handle, 6).await;
    for round in 0..7 {
        script.round(&handle, round).await;
    }
    let at_frame_7 = hashes(&handle).await;
    // An input that no frame follows. Every process forces each input to
    // the disk before it applies it, so the input is on the disk. No seal
    // covers it.
    handle.spawn(spec("Unsealed")).await.expect("spawn");
    // The power fails: only what was forced to the disk is left.
    let after = Disks {
        coordinator: disks.coordinator.fork_durable(),
        workers: disks.workers.iter().map(MemStorage::fork_durable).collect(),
    };
    kill(cluster).await;

    let cluster = setup(&after).start().expect("the cluster recovers");
    assert_eq!(cluster.handle.stats().await.expect("stats").frame, 7);
    assert_eq!(hashes(&cluster.handle).await, at_frame_7);
    cluster.stop();
}

#[tokio::test]
async fn stored_state_that_does_not_fit_together_stops_the_start() {
    let disks = Disks::new(3);
    let cluster = setup(&disks).start().expect("the cluster starts");
    cluster.handle.spawn(spec("Ada")).await.expect("spawn");
    cluster.handle.step(3).await.expect("step");
    cluster.stop();

    // Worker 1 comes back with an empty disk. A new world for its zones
    // next to the old world of the others would be a hole.
    let mut lost = disks.fork();
    lost.workers[1] = MemStorage::new();
    let error = setup(&lost).start().map(|_| ()).expect_err("a disk is empty");
    assert_eq!(
        error,
        ClusterError::Recovery {
            realm: None,
            message: "some realms have stored state and these have none: zone 3, zone 4, zone 5"
                .into()
        }
    );

    // The file with the seals of zone 4 is gone. The zone has a world
    // that ran frames, so it does not start as a new world.
    let damaged = disks.fork();
    damaged.workers[1].delete("zone-00004/seals").expect("delete");
    let error = setup(&damaged).start().map(|_| ()).expect_err("no seal for zone 4");
    assert!(
        matches!(
            &error,
            ClusterError::Failed { realm, message } | ClusterError::Recovery { realm: Some(realm), message }
                if *realm == RealmId::Zone(ZoneId(4)) && message.contains("no seal of it can be used")
        ),
        "{error:?}"
    );

    // The disks as they are start fine: nothing above changed them.
    let cluster = setup(&disks).start().expect("the cluster starts again");
    assert_eq!(cluster.handle.stats().await.expect("stats").frame, 3);
    cluster.stop();
}

#[tokio::test]
async fn a_start_that_was_cut_off_before_its_first_seal_starts_again_as_new() {
    // A new cluster on empty disks, for the comparison.
    let new = setup(&Disks::new(3)).start().expect("the cluster starts");
    let expected = hashes(&new.handle).await;
    new.stop();

    // A cluster that stopped right after its start: every realm has its
    // first snapshot, the first exchange in its log and seal 0.
    let disks = Disks::new(3);
    setup(&disks).start().expect("the cluster starts").stop();
    // The process of worker 2 died in that first exchange: its zones wrote
    // no seal.
    for zone in 6..9 {
        disks.workers[2].delete(&format!("zone-{zone:05}/seals")).expect("delete");
    }
    // No input came before seal 0 was in every realm. So the world starts
    // as new: every realm goes back to its first snapshot, and the first
    // exchange runs again.
    let cluster = setup(&disks).start().expect("the cluster starts as new");
    assert_eq!(hashes(&cluster.handle).await, expected);
    assert_eq!(cluster.handle.cluster_stats().await.expect("stats").epoch, 0);
    assert_eq!(cluster.handle.spawn(spec("Ada")).await, Ok(ResidentId(1)));
    cluster.handle.step(2).await.expect("step");
    cluster.handle.audit().await.expect("the union balances");
    cluster.stop();
}

#[tokio::test]
async fn a_cluster_on_the_file_system_stops_and_starts_again() {
    use std::sync::Arc;
    use unsup_runtime::cluster::{ClusterConfig, ClusterDurable, Coordinator, WorkerConfig};
    use unsup_runtime::{DurableConfig, FsStorage, SyncPolicy};

    let root = std::env::temp_dir().join(format!("unsup-cluster-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let world = world_config(SEED, 2, 16);
    // One directory per process. `FsStorage` locks it, so each start opens
    // it again.
    let open = |name: &str| ClusterDurable {
        storage: Arc::new(FsStorage::open(root.join(name)).expect("the directory opens")),
        config: DurableConfig { sync: SyncPolicy::EveryBatch, ..DurableConfig::default() },
    };
    let start = || {
        let coordinator = Coordinator::bind("127.0.0.1:0").expect("bind");
        let address = coordinator.local_addr().expect("address");
        let workers: Vec<_> = (0..2u32)
            .map(|slot| {
                let config = WorkerConfig {
                    durable: Some(open(&format!("worker-{slot}"))),
                    ..WorkerConfig::new(world.clone(), slot, address)
                };
                std::thread::spawn(move || unsup_runtime::cluster::run_worker(plugins(), config))
            })
            .collect();
        let config = ClusterConfig {
            durable: Some(open("coordinator")),
            ..ClusterConfig::manual(world.clone(), 2)
        };
        let (handle, join) = coordinator.start(plugins(), config).expect("the cluster starts");
        (handle, join, workers)
    };

    let (handle, join, workers) = start();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.act(ada, act("walk", json!({ "x": 20, "y": 20 }))).await.expect("walk");
    handle.step(15).await.expect("step");
    let before = hashes_of(&handle).await;
    drop(handle);
    join.shutdown().expect("a stop in order");
    for worker in workers {
        worker.join().expect("the worker thread ran").expect("a stop in order");
    }
    // Each realm has its own directory with a log, snapshots and seals.
    for path in
        ["coordinator/global/seals", "worker-0/zone-00000/log", "worker-1/zone-00003/snapshots"]
    {
        assert!(root.join(path).exists(), "{path}");
    }

    let (handle, join, workers) = start();
    assert_eq!(handle.stats().await.expect("stats").frame, 15);
    assert_eq!(hashes_of(&handle).await, before);
    let seen = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
    assert_eq!(seen.sections["self"]["pos"], json!({ "x": 20, "y": 20 }));
    assert_eq!(seen.sections["self"]["zone"], json!(3));
    drop(handle);
    join.shutdown().expect("a stop in order");
    for worker in workers {
        worker.join().expect("the worker thread ran").expect("a stop in order");
    }
    std::fs::remove_dir_all(&root).expect("the directory can be removed");
}

/// Returns the state hash of the world of every realm of a cluster of any
/// size, as text.
async fn hashes_of(handle: &ClusterHandle) -> Vec<(RealmId, String)> {
    let hashes = handle.hashes().await.expect("the hashes of the cluster");
    hashes.into_iter().map(|(realm, hash)| (realm, hex(&hash))).collect()
}
