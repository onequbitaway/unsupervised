//! The input log, the snapshots and recovery.

mod common;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use unsup_kernel::{Input, Plugin, ResidentId, World};
use unsup_protocol::{AdminCommand, SimError};
use unsup_runtime::{
    DurableConfig, DurableJournal, Fault, FsStorage, Journal, LogError, MemStorage, RecoverError,
    ReplayFrom, ReplayOptions, ReplayReport, RuntimeConfig, RuntimeJoin, SimHandle, SimStats,
    Storage, SyncPolicy, recover, replay, start,
};

use common::{Town, act, hash_of, new_world, play, play_inputs, plugins, spec};

/// An interval that never ends during a test: with it only the explicit
/// sync points (a snapshot, a new segment, the stop) force the log to the
/// disk.
const NEVER: SyncPolicy = SyncPolicy::Interval(Duration::from_secs(3_600));

fn config(sync: SyncPolicy, snapshot_every: u64, keep_snapshots: usize) -> DurableConfig {
    DurableConfig { sync, snapshot_every, keep_snapshots, ..DurableConfig::default() }
}

/// Opens the durable world of a storage and starts a runtime with manual
/// stepping for it.
fn open(
    storage: &MemStorage,
    config: DurableConfig,
) -> (SimHandle, RuntimeJoin, Option<ReplayReport>) {
    let opened = DurableJournal::open(Arc::new(storage.clone()), plugins(), config, new_world)
        .expect("the storage opens");
    let runtime = RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
    let (handle, join) = start(opened.world, runtime);
    (handle, join, opened.recovery)
}

/// Runs a script against a world that has no journal and is never
/// interrupted, and returns the world.
async fn twin<F: Future<Output = ()>>(script: impl FnOnce(SimHandle) -> F) -> World {
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    script(handle).await;
    join.shutdown().expect("a clean stop")
}

/// Waits until the statistics of the runtime satisfy a condition.
async fn until(handle: &SimHandle, condition: impl Fn(&SimStats) -> bool) -> SimStats {
    loop {
        let stats = handle.stats().await.expect("stats");
        if condition(&stats) {
            return stats;
        }
        tokio::task::yield_now().await;
    }
}

/// Plays the rounds `from..to` of the script. After every frame that is due
/// for a snapshot it waits until the snapshot is on the disk, so that the
/// next one is never put off.
async fn play_with_snapshots(handle: &SimHandle, from: u64, to: u64, every: u64) {
    for n in from..to {
        play(handle, n, n + 1).await;
        if (n + 1).is_multiple_of(every) {
            until(handle, |s| s.last_snapshot_frame == n + 1).await;
        }
    }
}

fn recover_town(storage: &MemStorage) -> Result<unsup_runtime::Recovery, RecoverError> {
    recover(storage, &[&Town::default()])
}

/// Returns the names of the log segments, oldest first.
fn segments(storage: &MemStorage) -> Vec<String> {
    storage.list("log").expect("list")
}

/// Returns the frames of the snapshots, oldest first. The name of a
/// snapshot is `snapshots/<sequence number>-<frame>.snap`.
fn snapshot_frames(storage: &MemStorage) -> Vec<u64> {
    storage
        .list("snapshots")
        .expect("list")
        .iter()
        .map(|name| {
            let frame = name.rsplit('-').next().and_then(|rest| rest.strip_suffix(".snap"));
            frame.and_then(|digits| digits.parse().ok()).expect("a snapshot name ends in a frame")
        })
        .collect()
}

/// Returns the position of record `n` (from 0) of a segment file: the
/// header has 32 bytes, and each record starts with its length.
fn record_offset(segment: &[u8], n: u64) -> usize {
    let mut offset = 32;
    for _ in 0..n {
        let len = u32::from_le_bytes(segment[offset..offset + 4].try_into().expect("4 bytes"));
        offset += 8 + len as usize;
    }
    offset
}

#[tokio::test]
async fn a_crash_loses_no_input_and_duplicates_none() {
    let storage = MemStorage::new();
    let (handle, _join, recovery) = open(&storage, config(SyncPolicy::EveryBatch, 4, 5));
    assert_eq!(recovery, None, "an empty storage gives a new world");
    play_with_snapshots(&handle, 0, 10, 4).await;
    // Inputs after the last frame are in the log too.
    play_inputs(&handle, 10).await;
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.frame, stats.last_snapshot_frame), (10, 8));

    // The process dies here. The disk holds what was written; nothing gets
    // a chance to stop in order.
    let disk = storage.fork();

    let at_crash = twin(|handle| async move {
        play(&handle, 0, 10).await;
        play_inputs(&handle, 10).await;
    })
    .await;
    let recovery = recover_town(&disk).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&at_crash));
    recovery.world.audit().expect("money and goods balance after the recovery");
    let report = &recovery.report;
    assert_eq!((report.snapshot_frame, report.frame, report.frames), (8, 10, 2));
    assert_eq!(report.next_seq, stats.journal_records);
    assert_eq!(report.inputs, report.next_seq - report.snapshot_seq);
    assert!(report.inputs > 2, "the acts of two rounds and of a third were replayed");
    assert!(report.snapshot_hash_ok);
    assert_eq!((report.torn_tail.clone(), report.skipped.len()), (None, 0));
    assert_eq!(common::hex(&report.state_hash), hash_of(&at_crash));

    // The server starts again on the same disk and the run goes on. At the
    // end the world is the one of a run that never stopped.
    let (handle, join, recovery) = open(&disk, config(SyncPolicy::EveryBatch, 4, 5));
    assert_eq!(recovery.as_ref().map(|r| r.next_seq), Some(stats.journal_records));
    handle.step(1).await.expect("step");
    play(&handle, 11, 14).await;
    let after = join.shutdown().expect("a clean stop");
    let uninterrupted = twin(|handle| async move { play(&handle, 0, 14).await }).await;
    assert_eq!(hash_of(&after), hash_of(&uninterrupted));
    assert_eq!(after.frame(), 14);
}

#[tokio::test]
async fn a_torn_last_record_is_ignored() {
    // What a crash in the middle of a write can leave: a record that is cut
    // off, or a whole record with wrong bytes.
    for damage in ["cut", "garbage"] {
        let storage = MemStorage::new();
        let (handle, _join, _) = open(&storage, config(SyncPolicy::EveryBatch, 0, 5));
        play(&handle, 0, 6).await;
        let records = handle.stats().await.expect("stats").journal_records;
        let disk = storage.fork();

        let name = segments(&disk).pop().expect("one segment");
        let mut bytes = disk.read(&name).expect("read");
        let whole = bytes.len();
        if damage == "cut" {
            bytes.truncate(whole - 3);
        } else {
            bytes[whole - 3] ^= 0xff;
        }
        disk.write(&name, &[&bytes]).expect("write");

        // The last record was the frame of round 5. The world comes back
        // as it was before that frame.
        let recovery = recover_town(&disk).expect("the world recovers");
        let before_frame = twin(|handle| async move {
            play(&handle, 0, 5).await;
            play_inputs(&handle, 5).await;
        })
        .await;
        assert_eq!(hash_of(&recovery.world), hash_of(&before_frame), "{damage}");
        assert_eq!(recovery.report.next_seq, records - 1);
        let torn = recovery.report.torn_tail.expect("the report names the torn record");
        assert_eq!((torn.segment.as_str(), torn.seq), (name.as_str(), records - 1));
        assert_eq!(torn.offset as usize, record_offset(&bytes, records - 1));
        assert_eq!((torn.offset + torn.ignored_bytes) as usize, bytes.len());

        // The journal goes on in a new segment that starts at the torn
        // record. The torn bytes stay where they are and never count again.
        let (handle, join, _) = open(&disk, config(SyncPolicy::EveryBatch, 0, 5));
        assert_eq!(
            segments(&disk),
            [name.clone(), format!("log/{:010}-{:020}.seg", 1, records - 1)]
        );
        handle.step(1).await.expect("step");
        let crashed_again = disk.fork();
        let whole_run = twin(|handle| async move { play(&handle, 0, 6).await }).await;
        let again = recover_town(&crashed_again).expect("the world recovers again");
        assert_eq!(hash_of(&again.world), hash_of(&whole_run), "{damage}");
        assert_eq!((again.report.next_seq, again.report.torn_tail), (records, None));
        assert_eq!(hash_of(&join.shutdown().expect("a clean stop")), hash_of(&whole_run));
    }
}

#[tokio::test]
async fn damage_in_the_middle_of_the_log_is_reported_not_skipped() {
    let storage = MemStorage::new();
    let (handle, _join, _) = open(&storage, config(SyncPolicy::EveryBatch, 0, 5));
    play(&handle, 0, 3).await;
    let target = handle.stats().await.expect("stats").journal_records;
    play(&handle, 3, 6).await;
    let disk = storage.fork();

    // One bit of the first input of round 3 flips on the disk.
    let name = segments(&disk).pop().expect("one segment");
    let mut bytes = disk.read(&name).expect("read");
    let offset = record_offset(&bytes, target);
    bytes[offset + 8] ^= 0x01;
    disk.write(&name, &[&bytes]).expect("write");

    // Valid records follow the damage, so it is not the torn end of the
    // log. To go on without them would lose inputs that were applied.
    let error = recover_town(&disk).expect_err("the damage is found");
    let RecoverError::Log(LogError::Corrupt { segment, seq, offset: at, .. }) = &error else {
        panic!("a damaged record, not {error:?}");
    };
    assert_eq!((segment.as_str(), *seq, *at as usize), (name.as_str(), target, offset));
    let opened = DurableJournal::open(
        Arc::new(disk.clone()),
        plugins(),
        config(SyncPolicy::EveryBatch, 0, 5),
        new_world,
    );
    assert!(matches!(opened, Err(unsup_runtime::OpenError::Recover(_))), "{opened:?}");

    // The operator decides: the world goes on from before the damage.
    let (_handle, join, recovery) = open(
        &disk,
        DurableConfig { drop_from: Some(target), ..config(SyncPolicy::EveryBatch, 0, 5) },
    );
    assert_eq!(recovery.expect("recovered").next_seq, target);
    let three_rounds = twin(|handle| async move { play(&handle, 0, 3).await }).await;
    assert_eq!(hash_of(&join.shutdown().expect("a clean stop")), hash_of(&three_rounds));
    // The choice is permanent: the next start needs no option.
    let again = recover_town(&disk).expect("the world recovers");
    assert_eq!(hash_of(&again.world), hash_of(&three_rounds));
}

#[tokio::test]
async fn snapshots_rotate_and_take_the_old_log_with_them() {
    let storage = MemStorage::new();
    // Small segments, so that the log rolls many times.
    let small = DurableConfig { segment_bytes: 600, ..config(SyncPolicy::EveryBatch, 2, 3) };
    let (handle, join, _) = open(&storage, small);
    play_with_snapshots(&handle, 0, 10, 2).await;
    let stats = handle.stats().await.expect("stats");
    let live = join.shutdown().expect("a clean stop");

    // The first snapshot (frame 0) and five periodic ones (frames 2 to 10)
    // were written. The newest three are left. The stop adds none, because
    // nothing happened after the snapshot of frame 10.
    assert_eq!(stats.frame, 10);
    assert_eq!(snapshot_frames(&storage), [6, 8, 10]);

    // The log rolled, and the segments that only older snapshots needed are
    // deleted: the first segment that is left does not start at 0.
    let names = segments(&storage);
    assert!(names.len() >= 2, "{names:?}");
    assert_ne!(names[0], format!("log/{:010}-{:020}.seg", 0, 0), "{names:?}");
    // Every snapshot that is left can still be replayed to the end.
    let report =
        replay(&storage, &[&Town::default()], &ReplayOptions::default()).expect("the replay runs");
    assert_eq!((report.snapshot_frame, report.frame, report.frames), (6, 10, 4));
    assert_eq!(common::hex(&report.state_hash), hash_of(&live));
    let passed: Vec<(u64, bool)> =
        report.checkpoints.iter().map(|c| (c.frame, c.matches())).collect();
    assert_eq!(passed, [(8, true), (10, true)]);
    assert_eq!(report.next_seq, stats.journal_records);
}

#[tokio::test]
async fn a_replay_gives_the_live_run() {
    let storage = MemStorage::new();
    let (handle, join, _) = open(&storage, config(SyncPolicy::EveryBatch, 3, 10));
    play_with_snapshots(&handle, 0, 10, 3).await;
    let stats = handle.stats().await.expect("stats");
    let live = join.shutdown().expect("a clean stop");

    // From the first snapshot: every input, every frame, and the state hash
    // of the live world at each later snapshot and at the end.
    let report =
        replay(&storage, &[&Town::default()], &ReplayOptions::default()).expect("the replay runs");
    assert_eq!((report.snapshot_frame, report.snapshot_seq), (0, 0));
    assert_eq!((report.inputs, report.frames, report.frame), (stats.journal_records, 10, 10));
    assert_eq!(report.next_seq, stats.journal_records);
    assert_eq!(common::hex(&report.state_hash), hash_of(&live));
    let frames: Vec<u64> = report.checkpoints.iter().map(|c| c.frame).collect();
    assert_eq!(frames, [3, 6, 9, 10], "three periodic snapshots and the final one");
    assert!(report.is_consistent(), "{report:?}");

    // From a later snapshot the replay is shorter and ends in the same
    // state.
    let from = ReplayOptions { from: ReplayFrom::Frame(7) };
    let later = replay(&storage, &[&Town::default()], &from).expect("the replay runs");
    assert_eq!((later.snapshot_frame, later.frames), (6, 4));
    assert_eq!(later.state_hash, report.state_hash);
    let newest = ReplayOptions { from: ReplayFrom::Newest };
    let last = replay(&storage, &[&Town::default()], &newest).expect("the replay runs");
    assert_eq!((last.snapshot_frame, last.inputs, last.checkpoints.len()), (10, 0, 0));

    // A build whose rules differ does not reach the recorded states, and
    // the replay says so: this town pays another wage.
    let other_rules = Town { wage: 501 };
    let report =
        replay(&storage, &[&other_rules], &ReplayOptions::default()).expect("the replay runs");
    assert!(report.snapshot_hash_ok, "the first snapshot restores the same");
    assert!(!report.is_consistent());
    assert!(report.checkpoints.iter().any(|c| !c.matches()));
    assert_ne!(common::hex(&report.state_hash), hash_of(&live));
}

#[tokio::test]
async fn a_replay_names_a_snapshot_that_the_log_does_not_reach() {
    let storage = MemStorage::new();
    let (handle, join, _) = open(&storage, config(SyncPolicy::EveryBatch, 0, 5));
    play(&handle, 0, 4).await;
    let records = handle.stats().await.expect("stats").journal_records;
    // The clean stop writes the final snapshot, after the last input.
    let live = join.shutdown().expect("a clean stop");

    // Later the disk loses the end of the log: the frame of round 3.
    let name = segments(&storage).pop().expect("one segment");
    let mut bytes = storage.read(&name).expect("read");
    bytes.truncate(bytes.len() - 3);
    storage.write(&name, &[&bytes]).expect("write");

    // The final snapshot stands in for the lost record: the world recovers.
    let recovery = recover_town(&storage).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&live));
    assert_eq!((recovery.report.snapshot_seq, recovery.report.inputs), (records, 0));

    // A replay from the first snapshot ends one input early. It cannot
    // compare itself with the final snapshot, and it says so.
    let report =
        replay(&storage, &[&Town::default()], &ReplayOptions::default()).expect("the replay runs");
    assert_eq!((report.next_seq, report.frame, report.checkpoints.len()), (records - 1, 3, 0));
    assert_eq!(report.torn_tail.as_ref().map(|torn| torn.seq), Some(records - 1));
    let final_snapshot = storage.list("snapshots").expect("list").pop().expect("a snapshot");
    let skipped: Vec<(&str, &str)> =
        report.skipped.iter().map(|s| (s.name.as_str(), s.reason.as_str())).collect();
    assert_eq!(
        skipped,
        [(final_snapshot.as_str(), "the log ends before this snapshot; missing inputs: 1")]
    );
    assert_ne!(common::hex(&report.state_hash), hash_of(&live));
}

#[tokio::test]
async fn a_clean_stop_writes_a_final_snapshot() {
    let storage = MemStorage::new();
    let (handle, join, _) = open(&storage, config(NEVER, 4, 5));
    play_with_snapshots(&handle, 0, 6, 4).await;
    // Inputs after the last frame belong to the final snapshot too.
    play_inputs(&handle, 6).await;
    let records = handle.stats().await.expect("stats").journal_records;
    let live = join.shutdown().expect("a clean stop");

    // Also after a loss of power right after the stop: the final snapshot
    // and the log are on the disk, though the sync interval never ended.
    for disk in [storage.fork(), storage.fork_durable()] {
        let recovery = recover_town(&disk).expect("the world recovers");
        assert_eq!(hash_of(&recovery.world), hash_of(&live));
        let report = recovery.report;
        assert_eq!((report.snapshot_frame, report.snapshot_seq), (6, records));
        assert_eq!((report.inputs, report.frames), (0, 0), "nothing to replay");
    }
    assert_eq!(snapshot_frames(&storage), [0, 4, 6]);

    // A second stop without a new input writes no second file.
    let (_handle, join, recovery) = open(&storage, config(NEVER, 4, 5));
    assert_eq!(recovery.expect("recovered").inputs, 0);
    join.shutdown().expect("a clean stop");
    assert_eq!(snapshot_frames(&storage), [0, 4, 6]);
}

#[tokio::test]
async fn the_sync_policy_decides_what_a_loss_of_power_keeps() {
    for (policy, lag, kept) in
        [(SyncPolicy::EveryRecord, 0, 3), (SyncPolicy::EveryBatch, 0, 3), (NEVER, 3, 0)]
    {
        let storage = MemStorage::new();
        let (handle, _join, _) = open(&storage, config(policy, 0, 5));
        for name in ["Ada", "Bob", "Cy"] {
            handle.spawn(spec(name)).await.expect("spawn");
        }
        // The three callers have their answers.
        let stats = handle.stats().await.expect("stats");
        assert_eq!((stats.journal_records, stats.journal_lag), (3, lag), "{policy:?}");

        // The process is killed: every policy has handed the inputs to the
        // operating system before they were applied.
        let killed = recover_town(&storage.fork()).expect("the world recovers");
        assert_eq!(killed.world.residents(), 3, "{policy:?}");
        // The power goes off: only what was forced to the disk is left.
        let dark = recover_town(&storage.fork_durable()).expect("the world recovers");
        assert_eq!(dark.world.residents(), kept, "{policy:?}");
        assert_eq!(dark.report.next_seq, kept as u64);
    }
}

#[tokio::test]
async fn a_failing_disk_refuses_inputs_and_the_log_stays_whole() {
    for (policy, fault) in [
        (SyncPolicy::EveryBatch, Fault::Append),
        (SyncPolicy::EveryBatch, Fault::Sync),
        (SyncPolicy::EveryRecord, Fault::Append),
        (NEVER, Fault::Append),
    ] {
        let storage = MemStorage::new();
        let (handle, join, _) = open(&storage, config(policy, 0, 5));
        let ada = handle.spawn(spec("Ada")).await.expect("spawn");

        storage.set_fault(Some(fault));
        // The input cannot be made durable, so it is not applied. Half of
        // its record is on the disk already when the append failed.
        assert_eq!(handle.spawn(spec("Bob")).await, Err(SimError::Overloaded), "{policy:?}");
        assert_eq!(handle.step(1).await, Err(SimError::Overloaded));
        let stats = handle.stats().await.expect("stats");
        assert_eq!((stats.residents, stats.frame, stats.journal_records), (1, 0, 1));
        assert_eq!(stats.journal_errors, 2);

        // The disk works again. After each failed write the journal went
        // on in a new segment that starts at the input that failed, so a
        // half-written record never counts.
        storage.set_fault(None);
        let cy = handle.spawn(spec("Cy")).await.expect("spawn");
        assert_eq!((ada, cy), (ResidentId(1), ResidentId(2)));
        handle.step(2).await.expect("step");
        let name = |index: u64, first_seq: u64| format!("log/{index:010}-{first_seq:020}.seg");
        assert_eq!(
            segments(&storage),
            [name(0, 0), name(1, 1), name(2, 1)],
            "{policy:?} {fault:?}"
        );

        let expected = twin(|handle| async move {
            handle.spawn(spec("Ada")).await.expect("spawn");
            handle.spawn(spec("Cy")).await.expect("spawn");
            handle.step(2).await.expect("step");
        })
        .await;
        let recovery = recover_town(&storage.fork()).expect("the world recovers");
        assert_eq!(hash_of(&recovery.world), hash_of(&expected), "{policy:?} {fault:?}");
        assert_eq!((recovery.report.next_seq, recovery.report.torn_tail), (4, None));
        assert_eq!(hash_of(&join.shutdown().expect("a clean stop")), hash_of(&expected));
    }
}

#[tokio::test]
async fn a_refused_input_does_not_come_back_after_a_crash() {
    for (policy, fault) in [
        (SyncPolicy::EveryBatch, Fault::Sync),
        (SyncPolicy::EveryRecord, Fault::Sync),
        (SyncPolicy::EveryBatch, Fault::Append),
        (NEVER, Fault::Append),
    ] {
        let storage = MemStorage::new();
        let (handle, _join, _) = open(&storage, config(policy, 0, 5));
        handle.spawn(spec("Ada")).await.expect("spawn");

        // The record of this input reaches the file, whole or in part, but
        // the journal cannot make it durable. The caller is told that
        // nothing changed.
        storage.set_fault(Some(fault));
        assert_eq!(handle.spawn(spec("Bob")).await, Err(SimError::Overloaded), "{policy:?}");
        storage.set_fault(None);

        // The process dies before the next input. The record must not count
        // in a recovery: the world has one resident, as the caller was told.
        let recovery = recover_town(&storage.fork()).expect("the world recovers");
        assert_eq!(recovery.world.residents(), 1, "{policy:?} {fault:?}");
        assert_eq!((recovery.report.next_seq, recovery.report.torn_tail), (1, None));
    }
}

#[tokio::test]
async fn a_failed_background_sync_is_covered_by_a_snapshot() {
    let storage = MemStorage::new();
    let (handle, _join, _) = open(&storage, config(NEVER, 2, 5));
    play(&handle, 0, 2).await;
    until(&handle, |s| s.last_snapshot_frame == 2).await;

    // The disk refuses to sync. Inputs are still written and applied: this
    // policy does not wait for the disk. The snapshot of frame 4 fails,
    // because it must not be newer than the log on the disk.
    storage.set_fault(Some(Fault::Sync));
    play(&handle, 2, 4).await;
    let stats = until(&handle, |s| s.snapshot_errors == 1).await;
    assert_eq!((stats.frame, stats.last_snapshot_frame), (4, 2));

    // The disk works again. The journal does not trust the old segment any
    // more: it starts a new one and takes a snapshot at the next frame,
    // which is not a frame of the schedule.
    storage.set_fault(None);
    play(&handle, 4, 5).await;
    until(&handle, |s| s.last_snapshot_frame == 5).await;
    assert_eq!(segments(&storage).len(), 2);

    // After a loss of power the snapshot stands in for the inputs that
    // never reached the disk.
    let expected = twin(|handle| async move { play(&handle, 0, 5).await }).await;
    let recovery = recover_town(&storage.fork_durable()).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    assert_eq!((recovery.report.snapshot_frame, recovery.report.inputs), (5, 0));
}

#[tokio::test]
async fn a_damaged_snapshot_is_passed_over_for_an_older_one() {
    let storage = MemStorage::new();
    // The stop is not clean: the newest snapshot is the one of frame 6.
    let (handle, _join, _) = open(&storage, config(SyncPolicy::EveryBatch, 3, 5));
    play_with_snapshots(&handle, 0, 7, 3).await;
    let disk = storage.fork();
    let expected = twin(|handle| async move { play(&handle, 0, 7).await }).await;

    let newest = disk.list("snapshots").expect("list").pop().expect("a snapshot");
    let mut bytes = disk.read(&newest).expect("read");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x01;
    disk.write(&newest, &[&bytes]).expect("write");

    let recovery = recover_town(&disk).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&expected));
    let report = recovery.report;
    assert_eq!((report.snapshot_frame, report.frames), (3, 4), "from the snapshot before");
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(
        (report.skipped[0].name.as_str(), report.skipped[0].reason.as_str()),
        (newest.as_str(), "the checksum of the payload is wrong")
    );

    // Without any snapshot the log alone cannot give the world back.
    for name in disk.list("snapshots").expect("list") {
        disk.delete(&name).expect("delete");
    }
    assert_eq!(recover_town(&disk).map(|_| ()), Err(RecoverError::NoSnapshot { skipped: vec![] }));
    assert_eq!(recover_town(&MemStorage::new()).map(|_| ()), Err(RecoverError::Empty));
}

#[tokio::test]
async fn an_input_that_panics_does_not_stay_in_the_log() {
    // Real files this time.
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("durability-panic");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("the old test directory can be removed");
    }
    let open_files = || {
        let storage = Arc::new(FsStorage::open(&dir).expect("open"));
        let opened = DurableJournal::open(
            storage,
            plugins(),
            config(SyncPolicy::EveryBatch, 0, 5),
            new_world,
        )
        .expect("the storage opens");
        let runtime =
            RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
        (start(opened.world, runtime), opened.recovery)
    };

    let ((handle, join), _) = open_files();
    play(&handle, 0, 3).await;
    // The module has a bug. The input is in the log before the world gets
    // it; the simulation thread dies while it applies it.
    assert_eq!(
        handle.act(ResidentId(1), act("crash", json!({}))).await.map(|_| ()),
        Err(SimError::Shutdown)
    );
    let panic = join.shutdown().expect_err("the thread panicked");
    assert_eq!(panic.message, "the town module has a bug");

    // The next start does not meet the input again.
    let ((handle, join), recovery) = open_files();
    let report = recovery.expect("the world was recovered");
    assert_eq!((report.frame, report.torn_tail.clone()), (3, None));
    play(&handle, 3, 5).await;
    let world = join.shutdown().expect("a clean stop");
    let expected = twin(|handle| async move { play(&handle, 0, 5).await }).await;
    assert_eq!(hash_of(&world), hash_of(&expected));
}

#[test]
fn a_log_with_an_input_that_panics_needs_the_operator() {
    // The journal without a runtime: the input reaches the log, and the
    // process dies before anything can take it back.
    let storage = MemStorage::new();
    let town = Town::default();
    let refs: [&dyn Plugin; 1] = [&town];
    let opened = DurableJournal::open(
        Arc::new(storage.clone()),
        plugins(),
        config(SyncPolicy::EveryBatch, 0, 5),
        new_world,
    )
    .expect("the storage opens");
    let (mut world, mut journal) = (opened.world, opened.journal);
    let mut apply = |world: &mut World, input: Input| {
        journal.record(&input).expect("record");
        journal.commit().expect("commit");
        world.apply(input);
    };
    apply(&mut world, Input::Spawn(spec("Ada")));
    apply(&mut world, Input::Step { skip: vec![] });
    let good = hash_of(&world);
    let crash = Input::Act { resident: ResidentId(1), request: act("crash", json!({})) };
    journal.record(&crash).expect("record");
    journal.commit().expect("commit");
    journal.record(&Input::Step { skip: vec![] }).expect("record");
    journal.commit().expect("commit");
    let disk = storage.fork();

    // A recovery finds the input and says which one it is.
    let error = recover(&disk, &refs).expect_err("the input panics again");
    assert_eq!(error, RecoverError::Panic { seq: 2, message: "the town module has a bug".into() });
    // The operator drops it, with everything after it.
    let opened = DurableJournal::open(
        Arc::new(disk.clone()),
        plugins(),
        DurableConfig { drop_from: Some(2), ..config(SyncPolicy::EveryBatch, 0, 5) },
        new_world,
    )
    .expect("the storage opens");
    assert_eq!(hash_of(&opened.world), good);
    assert_eq!(opened.recovery.expect("recovered").next_seq, 2);
    drop(opened.journal);
    let recovery = recover(&disk, &refs).expect("no option is needed any more");
    assert_eq!((hash_of(&recovery.world), recovery.report.next_seq), (good, 2));
}

#[tokio::test]
async fn an_admin_command_survives_a_crash() {
    // Parameters are state. A change must come back with the world.
    let storage = MemStorage::new();
    let (handle, _join, _) = open(&storage, config(SyncPolicy::EveryBatch, 0, 5));
    let command =
        AdminCommand::SetParam { name: "town.heavy_rounds".into(), value: 12, city: None };
    handle.admin(command).await.expect("admin");
    handle.step(1).await.expect("step");
    let recovery = recover_town(&storage.fork()).expect("the world recovers");
    let rounds =
        recovery.world.catalogue().params.into_iter().find(|p| p.name == "town.heavy_rounds");
    assert_eq!(rounds.map(|p| p.value), Some(12));
}
