//! Recovery and the replay check: a snapshot plus the input log gives the
//! world back.
//!
//! [`recover`] is for a start after a stop of any kind: it restores the
//! newest snapshot that it can use and applies every later input of the
//! log. [`replay`] is for an audit: it starts from an older snapshot and
//! compares the state hash with every later snapshot on the way. Neither
//! changes the storage.

use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};

use unsup_kernel::{Input, Plugin, World};

use crate::log::{LogError, LogIndex, TornTail, read_from};
use crate::snapshot::{self, SnapshotEntry, SnapshotMeta};
use crate::storage::{Storage, StorageError};

/// A snapshot that a recovery or a replay could not use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedSnapshot {
    /// Storage name of the snapshot.
    pub name: String,
    /// Why it was not used.
    pub reason: String,
}

/// A later snapshot that a replay passed, and the state hash at that point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Checkpoint {
    /// Frame of the snapshot.
    pub frame: u64,
    /// Number of inputs in the snapshot.
    pub seq: u64,
    /// The state hash that the live run wrote into the snapshot.
    pub expected: [u8; 32],
    /// The state hash of the replayed world at the same point.
    pub found: [u8; 32],
}

impl Checkpoint {
    /// Returns true when the replay reached the same state as the live run.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.expected == self.found
    }
}

/// What a recovery or a replay did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplayReport {
    /// Frame of the snapshot that was restored.
    pub snapshot_frame: u64,
    /// Number of inputs in that snapshot.
    pub snapshot_seq: u64,
    /// True when the restored world has the state hash that the snapshot
    /// names. It can be false after a change of the plugins, which give a
    /// new component its default value.
    pub snapshot_hash_ok: bool,
    /// Inputs of the log that were applied after the snapshot.
    pub inputs: u64,
    /// How many of those inputs were frames.
    pub frames: u64,
    /// Number of the last frame of the world at the end.
    pub frame: u64,
    /// Number of inputs in the world at the end: the sequence number of the
    /// next input.
    pub next_seq: u64,
    /// State hash of the world at the end.
    pub state_hash: [u8; 32],
    /// Later snapshots that the replay passed. [`recover`] starts at the
    /// newest snapshot, so it has none.
    pub checkpoints: Vec<Checkpoint>,
    /// The end of the log that a crash cut off, when there is one. It was
    /// ignored.
    pub torn_tail: Option<TornTail>,
    /// Snapshots that could not be used, with the reason.
    pub skipped: Vec<SkippedSnapshot>,
}

impl ReplayReport {
    /// Returns true when the snapshot and every checkpoint gave the state
    /// hash that the live run recorded.
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.snapshot_hash_ok && self.checkpoints.iter().all(Checkpoint::matches)
    }
}

/// A world that was recovered from storage.
#[derive(Debug)]
pub struct Recovery {
    /// The world, as it was after the last input of the log.
    pub world: World,
    /// What the recovery did.
    pub report: ReplayReport,
}

/// A recovery or a replay failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoverError {
    /// The storage holds no snapshot and no log. Make a new world.
    Empty,
    /// No snapshot can be used.
    NoSnapshot {
        /// The snapshots that were tried.
        skipped: Vec<SkippedSnapshot>,
    },
    /// The storage failed.
    Storage(StorageError),
    /// The input log cannot be read.
    Log(LogError),
    /// An input of the log made the simulation panic.
    Panic {
        /// Sequence number of the input.
        seq: u64,
        /// The panic message.
        message: String,
    },
}

impl fmt::Display for RecoverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoverError::Empty => f.write_str("the storage holds no world"),
            RecoverError::NoSnapshot { skipped } => {
                f.write_str("no snapshot can be used")?;
                for snapshot in skipped {
                    write!(f, "; `{}`: {}", snapshot.name, snapshot.reason)?;
                }
                Ok(())
            }
            RecoverError::Storage(e) => write!(f, "cannot read the storage: {e}"),
            RecoverError::Log(e) => e.fmt(f),
            RecoverError::Panic { seq, message } => write!(
                f,
                "input {seq} of the log made the simulation panic: {message}. \
                 Start again with `drop_from = {seq}` to drop this input and all later ones"
            ),
        }
    }
}

impl std::error::Error for RecoverError {}

impl From<StorageError> for RecoverError {
    fn from(error: StorageError) -> RecoverError {
        RecoverError::Storage(error)
    }
}

impl From<LogError> for RecoverError {
    fn from(error: LogError) -> RecoverError {
        RecoverError::Log(error)
    }
}

/// Options of [`recover_with`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RecoverOptions {
    /// Stop before the input with this sequence number: that input and all
    /// later ones are not applied. An operator uses it to get past an input
    /// that makes the simulation panic.
    pub stop_before: Option<u64>,
}

/// Where a [`replay`] starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReplayFrom {
    /// The oldest snapshot that can be used: the longest check.
    #[default]
    Oldest,
    /// The newest snapshot that can be used: what a recovery does.
    Newest,
    /// The newest snapshot at or before this frame.
    Frame(u64),
}

/// Options of [`replay`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayOptions {
    /// The snapshot to start from.
    pub from: ReplayFrom,
}

/// A snapshot that was restored.
struct Start {
    world: World,
    meta: SnapshotMeta,
    hash_ok: bool,
}

/// Returns the message of a panic.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
        .unwrap_or_else(|| "no message".to_string())
}

/// Restores the first snapshot of `candidates` that can be used.
fn restore_first<'a>(
    storage: &dyn Storage,
    plugins: &[&dyn Plugin],
    index: &LogIndex,
    candidates: impl Iterator<Item = &'a SnapshotEntry>,
    skipped: &mut Vec<SkippedSnapshot>,
) -> Option<Start> {
    for entry in candidates {
        let mut skip = |reason: String| {
            skipped.push(SkippedSnapshot { name: entry.name.clone(), reason });
        };
        let loaded = match snapshot::load(storage, entry) {
            Ok(loaded) => loaded,
            Err(reason) => {
                skip(reason);
                continue;
            }
        };
        if index.voids(loaded.meta.seq, loaded.meta.segment) {
            skip("a rollback dropped inputs that this snapshot holds".into());
            continue;
        }
        match World::restore(plugins, loaded.payload()) {
            Ok(world) => {
                let hash_ok = world.state_hash() == loaded.meta.state_hash;
                return Some(Start { world, meta: loaded.meta, hash_ok });
            }
            Err(error) => skip(error.to_string()),
        }
    }
    None
}

/// Applies the log to a restored snapshot and writes the report.
fn run(
    storage: &dyn Storage,
    index: &LogIndex,
    start: Start,
    stop_before: Option<u64>,
    later: &[SnapshotMeta],
    skipped: Vec<SkippedSnapshot>,
) -> Result<Recovery, RecoverError> {
    let Start { mut world, meta, hash_ok } = start;
    let mut inputs = 0u64;
    let mut frames = 0u64;
    let mut checkpoints = Vec::new();
    let mut panic = None;
    let check = |world: &World, seq: u64, checkpoints: &mut Vec<Checkpoint>| {
        for snapshot in later.iter().filter(|snapshot| snapshot.seq == seq) {
            checkpoints.push(Checkpoint {
                frame: snapshot.frame,
                seq,
                expected: snapshot.state_hash,
                found: world.state_hash(),
            });
        }
    };

    let until = stop_before.unwrap_or(u64::MAX);
    let end = read_from(storage, index, meta.seq, until, &mut |seq, input| {
        check(&world, seq, &mut checkpoints);
        let is_frame = matches!(input, Input::Step { .. });
        // A panic leaves the world half-changed. It is not used again: the
        // caller gets the error and the world is dropped.
        match catch_unwind(AssertUnwindSafe(|| world.apply(input))) {
            Ok(_) => {
                inputs += 1;
                frames += u64::from(is_frame);
                true
            }
            Err(payload) => {
                panic = Some((seq, panic_message(&*payload)));
                false
            }
        }
    })?;
    if let Some((seq, message)) = panic {
        return Err(RecoverError::Panic { seq, message });
    }
    check(&world, end.next_seq, &mut checkpoints);

    let report = ReplayReport {
        snapshot_frame: meta.frame,
        snapshot_seq: meta.seq,
        snapshot_hash_ok: hash_ok,
        inputs,
        frames,
        frame: world.frame(),
        next_seq: end.next_seq,
        state_hash: world.state_hash(),
        checkpoints,
        torn_tail: end.torn,
        skipped,
    };
    Ok(Recovery { world, report })
}

/// Loads the newest snapshot that can be used and applies every later
/// input of the log.
///
/// A snapshot that is damaged or that the plugins cannot restore is passed
/// over for an older one; the report names it. A record at the end of the
/// log that a crash cut off is ignored; the report names it too. The
/// storage is not changed.
///
/// # Errors
///
/// [`RecoverError::Empty`] when nothing is stored: make a new world then.
/// The other errors mean that a stored world cannot be read.
pub fn recover(storage: &dyn Storage, plugins: &[&dyn Plugin]) -> Result<Recovery, RecoverError> {
    recover_with(storage, plugins, &RecoverOptions::default())
}

/// Like [`recover`], with options.
///
/// # Errors
///
/// See [`recover`].
pub fn recover_with(
    storage: &dyn Storage,
    plugins: &[&dyn Plugin],
    options: &RecoverOptions,
) -> Result<Recovery, RecoverError> {
    let entries = snapshot::list(storage)?;
    let index = LogIndex::load(storage)?;
    if entries.is_empty() && index.is_empty() {
        return Err(RecoverError::Empty);
    }
    let mut skipped = Vec::new();
    let candidates = entries
        .iter()
        .rev()
        .filter(|entry| options.stop_before.is_none_or(|stop| entry.seq <= stop));
    let Some(start) = restore_first(storage, plugins, &index, candidates, &mut skipped) else {
        return Err(RecoverError::NoSnapshot { skipped });
    };
    run(storage, &index, start, options.stop_before, &[], skipped)
}

/// Replays the log from a snapshot and reports the frames, the inputs and
/// the final state hash. On the way it compares the state hash with the
/// one in every later snapshot.
///
/// A later snapshot that the replay cannot compare is in
/// `ReplayReport::skipped` with the reason: the file is damaged, a rollback
/// made it void, or the log ends before it.
///
/// Anybody who has the storage and the plugins can run it to check that
/// the recorded inputs give the recorded world. The storage is not changed.
///
/// # Errors
///
/// See [`recover`].
pub fn replay(
    storage: &dyn Storage,
    plugins: &[&dyn Plugin],
    options: &ReplayOptions,
) -> Result<ReplayReport, RecoverError> {
    let entries = snapshot::list(storage)?;
    let index = LogIndex::load(storage)?;
    if entries.is_empty() && index.is_empty() {
        return Err(RecoverError::Empty);
    }
    let mut skipped = Vec::new();
    let start = match options.from {
        ReplayFrom::Oldest => restore_first(storage, plugins, &index, entries.iter(), &mut skipped),
        ReplayFrom::Newest => {
            restore_first(storage, plugins, &index, entries.iter().rev(), &mut skipped)
        }
        ReplayFrom::Frame(frame) => restore_first(
            storage,
            plugins,
            &index,
            entries.iter().rev().filter(|entry| entry.frame <= frame),
            &mut skipped,
        ),
    };
    let Some(start) = start else {
        return Err(RecoverError::NoSnapshot { skipped });
    };
    // The headers of the later snapshots are the checkpoints.
    let mut later = Vec::new();
    let mut names = Vec::new();
    for entry in entries.iter().filter(|entry| entry.seq > start.meta.seq) {
        match snapshot::head(storage, entry) {
            Ok(meta) if index.voids(meta.seq, meta.segment) => skipped.push(SkippedSnapshot {
                name: entry.name.clone(),
                reason: "a rollback dropped inputs that this snapshot holds".into(),
            }),
            Ok(meta) => {
                later.push(meta);
                names.push(&entry.name);
            }
            Err(reason) => skipped.push(SkippedSnapshot { name: entry.name.clone(), reason }),
        }
    }
    let mut report = run(storage, &index, start, None, &later, skipped)?.report;
    // A snapshot behind the end of the log cannot be checked: the log lost
    // its last records after the snapshot was written.
    for (meta, name) in later.iter().zip(names) {
        if meta.seq > report.next_seq {
            report.skipped.push(SkippedSnapshot {
                name: name.clone(),
                reason: format!(
                    "the log ends before this snapshot; missing inputs: {}",
                    meta.seq - report.next_seq
                ),
            });
        }
    }
    Ok(report)
}
