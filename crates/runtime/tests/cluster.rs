//! The cluster: zones in several processes give the world of one process.
//!
//! Every cluster of these tests runs in this process: the coordinator and
//! each worker are threads, and they talk over TCP on this machine, as
//! separate processes do.

mod common;

use std::collections::BTreeMap;

use unsup_kernel::{Plugin, RealmId, World, WorldConfig};
use unsup_runtime::cluster::ClusterStats;
use unsup_runtime::{RuntimeConfig, SimHandle, start};

use common::cluster::{Setup, cluster_digests, world_digests};
use common::roads::{plugins, world_config};
use common::script::Script;

/// Seed of the world and of the script.
const SEED: u64 = 11;

/// Residents of the script.
const RESIDENTS: usize = 12;

/// The script has this many parts. The residents come to rest after each
/// part, and the states of all realms are compared.
const PARTS: u64 = 3;

/// Rounds in one part of the script.
const ROUNDS: u64 = 40;

/// Nine zones of 16 by 16 tiles: a short walk crosses a border.
fn world() -> WorldConfig {
    world_config(SEED, 3, 16)
}

/// What one run of the script gave.
struct Run {
    /// Every answer and every observation, frame by frame.
    record: Vec<String>,
    /// The hash of the state of every realm after each part.
    states: Vec<BTreeMap<RealmId, String>>,
    /// Messages that the kernel dropped.
    dropped: u64,
    /// Residents that were seen in more than one zone.
    travelled: usize,
    /// Residents that are alive at the end.
    living: usize,
}

/// Plays one part of the script and lets the residents come to rest.
async fn play_part(script: &mut Script, handle: &SimHandle, part: u64) {
    if part == 0 {
        script.spawn(handle, RESIDENTS).await;
    }
    for round in part * ROUNDS..(part + 1) * ROUNDS {
        script.round(handle, round).await;
    }
    script.rest(handle).await;
}

/// Runs the script in one process, with the runtime of one process.
async fn in_one_process(config: &WorldConfig) -> Run {
    let plugins = plugins();
    let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    let mut world = World::new(&refs, config.clone());
    let mut script = Script::new(SEED, config);
    let mut states = Vec::new();
    for part in 0..PARTS {
        let (handle, join) = start(world, RuntimeConfig::manual());
        play_part(&mut script, &handle, part).await;
        script.frame(&handle).await;
        // The runtime gives the world back, and the next part goes on
        // with it.
        world = join.shutdown().expect("a clean stop");
        world.audit().expect("money and goods balance in one process");
        states.push(world_digests(&world));
    }
    Run {
        travelled: script.travelled(),
        living: script.living().len(),
        record: script.record,
        states,
        dropped: world.dropped_messages(),
    }
}

/// Runs the script in a cluster of `workers` workers. Returns the run and
/// the numbers of the cluster at the end.
async fn in_a_cluster(config: &WorldConfig, workers: usize) -> (Run, ClusterStats) {
    let cluster =
        Setup::new(plugins(), config.clone(), workers).start().expect("the cluster starts");
    let handle = &cluster.handle;
    let mut script = Script::new(SEED, config);
    let mut states = Vec::new();
    for part in 0..PARTS {
        play_part(&mut script, handle, part).await;
        // One more frame in which nothing must be on the way: then no
        // realm holds a message, and the states can be compared.
        let before = handle.cluster_stats().await.expect("stats");
        script.frame(handle).await;
        let after = handle.cluster_stats().await.expect("stats");
        assert_eq!(
            (after.messages, after.migrations),
            (before.messages, before.migrations),
            "the world is at rest after part {part}"
        );
        handle.audit().await.expect("money and goods balance over all processes");
        states.push(cluster_digests(handle).await);
    }
    let stats = handle.cluster_stats().await.expect("stats");
    let run = Run {
        travelled: script.travelled(),
        living: script.living().len(),
        record: script.record,
        states,
        dropped: stats.dropped_messages,
    };
    cluster.stop();
    (run, stats)
}

#[tokio::test]
async fn three_processes_reach_the_state_of_one_process() {
    let config = world();
    let one = in_one_process(&config).await;
    let (three, stats) = in_a_cluster(&config, 3).await;

    // The script did what it is for: residents crossed the borders of the
    // processes, messages went between realms, some residents left the
    // world.
    assert_eq!(stats.workers.len(), 3);
    assert!(one.travelled >= RESIDENTS - 2, "{} residents left their first zone", one.travelled);
    assert!(one.living < RESIDENTS, "a resident left the world");
    assert!(stats.migrations > 200, "{stats:?}");
    assert!(stats.messages > 1_000, "{stats:?}");

    // Every answer to every input and every observation of every resident
    // is the same, frame by frame.
    assert_eq!(one.record.len(), three.record.len());
    for (frame, (single, cluster)) in one.record.iter().zip(&three.record).enumerate() {
        assert_eq!(single, cluster, "frame {frame} of {}", one.record.len());
    }
    assert_eq!((one.travelled, one.living), (three.travelled, three.living));

    // The state of every realm is the same after each part: the global
    // realm and the nine zones.
    assert_eq!(one.states.len(), PARTS as usize);
    for (part, (single, cluster)) in one.states.iter().zip(&three.states).enumerate() {
        assert_eq!(single.len(), 10, "the global realm and nine zones");
        assert_eq!(single, cluster, "the states after part {part}");
    }
    assert_ne!(one.states[0], one.states[1], "the parts change the world");

    // The kernel dropped the same messages: those for residents that are
    // gone. A message that a world of the cluster dropped because it had
    // never seen the resident would count here.
    assert_eq!(one.dropped, three.dropped);
}
