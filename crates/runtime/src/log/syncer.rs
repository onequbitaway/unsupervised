//! What the log writer shares with the threads that force the log to the
//! disk, and the thread for the `Interval` sync policy.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::storage::{Segment, StorageError};

/// The state of the newest segment.
pub(crate) struct LogState {
    /// The segment that takes new records.
    pub segment: Arc<dyn Segment>,
    /// Changes whenever `segment` changes, so that a sync that was started
    /// on an older segment does not count for the new one.
    pub generation: u64,
    /// Sequence number after the last record that the operating system has.
    pub written: u64,
    /// Sequence number after the last record that is forced to the disk.
    pub synced: u64,
    /// A sync failed since the writer last looked. Records that were
    /// applied can be missing from the disk.
    pub sync_failed: bool,
}

/// The state of the log that several threads use: the simulation thread
/// appends, the sync thread and the snapshot thread force to the disk.
pub(crate) struct LogShared {
    state: Mutex<LogState>,
    /// Failed syncs since the start.
    pub sync_errors: AtomicU64,
}

impl LogShared {
    /// Makes the shared state for a first segment that holds no record yet.
    pub(crate) fn new(segment: Arc<dyn Segment>, next_seq: u64) -> LogShared {
        LogShared {
            state: Mutex::new(LogState {
                segment,
                generation: 0,
                written: next_seq,
                synced: next_seq,
                sync_failed: false,
            }),
            sync_errors: AtomicU64::new(0),
        }
    }

    /// The state is a few numbers and a handle. A thread that panicked
    /// while it held the lock cannot have left them half-written in a way
    /// that matters: every field is set in one step.
    pub(crate) fn lock(&self) -> MutexGuard<'_, LogState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Forces everything that was written so far to the disk. The lock is
    /// not held during the slow part, so the simulation thread can go on
    /// appending.
    pub(crate) fn sync(&self) -> Result<(), StorageError> {
        let (segment, generation, written) = {
            let state = self.lock();
            if state.synced >= state.written {
                return Ok(());
            }
            (Arc::clone(&state.segment), state.generation, state.written)
        };
        let result = segment.sync();
        let mut state = self.lock();
        match &result {
            Ok(()) if state.generation == generation => state.synced = state.synced.max(written),
            Ok(()) => {}
            Err(error) => {
                state.sync_failed = true;
                self.sync_errors.fetch_add(1, Ordering::Relaxed);
                tracing::error!(%error, "the input log could not be forced to the disk");
            }
        }
        result
    }

    /// Returns how many records the operating system has that are not
    /// forced to the disk.
    pub(crate) fn lag(&self) -> u64 {
        let state = self.lock();
        state.written.saturating_sub(state.synced)
    }
}

/// The thread of the `Interval` sync policy. It forces the log to the disk
/// every interval while there is something new.
pub(crate) struct Syncer {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<JoinHandle<()>>,
}

impl Syncer {
    /// Starts the thread.
    pub(crate) fn start(shared: Arc<LogShared>, interval: Duration) -> std::io::Result<Syncer> {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = Arc::clone(&stop);
        let thread =
            std::thread::Builder::new().name("unsup-log-sync".into()).spawn(move || {
                let (flag, wake) = &*signal;
                let mut stopped = flag.lock().unwrap_or_else(PoisonError::into_inner);
                while !*stopped {
                    let (guard, _) = wake
                        .wait_timeout(stopped, interval)
                        .unwrap_or_else(PoisonError::into_inner);
                    stopped = guard;
                    // An error is counted and logged in `sync`, and the writer
                    // sees `sync_failed` at its next write.
                    let _ = shared.sync();
                }
            })?;
        Ok(Syncer { stop, thread: Some(thread) })
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        let (flag, wake) = &*self.stop;
        *flag.lock().unwrap_or_else(PoisonError::into_inner) = true;
        wake.notify_all();
        if let Some(thread) = self.thread.take() {
            // The thread has no result. A panic in it was already printed.
            let _ = thread.join();
        }
    }
}
