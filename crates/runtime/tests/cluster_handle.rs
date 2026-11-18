//! The handle of a cluster does what the handle of one process does, and
//! the split of the zones over the workers does not change the world.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use unsup_kernel::{RealmId, ZoneTiming};
use unsup_protocol::{
    AdminCommand, AdminReply, ObserveOpts, ResidentId, SimError, ViewQuery, WaitFilter, WorldView,
    ZoneId,
};
use unsup_runtime::cluster::{ClusterConfig, ClusterError, ClusterHandle};
use unsup_runtime::{
    Fault, Pacer, PostError, RollbackError, RuntimeConfig, SimHandle, SimStats, start,
};

use common::cluster::{Disks, Running, Setup};
use common::roads::{plugins, world_config};
use common::script::Script;
use common::{Decree, act, hex, spec};

/// Seed of the world and of the script.
const SEED: u64 = 31;

fn setup(workers: usize) -> Setup {
    Setup::new(plugins(), world_config(SEED, 3, 16), workers)
}

async fn hashes(handle: &ClusterHandle) -> Vec<(RealmId, String)> {
    let hashes = handle.hashes().await.expect("the hashes of the cluster");
    hashes.into_iter().map(|(realm, hash)| (realm, hex(&hash))).collect()
}

async fn until(handle: &SimHandle, condition: impl Fn(&SimStats) -> bool) -> SimStats {
    loop {
        let stats = handle.stats().await.expect("stats");
        if condition(&stats) {
            return stats;
        }
        tokio::task::yield_now().await;
    }
}

#[tokio::test]
async fn the_split_over_the_workers_does_not_change_any_realm() {
    // The same script in clusters of one, three and nine workers, and in
    // one of three workers whose zones are mixed and not in order. The
    // hashes are those of the worlds of the realms, so they can be
    // compared after every frame, also while messages are on the way.
    let mixed: Vec<Vec<ZoneId>> =
        [[8, 0, 4], [7, 2, 3], [1, 6, 5]].iter().map(|zones| zones.map(ZoneId).to_vec()).collect();
    let mut runs = Vec::new();
    for (workers, split) in [(1, None), (3, None), (9, None), (3, Some(mixed))] {
        let mut config = setup(workers).coordinator();
        if let Some(split) = split {
            config.workers = split;
        }
        let cluster = setup(workers).start_with(config).expect("the cluster starts");
        let handle = cluster.handle.clone();
        let mut script = Script::new(SEED, &setup(workers).world);
        script.spawn(&handle, 10).await;
        let mut frames = vec![hashes(&handle).await];
        for round in 0..40 {
            script.round(&handle, round).await;
            frames.push(hashes(&handle).await);
        }
        let stats = handle.cluster_stats().await.expect("stats");
        assert_eq!(stats.workers.len(), workers);
        handle.audit().await.expect("the union balances");
        cluster.stop();
        runs.push((frames, script.record, stats.migrations, stats.messages));
    }
    let (one, three, nine, mixed) = (&runs[0], &runs[1], &runs[2], &runs[3]);
    assert_eq!(one.0.len(), 41);
    assert_eq!(one.0[0].len(), 10, "the global realm and nine zones");
    assert!(
        one.2 > 20 && one.3 > 500,
        "residents and messages crossed borders: {} {}",
        one.2,
        one.3
    );
    for (frame, hashes) in one.0.iter().enumerate() {
        assert_eq!(hashes, &three.0[frame], "frame {frame}, one worker and three");
        assert_eq!(hashes, &nine.0[frame], "frame {frame}, one worker and nine");
        assert_eq!(hashes, &mixed.0[frame], "frame {frame}, one worker and three mixed");
    }
    assert_ne!(one.0[39], one.0[40], "the frames change the world");
    assert_eq!(one.1, three.1, "every answer and every observation");
    assert_eq!(one.1, nine.1, "every answer and every observation");
    assert_eq!((one.2, one.3), (three.2, three.3), "the router did the same work");
    assert_eq!((one.2, one.3), (nine.2, nine.3), "the router did the same work");
    assert_eq!((&one.1, one.2, one.3), (&mixed.1, mixed.2, mixed.3), "the mixed split");
}

#[tokio::test]
async fn a_wait_ends_when_news_comes_in_another_process() {
    let cluster = setup(3).start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    handle.act(ada, act("travel", json!({ "zone": 8 }))).await.expect("travel");

    // No news: a wait without time returns at once, with nothing.
    let now = handle.wait(ada, WaitFilter::Any, Duration::ZERO).await.expect("wait");
    assert_eq!((now.notices.len(), now.sections["reports"].clone()), (0, json!([])));

    // Two calls wait for Ada: one for any news, one for a kind of notice
    // that does not come.
    let waiting = tokio::spawn({
        let handle = handle.clone();
        async move { handle.wait(ada, WaitFilter::Any, Duration::from_secs(60)).await }
    });
    let other = tokio::spawn({
        let handle = handle.clone();
        let filter = WaitFilter::Kinds { kinds: vec!["town.decree".into()] };
        async move { handle.wait(ada, filter, Duration::from_secs(60)).await }
    });
    until(&handle, |stats| stats.waiting == 2).await;
    // Bob, in the first process, tells Ada, in the third. The notice
    // arrives with the next frame and ends the first wait only.
    handle.act(bob, act("tell", json!({ "to": ada, "kind": "town.greeting" }))).await.expect("act");
    handle.step(1).await.expect("step");
    let observation = waiting.await.expect("the task ran").expect("an observation");
    let kinds: Vec<&str> = observation.notices.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(kinds, ["town.greeting"]);
    assert_eq!(handle.stats().await.expect("stats").waiting, 1);

    // With the cursor of that observation nothing is new. With the cursor
    // before it the notice is there at once.
    let since = Some(observation.next_notice);
    let nothing = handle.wait_since(ada, WaitFilter::Any, since, Duration::ZERO).await;
    assert_eq!(nothing.expect("wait").notices.len(), 0);
    let again = handle.wait_since(ada, WaitFilter::Any, Some(0), Duration::from_secs(60)).await;
    assert_eq!(again.expect("wait").notices.len(), 1);

    // The other call ends when its time is up, with nothing for it. A
    // call that waits for an action ends with the report.
    handle.act(ada, act("work", json!({ "ticks": 2 }))).await.expect("act");
    let done = tokio::spawn({
        let handle = handle.clone();
        async move { handle.wait(ada, WaitFilter::ActionDone, Duration::from_secs(60)).await }
    });
    until(&handle, |stats| stats.waiting == 2).await;
    handle.step(2).await.expect("step");
    let report = done.await.expect("the task ran").expect("an observation");
    assert_eq!(report.sections["reports"][0]["verb"], json!("work"));
    let short = handle.wait(ada, WaitFilter::ActionDone, Duration::from_millis(20)).await;
    assert_eq!(short.expect("wait").sections["reports"], json!([]));
    // A resident that does not exist.
    let nobody = handle.wait(ResidentId(99), WaitFilter::Any, Duration::from_secs(1)).await;
    assert_eq!(nobody.map(|_| ()), Err(SimError::UnknownResident));

    // The stop ends the call that still waits.
    cluster.stop();
    assert_eq!(other.await.expect("the task ran").map(|_| ()), Err(SimError::Shutdown));
}

#[tokio::test]
async fn the_calls_of_a_sim_handle_work_on_a_cluster() {
    let cluster = setup(3).start().expect("the cluster starts");
    let handle = cluster.handle.sim();
    let mut frames = handle.frames();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");

    // The catalogue is the one of the plugins.
    let catalogue = handle.catalogue();
    let verbs: Vec<&str> = catalogue.verbs.iter().map(|v| v.verb.as_str()).collect();
    assert!(verbs.contains(&"walk") && verbs.contains(&"withdraw"), "{verbs:?}");

    // Views: the layout from the coordinator, a zone from its worker.
    handle.act(ada, act("travel", json!({ "zone": 5 }))).await.expect("travel");
    match handle.view(ViewQuery::Zone { zone: ZoneId(5) }).await.expect("the zone") {
        WorldView::Zone { zone, residents, .. } => {
            assert_eq!((zone.id, zone.residents), (ZoneId(5), 1));
            assert_eq!(residents.iter().map(|r| r.id).collect::<Vec<_>>(), [ada]);
        }
        other => panic!("not a zone: {other:?}"),
    }
    let no_zone = handle.view(ViewQuery::Zone { zone: ZoneId(9) }).await;
    assert_eq!(no_zone.map(|_| ()), Err(SimError::Invalid));

    // Commands of an administrator: a parameter goes to every zone, a
    // freeze to the zone of the resident.
    let set = AdminCommand::SetParam { name: "kernel.queue_max".into(), value: 1, city: None };
    assert_eq!(handle.admin(set).await, Ok(AdminReply::Ok));
    assert_eq!(handle.admin(AdminCommand::Freeze { resident: ada }).await, Ok(AdminReply::Ok));
    let frozen = handle.act(ada, act("chop", json!({}))).await.expect("act");
    assert!(!frozen.status.is_accepted(), "{frozen:?}");
    assert_eq!(handle.admin(AdminCommand::Unfreeze { resident: ada }).await, Ok(AdminReply::Ok));
    let nobody = handle.admin(AdminCommand::Freeze { resident: ResidentId(77) }).await;
    assert_eq!(
        nobody,
        Ok(AdminReply::Error {
            message: "The resident cannot be reached: no living resident has this id.".into()
        })
    );
    let zone = handle.admin(AdminCommand::RollbackZone { zone: ZoneId(1), frames: 1 }).await;
    assert!(
        matches!(&zone, Ok(AdminReply::Error { message }) if message.starts_with("A single zone cannot be rolled back.")),
        "{zone:?}"
    );
    handle.step(1).await.expect("step");
    // The new parameter is in zone 5: one planned action, not two.
    for expected in [true, true, false] {
        let ack = handle.act(ada, act("work", json!({ "ticks": 5 }))).await.expect("act");
        assert_eq!(ack.status.is_accepted(), expected, "{ack:?}");
    }

    // A message from outside goes to the global realm of the coordinator.
    let decree = Decree { title: "Fair".into(), to: ada, bonus: 30 };
    assert_eq!(handle.post_global(&decree).await, Ok(()));
    #[derive(serde::Serialize, serde::Deserialize)]
    struct Unheard {}
    impl unsup_kernel::Message for Unheard {
        const KIND: &'static str = "town.unheard";
    }
    assert_eq!(
        handle.post_global(&Unheard {}).await,
        Err(PostError::NoHandler { kind: "town.unheard".into() })
    );
    handle.step(2).await.expect("step");
    let seen = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
    assert_eq!(seen.sections["self"]["cash"], json!(130));

    // The stream of frames has every frame, with the changes of all
    // processes.
    let first = frames.recv().await.expect("frame 1");
    assert_eq!(first.frame, 1);
    assert_eq!(first.clock.minute, 1);
    let spawned: Vec<&str> = first.events.iter().map(|event| event.kind.as_str()).collect();
    assert_eq!(spawned, ["kernel.spawned"]);
    let entered: Vec<ZoneId> = first
        .zones
        .iter()
        .filter(|delta| !delta.entered.is_empty())
        .map(|delta| delta.zone)
        .collect();
    assert_eq!(entered, [ZoneId(0), ZoneId(5)], "she was made in zone 0 and went to zone 5");
    assert_eq!(frames.recv().await.expect("frame 2").frame, 2);

    // The numbers of the runtime, summed over the cluster.
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.frame, stats.frames_run, stats.residents), (3, 3, 1));
    // Fourteen inputs: the spawn, the travel, four commands of the
    // administrator (the kernel refused one of them, and that is an input
    // too), four actions, the decree and three frames. The zone rollback
    // and the message that nobody handles never were inputs.
    assert_eq!(stats.inputs_applied, 14);
    assert!(stats.mean_frame_micros > 0 && stats.p99_frame_micros >= stats.p50_frame_micros);

    // A cluster cannot roll back.
    assert_eq!(handle.rollback(0).await, Err(RollbackError::Unsupported));
    cluster.stop();
}

/// Makes zone 4 skip every second frame.
struct EverySecond {
    frames: u64,
    /// Every list of zone tick times that the pacer saw.
    seen: Arc<std::sync::Mutex<Vec<usize>>>,
}

impl Pacer for EverySecond {
    fn skip(&mut self, timings: &[ZoneTiming], _interval: Duration) -> Vec<ZoneId> {
        self.seen.lock().expect("no panic with the lock").push(timings.len());
        self.frames += 1;
        if self.frames.is_multiple_of(2) { vec![ZoneId(4)] } else { Vec::new() }
    }
}

#[tokio::test]
async fn a_zone_that_skips_frames_does_so_in_the_cluster_as_in_one_process() {
    // Ada walks through zone 4 while that zone skips every second frame.
    async fn walk(handle: &SimHandle) -> serde_json::Value {
        let ada = handle.spawn(spec("Ada")).await.expect("spawn");
        handle.act(ada, act("walk", json!({ "x": 40, "y": 40 }))).await.expect("walk");
        handle.step(30).await.expect("step");
        let seen = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
        json!({ "pos": seen.sections["self"]["pos"], "tick": seen.tick, "frame": seen.frame })
    }
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let pacer = || Box::new(EverySecond { frames: 0, seen: Arc::clone(&seen) });

    let plugins = plugins();
    let refs: Vec<&dyn unsup_kernel::Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    let world = unsup_kernel::World::new(&refs, world_config(SEED, 3, 16));
    let (handle, join) = start(world, RuntimeConfig { pacer: pacer(), ..RuntimeConfig::manual() });
    let single = walk(&handle).await;
    join.shutdown().expect("a clean stop");

    seen.lock().expect("no panic with the lock").clear();
    let setup = setup(3);
    let config = ClusterConfig { pacer: pacer(), ..setup.coordinator() };
    let running = setup.start_with(config).expect("the cluster starts");
    let cluster = walk(&running.handle).await;
    let stats = running.handle.stats().await.expect("stats");
    running.stop();

    // Eight frames to the corner of zone 4, then 16 tiles in zone 4 at
    // half speed: after 30 frames she is 11 tiles into it.
    assert_eq!(single["pos"], json!({ "x": 27, "y": 27 }));
    assert_eq!(single, cluster);
    assert_eq!(stats.zone_ticks_skipped, 15);
    // The pacer of the coordinator gets the tick time of every zone, from
    // the workers that measured it.
    let lists = seen.lock().expect("no panic with the lock").clone();
    assert_eq!(lists.len(), 30);
    assert_eq!(lists[0], 0, "no frame ran yet");
    assert!(lists[1..].iter().all(|&zones| zones == 9), "{lists:?}");
}

#[tokio::test]
async fn a_disk_that_fails_refuses_an_input_and_stops_the_cluster_at_a_frame() {
    let disks = Disks::new(2);
    let setup =
        Setup { disks: Some(disks.clone()), ..Setup::new(plugins(), world_config(SEED, 2, 16), 2) };
    let cluster = setup.start().expect("the cluster starts");
    let handle = cluster.handle.clone();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.step(1).await.expect("step");
    let before = hashes(&handle).await;

    // The disk of worker 0 fails. An input for its zone is not recorded,
    // so the world does not get it: the caller can try again.
    disks.workers[0].set_fault(Some(Fault::Append));
    assert_eq!(handle.act(ada, act("chop", json!({}))).await, Err(SimError::Overloaded));
    assert_eq!(handle.spawn(spec("Bob")).await, Err(SimError::Overloaded));
    assert_eq!(hashes(&handle).await, before, "nothing changed");
    assert_eq!(handle.stats().await.expect("stats").journal_errors, 2);
    disks.workers[0].set_fault(None);
    handle.act(ada, act("chop", json!({}))).await.expect("the disk works again");

    // The same for the global realm on the disk of the coordinator.
    disks.coordinator.set_fault(Some(Fault::Append));
    let set = AdminCommand::SetParam { name: "kernel.queue_max".into(), value: 2, city: None };
    assert_eq!(handle.admin(set).await, Err(SimError::Overloaded));
    let decree = Decree { title: "Lost".into(), to: ada, bonus: 5 };
    assert_eq!(handle.post_global(&decree).await, Err(PostError::Sim(SimError::Overloaded)));
    disks.coordinator.set_fault(None);

    // A frame cannot be refused by one realm alone: the others ran it.
    // The cluster stops and names the realm.
    disks.workers[1].set_fault(Some(Fault::Append));
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
    let Running { join, workers, .. } = cluster;
    let error = join.shutdown().expect_err("the cluster failed");
    assert!(
        matches!(&error, ClusterError::Failed { realm: RealmId::Zone(zone), message }
            if zone.0 >= 2 && message.starts_with("the journal failed:")),
        "{error:?}"
    );
    for worker in workers {
        worker.join().expect("the worker thread ran").expect_err("no stop in order");
    }

    // The disks hold a part of frame 2. A new start goes back to the last
    // seal, the end of frame 1. No seal covers the action that came after
    // that frame, so it is gone too.
    disks.workers[1].set_fault(None);
    let cluster = setup.start().expect("the cluster recovers");
    assert_eq!(cluster.handle.stats().await.expect("stats").frame, 1);
    assert_eq!(hashes(&cluster.handle).await, before);
    cluster.stop();
}

#[tokio::test]
async fn a_cluster_with_a_frame_interval_runs_frames_by_itself() {
    let setup = setup(3);
    let config = ClusterConfig { tick_interval: Duration::from_millis(5), ..setup.coordinator() };
    let cluster = setup.start_with(config).expect("the cluster starts");
    let handle = cluster.handle.sim();
    let mut frames = handle.frames();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.act(ada, act("walk", json!({ "x": 8, "y": 12 }))).await.expect("walk");
    // Nobody steps. The coordinator runs a frame every interval, in order.
    let mut seen = Vec::new();
    while seen.len() < 6 {
        seen.push(frames.recv().await.expect("a frame").frame);
    }
    assert_eq!(seen, [1, 2, 3, 4, 5, 6]);
    // A call that waits ends when the walk ends, four frames after it
    // started.
    let done = handle.wait_since(ada, WaitFilter::ActionDone, Some(0), Duration::from_secs(60));
    let report = done.await.expect("an observation");
    assert_eq!(report.sections["reports"][0]["result"], json!({ "pos": { "x": 8, "y": 12 } }));
    assert!(handle.stats().await.expect("stats").frames_run >= 6);
    cluster.stop();
}
