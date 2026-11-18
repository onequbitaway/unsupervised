//! Rollback of the whole world to a snapshot.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use unsup_kernel::{ResidentId, World, ZoneId};
use unsup_protocol::{AdminCommand, AdminReply, ObserveOpts, SimError, WaitFilter};
use unsup_runtime::{
    DurableConfig, DurableJournal, MemStorage, RollbackError, RollbackReport, RuntimeConfig,
    RuntimeJoin, SimHandle, SimStats, Storage, SyncPolicy, recover, replay, start,
};

use common::{Town, act, hash_of, new_world, play, plugins};

fn config(snapshot_every: u64, keep_snapshots: usize) -> DurableConfig {
    DurableConfig {
        sync: SyncPolicy::EveryBatch,
        snapshot_every,
        keep_snapshots,
        ..DurableConfig::default()
    }
}

fn open(storage: &MemStorage, config: DurableConfig) -> (SimHandle, RuntimeJoin) {
    let opened = DurableJournal::open(Arc::new(storage.clone()), plugins(), config, new_world)
        .expect("the storage opens");
    let runtime = RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
    start(opened.world, runtime)
}

async fn twin<F: Future<Output = ()>>(script: impl FnOnce(SimHandle) -> F) -> World {
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    script(handle).await;
    join.shutdown().expect("a clean stop")
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

/// Plays rounds and waits for every snapshot that comes due.
async fn play_with_snapshots(handle: &SimHandle, from: u64, to: u64, every: u64) {
    for n in from..to {
        play(handle, n, n + 1).await;
        if (n + 1).is_multiple_of(every) {
            until(handle, |s| s.last_snapshot_frame == n + 1).await;
        }
    }
}

#[tokio::test]
async fn the_world_goes_back_to_a_snapshot_and_the_later_log_is_gone() {
    let storage = MemStorage::new();
    let (handle, join) = open(&storage, config(4, 5));
    play_with_snapshots(&handle, 0, 10, 4).await;
    let before = handle.stats().await.expect("stats");
    assert_eq!((before.frame, before.last_snapshot_frame), (10, 8));

    // Back to the newest snapshot at or before frame 9: the one of frame 8.
    let report = handle.rollback(9).await.expect("the rollback works");
    let after = handle.stats().await.expect("stats");
    assert_eq!(report.frame, 8);
    assert_eq!(report.dropped_inputs, before.journal_records - after.journal_records);
    assert!(report.dropped_inputs > 2, "two frames and the acts of two rounds");
    assert_eq!((after.frame, after.rollbacks, after.last_snapshot_frame), (8, 1, 8));
    // Counters of the runtime are not history: they go on.
    assert_eq!(after.frames_run, 10);

    // The world is the world of frame 8, and it goes on from there. The
    // rounds 20 and 21 stand for what happens after the rollback.
    play(&handle, 20, 22).await;
    let world = join.shutdown().expect("a clean stop");
    let expected = twin(|handle| async move {
        play(&handle, 0, 8).await;
        play(&handle, 20, 22).await;
    })
    .await;
    assert_eq!(hash_of(&world), hash_of(&expected));
    assert_eq!(world.frame(), 10);
    world.audit().expect("money and goods balance after the rollback");

    // The rollback is permanent. A recovery gives the new history, and the
    // inputs of the dropped rounds 8 and 9 are in no replay.
    let recovery = recover(&storage, &[&Town::default()]).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    let audit =
        replay(&storage, &[&Town::default()], &Default::default()).expect("the replay runs");
    assert_eq!((audit.snapshot_frame, audit.frame, audit.frames), (0, 10, 10));
    assert_eq!(common::hex(&audit.state_hash), hash_of(&expected));
    assert!(audit.is_consistent(), "{audit:?}");
    // The checkpoints are the snapshots of the history that is left.
    let frames: Vec<u64> = audit.checkpoints.iter().map(|c| c.frame).collect();
    assert_eq!(frames, [4, 8, 10]);
}

#[tokio::test]
async fn a_crash_right_after_a_rollback_keeps_the_rollback() {
    let storage = MemStorage::new();
    let (handle, _join) = open(&storage, config(3, 5));
    play_with_snapshots(&handle, 0, 8, 3).await;
    assert_eq!(handle.rollback(6).await.map(|r| r.frame), Ok(6));
    // The process dies before any new input.
    let disk = storage.fork();
    let recovery = recover(&disk, &[&Town::default()]).expect("the world recovers");
    let expected = twin(|handle| async move { play(&handle, 0, 6).await }).await;
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    assert_eq!((recovery.report.snapshot_frame, recovery.report.inputs), (6, 0));

    // A second rollback, further back, and then a crash.
    let newest = storage.list("snapshots").expect("list").pop().expect("a snapshot");
    let snapshot_of_frame_6 = storage.read(&newest).expect("read");
    assert_eq!(handle.rollback(5).await.map(|r| r.frame), Ok(3));
    assert_eq!(storage.list("snapshots").expect("list").len(), 2, "frames 0 and 3 are left");
    play(&handle, 30, 31).await;
    let expected = twin(|handle| async move {
        play(&handle, 0, 3).await;
        play(&handle, 30, 31).await;
    })
    .await;
    let recovery = recover(&storage.fork(), &[&Town::default()]).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    assert_eq!(recovery.report.snapshot_frame, 3);

    // The rollback deletes the snapshots of the dropped history only to
    // free space. When a crash keeps one alive, the log still says that it
    // is void: a recovery does not go back to the dropped history.
    let disk = storage.fork();
    disk.write(&newest, &[&snapshot_of_frame_6]).expect("write");
    let recovery = recover(&disk, &[&Town::default()]).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    assert_eq!(recovery.report.snapshot_frame, 3);
    let skipped: Vec<(&str, &str)> =
        recovery.report.skipped.iter().map(|s| (s.name.as_str(), s.reason.as_str())).collect();
    assert_eq!(skipped, [(newest.as_str(), "a rollback dropped inputs that this snapshot holds")]);
}

#[tokio::test]
async fn a_rollback_needs_a_snapshot_that_is_old_enough() {
    let storage = MemStorage::new();
    // Only the newest snapshot is kept.
    let (handle, join) = open(&storage, config(4, 1));
    play_with_snapshots(&handle, 0, 9, 4).await;
    let before = handle.stats().await.expect("stats");

    assert_eq!(
        handle.rollback(7).await,
        Err(RollbackError::NoSnapshot { frame: 7, oldest: Some(8) })
    );
    // Nothing changed.
    let after = handle.stats().await.expect("stats");
    assert_eq!(
        (after.frame, after.journal_records, after.rollbacks),
        (before.frame, before.journal_records, 0)
    );
    // A frame in the future means the newest snapshot.
    assert_eq!(
        handle.rollback(1_000).await.map(|r| r.frame),
        Ok(8),
        "the newest snapshot at or before the frame"
    );
    let world = join.shutdown().expect("a clean stop");
    let expected = twin(|handle| async move { play(&handle, 0, 8).await }).await;
    assert_eq!(hash_of(&world), hash_of(&expected));
}

#[tokio::test]
async fn a_runtime_without_snapshots_cannot_roll_back() {
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    handle.step(3).await.expect("step");
    assert_eq!(handle.rollback(1).await, Err(RollbackError::Unsupported));
    assert_eq!(handle.stats().await.expect("stats").frame, 3);
    drop(join);
    assert_eq!(handle.rollback(1).await, Err(RollbackError::Sim(SimError::Shutdown)));
}

#[tokio::test]
async fn one_zone_alone_cannot_go_back() {
    let storage = MemStorage::new();
    let (handle, join) = open(&storage, config(2, 5));
    play_with_snapshots(&handle, 0, 4, 2).await;
    let before = handle.stats().await.expect("stats");

    let reply = handle
        .admin(AdminCommand::RollbackZone { zone: ZoneId(0), frames: 2 })
        .await
        .expect("the call is answered");
    let AdminReply::Error { message } = reply else {
        panic!("the single-zone rollback is refused, not {reply:?}");
    };
    assert!(message.contains("whole world"), "the answer names the way that works: {message}");
    // It is not an input: the world, the log and the counters are as before.
    let after = handle.stats().await.expect("stats");
    assert_eq!(
        (after.frame, after.inputs_applied, after.journal_records, after.rollbacks),
        (before.frame, before.inputs_applied, before.journal_records, 0)
    );
    let world = join.shutdown().expect("a clean stop");
    let expected = twin(|handle| async move { play(&handle, 0, 4).await }).await;
    assert_eq!(hash_of(&world), hash_of(&expected));
}

#[tokio::test]
async fn calls_that_wait_are_answered_when_the_world_goes_back() {
    let storage = MemStorage::new();
    let (handle, _join) = open(&storage, config(2, 5));
    play_with_snapshots(&handle, 0, 4, 2).await;
    // Resident 5 is born in round 4, after the snapshot of frame 4... the
    // rollback goes to frame 2, where only residents 1 to 2 exist.
    play(&handle, 4, 5).await;
    let waiting: Vec<_> = [ResidentId(1), ResidentId(5)]
        .into_iter()
        .map(|resident| {
            let handle = handle.clone();
            tokio::spawn(async move {
                handle.wait(resident, WaitFilter::Any, Duration::from_secs(60)).await
            })
        })
        .collect();
    until(&handle, |s| s.waiting == 2).await;
    let cursor =
        handle.observe(ResidentId(1), ObserveOpts::default()).await.expect("observe").next_notice;

    // The rounds 2, 3 and 4 are dropped. A round is a spawn, one act per
    // resident and a frame; round 3 has an admin command too: 5 + 7 + 7.
    assert_eq!(handle.rollback(3).await, Ok(RollbackReport { frame: 2, dropped_inputs: 19 }));
    let mut answers = Vec::new();
    for call in waiting {
        answers.push(call.await.expect("the task ran"));
    }
    // The resident that still exists gets the world as it is now. Its
    // cursor moved back with the inbox.
    let observation = answers[0].as_ref().expect("an observation");
    assert_eq!(observation.frame, 2);
    assert!(observation.notices.is_empty());
    assert!(observation.next_notice < cursor, "{} < {cursor}", observation.next_notice);
    // The resident that was born later is gone.
    assert_eq!(answers[1].as_ref().map(|_| ()), Err(&SimError::UnknownResident));
    assert_eq!(handle.stats().await.expect("stats").waiting, 0);

    // A cursor from the dropped future counts as "now": the call waits and
    // wakes for the next notice, it does not hang on numbers that the new
    // history has not reached.
    let stale = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .wait_since(ResidentId(1), WaitFilter::Any, Some(cursor), Duration::from_secs(60))
                .await
        }
    });
    until(&handle, |s| s.waiting == 1).await;
    let tell = act("tell", json!({ "to": 1, "kind": "town.after" }));
    handle.act(ResidentId(2), tell).await.expect("act");
    handle.step(1).await.expect("step");
    let observation = stale.await.expect("the task ran").expect("an observation");
    let kinds: Vec<&str> = observation.notices.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(kinds, ["town.after"]);
}
