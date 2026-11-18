//! The journal for production: the input log and the snapshots in a
//! storage, with recovery at the start and rollback on request.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use unsup_kernel::{Input, Plugin, World};
use unsup_protocol::FrameOutput;

use crate::journal::{Journal, JournalError, JournalStats, Rollback, RollbackError};
use crate::log::{LogIndex, LogWriter, SegmentId, Syncer};
use crate::replay::{RecoverError, RecoverOptions, Recovery, ReplayReport, recover_with};
use crate::settings::{SyncSetting, settings};
use crate::snapshot::{self, SnapshotJob, SnapshotMeta, SnapshotWriter};
use crate::stats::micros;
use crate::storage::{Storage, StorageError};

/// The plugins of a world, in a form that the simulation thread can keep.
/// A rollback needs them to restore a snapshot.
pub type Plugins = Arc<Vec<Box<dyn Plugin>>>;

/// When the input log is forced to the disk.
///
/// With every policy an input is handed to the operating system before it
/// is applied. So a crash of the process alone never loses an input that
/// was applied. The policies differ in what a loss of power can lose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncPolicy {
    /// Force every input to the disk on its own, before it is applied. A
    /// loss of power loses nothing. This is the slowest policy.
    EveryRecord,
    /// Force every batch of inputs to the disk in one step, before the
    /// batch is applied. A loss of power loses nothing. The size of a batch
    /// is `RuntimeConfig::batch`.
    EveryBatch,
    /// A background thread forces the log to the disk every interval. A
    /// loss of power can lose the inputs of the last interval, also inputs
    /// whose callers got an answer.
    Interval(Duration),
}

/// How a [`DurableJournal`] writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableConfig {
    /// When the input log is forced to the disk.
    pub sync: SyncPolicy,
    /// A log segment rolls when it is larger than this many bytes.
    pub segment_bytes: u64,
    /// Greatest size of one record in bytes. A larger input is refused.
    pub max_record_bytes: u32,
    /// Frames between two snapshots. 0 turns the periodic snapshots off;
    /// the first and the final snapshot are still written.
    pub snapshot_every: u64,
    /// Number of snapshots to keep. The older ones are deleted, with the
    /// log segments that only they need.
    pub keep_snapshots: usize,
    /// For an operator, after a recovery failed with
    /// [`RecoverError::Panic`]: the recovery stops before the input with
    /// this sequence number, and that input and all later ones leave the
    /// log.
    pub drop_from: Option<u64>,
}

impl Default for DurableConfig {
    /// The numbers of `data/runtime.toml`.
    fn default() -> DurableConfig {
        let s = settings();
        DurableConfig {
            sync: match s.log.sync {
                SyncSetting::Record => SyncPolicy::EveryRecord,
                SyncSetting::Batch => SyncPolicy::EveryBatch,
                SyncSetting::Interval => {
                    SyncPolicy::Interval(Duration::from_millis(s.log.sync_interval_ms))
                }
            },
            segment_bytes: s.log.segment_bytes,
            max_record_bytes: s.log.max_record_bytes,
            snapshot_every: s.snapshots.every_frames,
            keep_snapshots: s.snapshots.keep,
            drop_from: None,
        }
    }
}

/// A durable world could not be opened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// The storage holds a world that cannot be recovered.
    Recover(RecoverError),
    /// The storage refused a write.
    Storage(StorageError),
    /// The operating system could not start a background thread.
    Thread(String),
}

impl fmt::Display for OpenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OpenError::Recover(e) => write!(f, "cannot recover the world: {e}"),
            OpenError::Storage(e) => write!(f, "cannot start the input log: {e}"),
            OpenError::Thread(e) => write!(f, "cannot start a thread of the journal: {e}"),
        }
    }
}

impl std::error::Error for OpenError {}

impl From<StorageError> for OpenError {
    fn from(error: StorageError) -> OpenError {
        OpenError::Storage(error)
    }
}

/// What [`DurableJournal::open`] returns.
pub struct Opened {
    /// The world: recovered from the storage, or new.
    pub world: World,
    /// The journal for this world. Give it to
    /// [`RuntimeConfig::journal`](crate::RuntimeConfig::journal).
    pub journal: DurableJournal,
    /// What the recovery did. `None` when the storage was empty and the
    /// world is new.
    pub recovery: Option<ReplayReport>,
}

impl fmt::Debug for Opened {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Opened")
            .field("world", &self.world)
            .field("recovery", &self.recovery)
            .finish_non_exhaustive()
    }
}

/// A [`Journal`] that makes the world durable.
///
/// - Every input goes to the append-only input log before it is applied
///   (see [`crate::log`]).
/// - Every `snapshot_every` frames the journal takes a snapshot (see
///   [`crate::snapshot`]). The simulation thread only serializes the world
///   and computes its hash; a background thread writes the file.
/// - A clean stop writes a final snapshot, so the next start replays
///   nothing.
/// - [`Journal::rollback`] restores an earlier snapshot and drops the later
///   log.
pub struct DurableJournal {
    storage: Arc<dyn Storage>,
    plugins: Plugins,
    config: DurableConfig,
    log: LogWriter,
    snapshots: SnapshotWriter,
    /// The sync thread of the `Interval` policy. It stops when it is
    /// dropped.
    _syncer: Option<Syncer>,
    /// A snapshot is due and waits for the writer thread to be free.
    snapshot_due: bool,
    /// Microseconds that the last capture took on the simulation thread.
    capture_micros: u64,
    /// Snapshots that could not be handed to the writer thread.
    submit_errors: u64,
}

impl fmt::Debug for DurableJournal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DurableJournal")
            .field("config", &self.config)
            .field("next_seq", &self.log.next_seq())
            .finish_non_exhaustive()
    }
}

/// Returns the plugins as the kernel wants them.
fn plugin_refs(plugins: &Plugins) -> Vec<&dyn Plugin> {
    plugins.iter().map(AsRef::as_ref).collect()
}

impl DurableJournal {
    /// Opens the durable state of a world.
    ///
    /// When the storage holds a world, the world is recovered: the newest
    /// usable snapshot plus every later input of the log (see
    /// [`recover`](crate::recover)). When the storage is empty, `create`
    /// makes a new world and the journal writes its first snapshot.
    ///
    /// The journal then starts a new log segment. A torn end of the old
    /// log, which a crash can leave, is behind it for good.
    ///
    /// # Errors
    ///
    /// Fails when a stored world cannot be recovered, when the storage
    /// refuses a write or when a background thread cannot start.
    pub fn open(
        storage: Arc<dyn Storage>,
        plugins: Plugins,
        config: DurableConfig,
        create: impl FnOnce(&[&dyn Plugin]) -> World,
    ) -> Result<Opened, OpenError> {
        let options = RecoverOptions { stop_before: config.drop_from };
        let (world, recovery) = {
            let refs = plugin_refs(&plugins);
            match recover_with(&*storage, &refs, &options) {
                Ok(Recovery { world, report }) => (world, Some(report)),
                Err(RecoverError::Empty) => (create(&refs), None),
                Err(error) => return Err(OpenError::Recover(error)),
            }
        };
        let index =
            LogIndex::load(&*storage).map_err(|e| OpenError::Recover(RecoverError::Log(e)))?;
        let id = SegmentId {
            index: index.last_index().map_or(0, |last| last + 1),
            first_seq: recovery.as_ref().map_or(0, |report| report.next_seq),
        };
        // The snapshot that a recovery would start from now.
        let (base_frame, base_seq) = match &recovery {
            Some(report) => (report.snapshot_frame, report.snapshot_seq),
            None => {
                // The first snapshot comes before the first segment. A
                // crash between the two leaves a world that a recovery can
                // read.
                let meta = SnapshotMeta {
                    frame: world.frame(),
                    seq: 0,
                    segment: id.index,
                    state_hash: world.state_hash(),
                };
                snapshot::write(&*storage, &meta, &world.snapshot())?;
                (meta.frame, meta.seq)
            }
        };
        let log = LogWriter::open(
            Arc::clone(&storage),
            id,
            config.sync,
            config.segment_bytes,
            config.max_record_bytes,
        )?;
        let thread = |e: std::io::Error| OpenError::Thread(e.to_string());
        let snapshots = SnapshotWriter::start(
            Arc::clone(&storage),
            Arc::clone(log.shared()),
            config.keep_snapshots,
        )
        .map_err(thread)?;
        snapshots.shared().last_frame.store(base_frame, Ordering::Relaxed);
        snapshots.shared().last_seq_plus_one.store(base_seq + 1, Ordering::Relaxed);
        let syncer = match config.sync {
            SyncPolicy::Interval(interval) => {
                Some(Syncer::start(Arc::clone(log.shared()), interval).map_err(thread)?)
            }
            SyncPolicy::EveryRecord | SyncPolicy::EveryBatch => None,
        };
        let journal = DurableJournal {
            storage,
            plugins,
            config,
            log,
            snapshots,
            _syncer: syncer,
            snapshot_due: false,
            capture_micros: 0,
            submit_errors: 0,
        };
        Ok(Opened { world, journal, recovery })
    }

    /// Serializes the world and hands the snapshot to the writer thread.
    /// With `wait` it returns when the snapshot is on the disk.
    fn capture(&mut self, world: &World, wait: bool) {
        let start = Instant::now();
        let payload = world.snapshot();
        let state_hash = world.state_hash();
        self.capture_micros = micros(start.elapsed());
        let meta = SnapshotMeta {
            frame: world.frame(),
            seq: self.log.next_seq(),
            segment: self.log.segment_index(),
            state_hash,
        };
        let job = SnapshotJob { meta, payload };
        let result = if wait { self.snapshots.write_now(job) } else { self.snapshots.submit(job) };
        match result {
            Ok(()) => self.snapshot_due = false,
            // A failed write was counted and logged by the writer thread.
            Err(_) if wait => {}
            Err(error) => {
                self.submit_errors += 1;
                tracing::error!(%error, frame = meta.frame, "a snapshot could not be started");
            }
        }
    }
}

impl Journal for DurableJournal {
    fn record(&mut self, input: &Input) -> Result<(), JournalError> {
        self.log.record(input)
    }

    fn commit(&mut self) -> Result<(), JournalError> {
        self.log.commit()
    }

    fn frame_done(&mut self, world: &World, frame: &FrameOutput) {
        let every = self.config.snapshot_every;
        if self.log.take_snapshot_wanted() || (every > 0 && frame.frame.is_multiple_of(every)) {
            self.snapshot_due = true;
        }
        // A snapshot that comes due while the last one is still being
        // written waits for a later frame. The thread never holds two.
        if self.snapshot_due && !self.snapshots.is_busy() {
            self.capture(world, false);
        }
    }

    fn discard(&mut self, unapplied: usize) {
        self.log.discard(unapplied as u64);
    }

    fn shutdown(&mut self, world: &World) {
        if let Err(error) = self.log.sync_now() {
            tracing::error!(%error, "the input log could not be forced to the disk at the stop");
        }
        self.snapshots.wait_idle();
        let have = self.snapshots.shared().last_seq_plus_one.load(Ordering::Relaxed);
        if have != self.log.next_seq() + 1 {
            self.capture(world, true);
        }
    }

    fn rollback(&mut self, frame: u64) -> Result<Rollback, RollbackError> {
        let failed = |message: String| RollbackError::Failed { message };
        self.log.sync_now().map_err(|e| failed(e.to_string()))?;
        // A snapshot that is still being written must be in the list.
        self.snapshots.wait_idle();
        let storage = &*self.storage;
        let index = LogIndex::load(storage).map_err(|e| failed(e.to_string()))?;
        let entries = snapshot::list(storage).map_err(|e| failed(e.to_string()))?;
        let refs = plugin_refs(&self.plugins);

        for entry in entries.iter().rev().filter(|entry| entry.frame <= frame) {
            let loaded = match snapshot::load(storage, entry) {
                Ok(loaded) if !index.voids(loaded.meta.seq, loaded.meta.segment) => loaded,
                Ok(_) => continue,
                Err(reason) => {
                    tracing::warn!(name = %entry.name, %reason, "a rollback passed over a snapshot");
                    continue;
                }
            };
            let world = match World::restore(&refs, loaded.payload()) {
                Ok(world) => world,
                Err(error) => {
                    tracing::warn!(name = %entry.name, %error, "a rollback passed over a snapshot");
                    continue;
                }
            };
            let seq = loaded.meta.seq;
            let dropped_inputs = self.log.next_seq().saturating_sub(seq);
            // The new segment is the point of no return: from here on a
            // recovery gives the world of the snapshot.
            self.log.restart_at(seq).map_err(|e| failed(e.to_string()))?;
            for later in entries.iter().filter(|later| later.seq > seq) {
                // The log already makes these snapshots void. Deleting
                // them only frees space; the next rotation tries again.
                if let Err(error) = storage.delete(&later.name) {
                    tracing::warn!(%error, "a snapshot from after the rollback could not be deleted");
                }
            }
            let shared = self.snapshots.shared();
            shared.last_frame.store(loaded.meta.frame, Ordering::Relaxed);
            shared.last_seq_plus_one.store(seq + 1, Ordering::Relaxed);
            self.snapshot_due = false;
            return Ok(Rollback { world, frame: loaded.meta.frame, dropped_inputs });
        }
        Err(RollbackError::NoSnapshot { frame, oldest: entries.first().map(|entry| entry.frame) })
    }

    fn stats(&self) -> JournalStats {
        let shared = self.snapshots.shared();
        JournalStats {
            records: self.log.next_seq(),
            lag: self.log.shared().lag(),
            write_errors: self.log.errors() + self.log.shared().sync_errors.load(Ordering::Relaxed),
            snapshots: shared.written.load(Ordering::Relaxed),
            snapshot_errors: shared.errors.load(Ordering::Relaxed) + self.submit_errors,
            last_snapshot_frame: shared.last_frame.load(Ordering::Relaxed),
            last_snapshot_capture_micros: self.capture_micros,
            last_snapshot_write_micros: shared.last_write_micros.load(Ordering::Relaxed),
            last_snapshot_bytes: shared.last_bytes.load(Ordering::Relaxed),
        }
    }
}
