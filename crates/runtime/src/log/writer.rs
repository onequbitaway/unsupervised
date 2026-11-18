//! The writing end of the input log.

use std::sync::Arc;

use unsup_kernel::Input;

use super::record;
use super::segment::{SEGMENT_HEADER, SegmentId};
use super::syncer::LogShared;
use crate::durable::SyncPolicy;
use crate::journal::JournalError;
use crate::storage::{Segment, Storage, StorageError};

/// Writes the header of a new segment and opens the segment. The header is
/// written atomically: the segment exists with its header or not at all.
fn create_segment(storage: &dyn Storage, id: SegmentId) -> Result<Arc<dyn Segment>, StorageError> {
    let name = id.name();
    storage.write(&name, &[&id.header()])?;
    storage.append(&name)
}

/// Appends inputs to the log.
///
/// [`LogWriter::record`] puts an input into a buffer in memory.
/// [`LogWriter::commit`] hands the buffer to the operating system, and to
/// the disk when the sync policy says so. The runtime applies the inputs
/// only after `commit` returned, so the world never has an input that the
/// log lacks.
pub(crate) struct LogWriter {
    storage: Arc<dyn Storage>,
    shared: Arc<LogShared>,
    segment: Arc<dyn Segment>,
    policy: SyncPolicy,
    segment_bytes: u64,
    max_record: u32,
    /// The newest segment.
    id: SegmentId,
    /// Bytes in the newest segment.
    size: u64,
    /// Framed records that the operating system does not have yet.
    buffer: Vec<u8>,
    /// Number of records in `buffer`.
    buffered: u64,
    /// Sequence number of the next input.
    next_seq: u64,
    /// The newest segment can end in bytes that are not a record. The next
    /// write starts a new segment.
    broken: bool,
    /// Inputs that were applied can be missing from the disk. A snapshot
    /// makes them unnecessary.
    snapshot_wanted: bool,
    /// Writes and syncs that failed since the start.
    errors: u64,
}

impl LogWriter {
    /// Starts a new segment for inputs from `id.first_seq` on.
    pub(crate) fn open(
        storage: Arc<dyn Storage>,
        id: SegmentId,
        policy: SyncPolicy,
        segment_bytes: u64,
        max_record: u32,
    ) -> Result<LogWriter, StorageError> {
        let segment = create_segment(&*storage, id)?;
        Ok(LogWriter {
            shared: Arc::new(LogShared::new(Arc::clone(&segment), id.first_seq)),
            storage,
            segment,
            policy,
            segment_bytes,
            max_record,
            id,
            size: SEGMENT_HEADER as u64,
            buffer: Vec::new(),
            buffered: 0,
            next_seq: id.first_seq,
            broken: false,
            snapshot_wanted: false,
            errors: 0,
        })
    }

    /// Returns the sequence number of the next input.
    pub(crate) fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Returns the index of the newest segment.
    pub(crate) fn segment_index(&self) -> u64 {
        self.id.index
    }

    /// Returns the state that the sync threads share.
    pub(crate) fn shared(&self) -> &Arc<LogShared> {
        &self.shared
    }

    /// Returns the number of writes and syncs that failed.
    pub(crate) fn errors(&self) -> u64 {
        self.errors
    }

    /// Returns true once after a failure that can have lost inputs that
    /// were applied. The caller takes a snapshot.
    pub(crate) fn take_snapshot_wanted(&mut self) -> bool {
        std::mem::take(&mut self.snapshot_wanted)
    }

    /// Puts one input into the buffer. With the policy `EveryRecord` it
    /// also forces it to the disk.
    pub(crate) fn record(&mut self, input: &Input) -> Result<(), JournalError> {
        if let Err(message) =
            record::encode(self.next_seq, input, self.max_record, &mut self.buffer)
        {
            self.errors += 1;
            return Err(JournalError { message });
        }
        self.next_seq += 1;
        self.buffered += 1;
        match self.policy {
            SyncPolicy::EveryRecord => self.flush(true),
            SyncPolicy::EveryBatch | SyncPolicy::Interval(_) => Ok(()),
        }
    }

    /// Makes the inputs in the buffer as durable as the policy promises.
    /// When it fails, none of them is in the log and none must be applied.
    pub(crate) fn commit(&mut self) -> Result<(), JournalError> {
        match self.policy {
            SyncPolicy::EveryRecord | SyncPolicy::EveryBatch => self.flush(true),
            SyncPolicy::Interval(_) => self.flush(false),
        }
    }

    /// Forces everything to the disk now.
    pub(crate) fn sync_now(&mut self) -> Result<(), JournalError> {
        self.flush(true)
    }

    /// Forgets the buffer after a failure and returns the error for the
    /// caller. The inputs of the buffer get their sequence numbers back.
    fn fail(&mut self, error: &StorageError) -> JournalError {
        self.next_seq -= self.buffered;
        self.buffered = 0;
        self.buffer.clear();
        self.errors += 1;
        self.shared.lock().written = self.next_seq;
        JournalError { message: error.to_string() }
    }

    /// Handles a write that failed after bytes can have reached the
    /// segment. The inputs of the buffer are refused, but their records can
    /// be in the file, whole or in part. A new segment makes them void at
    /// once, so that a crash before the next input cannot bring a refused
    /// input back. When the new segment cannot be made either, the next
    /// write tries again.
    fn fail_and_void(&mut self, error: &StorageError) -> JournalError {
        let refused = self.fail(error);
        self.broken = true;
        if let Err(error) = self.roll(self.next_seq) {
            self.errors += 1;
            tracing::error!(
                %error,
                seq = self.next_seq,
                "records of refused inputs can still be in the log; a crash now would apply them"
            );
        }
        refused
    }

    /// Starts a new segment whose first record is `first_seq`. Every record
    /// from that number on in the older segments is void from now on.
    fn roll(&mut self, first_seq: u64) -> Result<(), StorageError> {
        // The old segment gets no more syncs after the roll. Records that
        // were applied must not depend on one.
        if let Err(error) = self.segment.sync() {
            self.errors += 1;
            self.snapshot_wanted = true;
            tracing::error!(%error, "a log segment could not be forced to the disk before the next one");
        }
        let id = SegmentId { index: self.id.index + 1, first_seq };
        let segment = create_segment(&*self.storage, id)?;
        {
            let mut state = self.shared.lock();
            state.segment = Arc::clone(&segment);
            state.generation += 1;
            state.written = first_seq;
            state.synced = first_seq;
            state.sync_failed = false;
        }
        self.segment = segment;
        self.id = id;
        self.size = SEGMENT_HEADER as u64;
        self.broken = false;
        Ok(())
    }

    /// Hands the buffer to the operating system, and to the disk when
    /// `sync` is true.
    fn flush(&mut self, sync: bool) -> Result<(), JournalError> {
        if std::mem::take(&mut self.shared.lock().sync_failed) {
            // A sync of the background thread failed. The operating system
            // can have dropped what it could not write, so the segment is
            // not to be trusted again.
            self.errors += 1;
            self.snapshot_wanted = true;
            self.broken = true;
        }
        if self.buffered > 0 {
            let first = self.next_seq - self.buffered;
            if (self.broken || self.size >= self.segment_bytes)
                && let Err(error) = self.roll(first)
            {
                return Err(self.fail(&error));
            }
            if let Err(error) = self.segment.append(&self.buffer) {
                return Err(self.fail_and_void(&error));
            }
            self.shared.lock().written = self.next_seq;
        }
        if sync && let Err(error) = self.shared.sync() {
            return Err(self.fail_and_void(&error));
        }
        self.size += self.buffer.len() as u64;
        self.buffer.clear();
        self.buffered = 0;
        Ok(())
    }

    /// Takes the last `unapplied` inputs out of the log. The runtime calls
    /// it when the simulation panicked before it applied them: a new
    /// segment makes their records void, so a recovery does not meet the
    /// input that caused the panic.
    pub(crate) fn discard(&mut self, unapplied: u64) {
        self.next_seq -= self.buffered;
        self.buffered = 0;
        self.buffer.clear();
        if unapplied == 0 {
            return;
        }
        let first = self.next_seq.saturating_sub(unapplied);
        match self.roll(first) {
            Ok(()) => self.next_seq = first,
            Err(error) => {
                self.errors += 1;
                tracing::error!(
                    %error,
                    seq = first,
                    "an input that the simulation did not apply stays in the log; a recovery will apply it"
                );
            }
        }
    }

    /// Starts the log again at `seq`: a new segment makes every later
    /// record void. This is the step that makes a rollback permanent.
    pub(crate) fn restart_at(&mut self, seq: u64) -> Result<(), JournalError> {
        self.next_seq -= self.buffered;
        self.buffered = 0;
        self.buffer.clear();
        match self.roll(seq) {
            Ok(()) => {
                self.next_seq = seq;
                Ok(())
            }
            Err(error) => {
                self.errors += 1;
                Err(JournalError { message: error.to_string() })
            }
        }
    }
}
