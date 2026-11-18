//! How a cluster starts and how it stops when a process is lost.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use unsup_kernel::{Builder, Plugin, WorldConfig};
use unsup_protocol::{ObserveOpts, ResidentId, SimError};
use unsup_runtime::Plugins;
use unsup_runtime::cluster::{
    ClusterConfig, ClusterError, Coordinator, Worker, WorkerConfig, run_worker,
};

use common::cluster::Setup;
use common::roads::{Roads, plugins, world_config};
use common::{Town, act, spec};

fn world() -> WorldConfig {
    world_config(5, 3, 16)
}

/// A plugin that adds one verb to the catalogue.
struct Extra;

impl Plugin for Extra {
    fn name(&self) -> &'static str {
        "extra"
    }

    fn build(&self, b: &mut Builder) {
        b.instant_verb::<unsup_kernel::NoArgs>(
            unsup_kernel::VerbSpec {
                verb: "idle",
                module: "extra",
                summary: "Do nothing.",
                free: true,
                allowed_in: &[],
            },
            |_, _, _| Ok(json!(null)),
        );
    }
}

/// Returns the reason with which the coordinator refuses a worker.
fn refused(plugins: Plugins, config: WorkerConfig) -> String {
    match Worker::connect(plugins, config) {
        Err(ClusterError::Refused { reason }) => reason,
        other => panic!("the worker was not refused: {other:?}"),
    }
}

#[tokio::test]
async fn a_worker_with_another_world_or_other_plugins_is_refused() {
    let setup = Setup::new(plugins(), world(), 2);
    let coordinator = Coordinator::bind("127.0.0.1:0").expect("bind");
    let address = coordinator.local_addr().expect("address");
    let config = ClusterConfig {
        join_timeout: Duration::from_secs(60),
        ..ClusterConfig::manual(world(), 2)
    };
    let starting = std::thread::spawn({
        let plugins = plugins();
        move || coordinator.start(plugins, config)
    });
    let worker = |world: WorldConfig, slot: u32| WorkerConfig::new(world, slot, address);

    // Another seed: another world.
    let reason = refused(plugins(), worker(world_config(6, 3, 16), 0));
    assert!(reason.starts_with("the world configuration of the worker is not"), "{reason}");
    // Another zone size.
    let reason = refused(plugins(), worker(world_config(5, 3, 32), 0));
    assert!(reason.starts_with("the world configuration of the worker is not"), "{reason}");
    // A data override changes a parameter.
    let tuned = WorldConfig {
        data: vec!["[params.\"roads.shout_radius\"]\nvalue = 9\n".into()],
        ..world()
    };
    let reason = refused(plugins(), worker(tuned, 0));
    assert!(reason.starts_with("the world configuration of the worker is not"), "{reason}");
    // One plugin is missing, one more is there, the order is another.
    let fewer: Plugins = Arc::new(vec![Box::new(Town::default())]);
    let more: Plugins = Arc::new(vec![Box::new(Town::default()), Box::new(Roads), Box::new(Extra)]);
    let swapped: Plugins = Arc::new(vec![Box::new(Roads), Box::new(Town::default())]);
    for other in [fewer, more, swapped] {
        let reason = refused(other, worker(world(), 0));
        assert!(reason.starts_with("the set of plugins of the worker is not"), "{reason}");
    }
    // The same plugins with another balance number.
    let richer: Plugins = Arc::new(vec![Box::new(Town { wage: 501 }), Box::new(Roads)]);
    // The wage is not in the catalogue: the handshake cannot see it. The
    // README names this limit. A slot that does not exist is seen.
    let reason = refused(richer, worker(world(), 2));
    assert_eq!(reason, "the cluster has 2 worker slots, there is no slot 2");

    // The slots are still free: the right workers join, and a second
    // worker for a slot that is taken is refused.
    let (first, _) = setup.worker(0, address);
    let reason = refused(plugins(), worker(world(), 0));
    assert_eq!(reason, "another worker has slot 0");
    let (second, _) = setup.worker(1, address);
    let (handle, join) = starting.join().expect("the thread ran").expect("the cluster starts");

    // The cluster runs, and nobody joins a cluster that runs.
    assert_eq!(handle.spawn(spec("Ada")).await, Ok(ResidentId(1)));
    assert_eq!(handle.step(1).await, Ok(1));
    let late = Worker::connect(
        plugins(),
        WorkerConfig { connect_timeout: Duration::from_millis(200), ..worker(world(), 1) },
    );
    assert!(matches!(late, Err(ClusterError::Io(_))), "{late:?}");
    drop(handle);
    join.shutdown().expect("a stop in order");
    for thread in [first, second] {
        let exit = thread.join().expect("the worker thread ran").expect("a stop in order");
        assert!(!exit.worlds.is_empty());
    }
}

#[test]
fn a_cluster_without_all_its_workers_does_not_start() {
    let coordinator = Coordinator::bind("127.0.0.1:0").expect("bind");
    let address = coordinator.local_addr().expect("address");
    let config = ClusterConfig {
        join_timeout: Duration::from_millis(300),
        ..ClusterConfig::manual(world(), 3)
    };
    // Only the worker of slot 1 comes.
    let worker =
        std::thread::spawn(move || run_worker(plugins(), WorkerConfig::new(world(), 1, address)));
    let error = coordinator.start(plugins(), config).expect_err("two workers are missing");
    assert_eq!(error, ClusterError::JoinTimeout { missing: vec![0, 2] });
    // The worker that came learns why it ends.
    let ended = worker.join().expect("the worker thread ran").expect_err("no cluster");
    assert_eq!(
        ended,
        ClusterError::Stopped {
            reason: "the cluster did not start: no worker joined for the slots [0, 2]".into()
        }
    );
}

#[test]
fn a_bad_configuration_is_an_error_before_any_connection() {
    let coordinator = || Coordinator::bind("127.0.0.1:0").expect("bind");
    // A zone without a worker.
    let mut config = ClusterConfig::manual(world(), 3);
    config.workers[2].pop();
    let error = coordinator().start(plugins(), config).expect_err("zone 8 has no worker");
    assert_eq!(error, ClusterError::Config("zone 8 has no worker".into()));
    // A world that the kernel refuses.
    let bad = WorldConfig { zone_size: 4, ..world() };
    let error = coordinator().start(plugins(), ClusterConfig::manual(bad.clone(), 1));
    let message = "invalid world configuration: zone_size is 4, it must be from 8 to 1024";
    assert_eq!(error.expect_err("a bad world"), ClusterError::Config(message.into()));
    let address = coordinator().local_addr().expect("address");
    let error = Worker::connect(plugins(), WorkerConfig::new(bad, 0, address));
    assert_eq!(error.expect_err("a bad world"), ClusterError::Config(message.into()));
}

#[tokio::test]
async fn a_worker_that_dies_stops_the_whole_cluster() {
    let cluster = Setup::new(plugins(), world(), 3).start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let mut frames = handle.frames();
    // One resident in each process.
    let mut residents = Vec::new();
    for zone in [0, 3, 6] {
        let id = handle.spawn(spec("R")).await.expect("spawn");
        handle.act(id, act("travel", json!({ "zone": zone }))).await.expect("travel");
        residents.push(id);
    }
    handle.step(2).await.expect("step");
    assert_eq!(frames.recv().await.expect("frame 1").frame, 1);
    assert_eq!(frames.recv().await.expect("frame 2").frame, 2);

    // The process of worker 1 dies.
    cluster.aborts[1].abort();

    // The cluster does not go on with a hole. No frame runs any more, and
    // every call fails, also for the residents of the workers that live.
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
    for &resident in &residents {
        assert_eq!(handle.act(resident, act("chop", json!({}))).await, Err(SimError::Shutdown));
        assert_eq!(
            handle.observe(resident, ObserveOpts::default()).await.map(|_| ()),
            Err(SimError::Shutdown)
        );
    }
    assert_eq!(handle.spawn(spec("Late")).await, Err(SimError::Shutdown));
    assert_eq!(handle.cluster_stats().await.map(|_| ()), Err(SimError::Shutdown));
    assert_eq!(handle.audit().await, Err(ClusterError::Sim(SimError::Shutdown)));
    // The stream of frames ends after frame 2.
    assert!(matches!(frames.recv().await, Err(RecvError::Closed)));

    // The coordinator names the worker that it lost.
    let common::cluster::Running { join, workers, .. } = cluster;
    assert!(matches!(
        join.shutdown(),
        Err(ClusterError::WorkerLost { slot: 1, reason }) if reason == "the connection is closed"
    ));
    // Every worker ends, and each knows why.
    let mut ends = Vec::new();
    for worker in workers {
        ends.push(worker.join().expect("the worker thread ran").expect_err("no stop in order"));
    }
    let told = ClusterError::Stopped {
        reason: "the cluster stopped: worker 1 is lost: the connection is closed".into(),
    };
    assert_eq!(ends[0], told);
    assert!(matches!(ends[1], ClusterError::CoordinatorLost { .. }), "{:?}", ends[1]);
    assert_eq!(ends[2], told);
}

#[tokio::test]
async fn a_worker_that_dies_while_the_cluster_waits_is_found_without_a_call() {
    let cluster = Setup::new(plugins(), world(), 2).start().expect("the cluster starts");
    let mut frames = cluster.handle.frames();
    cluster.handle.step(1).await.expect("step");
    assert_eq!(frames.recv().await.expect("frame 1").frame, 1);
    cluster.aborts[0].abort();
    // Nobody calls the cluster. The coordinator still learns of the dead
    // connection and stops: the stream of frames ends.
    assert!(matches!(frames.recv().await, Err(RecvError::Closed)));
    let common::cluster::Running { join, workers, handle, .. } = cluster;
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
    assert!(matches!(join.shutdown(), Err(ClusterError::WorkerLost { slot: 0, .. })));
    for worker in workers {
        worker.join().expect("the worker thread ran").expect_err("no stop in order");
    }
}

#[tokio::test]
async fn a_panic_in_a_zone_stops_the_whole_cluster_and_names_the_zone() {
    let cluster = Setup::new(plugins(), world(), 3).start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.act(ada, act("travel", json!({ "zone": 4 }))).await.expect("travel");
    handle.step(1).await.expect("step");
    // The verb `crash` of the town module panics, in zone 4 on worker 1.
    assert_eq!(handle.act(ada, act("crash", json!({}))).await, Err(SimError::Shutdown));
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));

    let common::cluster::Running { join, workers, .. } = cluster;
    let failed = ClusterError::Failed {
        realm: unsup_kernel::RealmId::Zone(unsup_protocol::ZoneId(4)),
        message: "the simulation panicked: the town module has a bug".into(),
    };
    assert_eq!(join.shutdown().expect_err("the cluster failed"), failed);
    let mut ends = Vec::new();
    for worker in workers {
        ends.push(worker.join().expect("the worker thread ran").expect_err("no stop in order"));
    }
    assert_eq!(ends[1], failed);
    assert!(
        matches!(&ends[0], ClusterError::Stopped { reason } if reason.contains("zone 4 failed"))
    );
    assert!(
        matches!(&ends[2], ClusterError::Stopped { reason } if reason.contains("zone 4 failed"))
    );
}
