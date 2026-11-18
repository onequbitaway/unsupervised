//! The thread that writes snapshot files, so that the simulation thread
//! does not wait for the disk.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;
use std::time::Instant;

use super::format::SnapshotMeta;
use super::store;
use crate::log::LogShared;
use crate::storage::Storage;

/// One snapshot to write.
pub(crate) struct SnapshotJob {
    /// Frame, sequence number, segment and state hash.
    pub meta: SnapshotMeta,
    /// What `World::snapshot` returned.
    pub payload: Vec<u8>,
}

/// Numbers that the writer thread publishes.
#[derive(Default)]
pub(crate) struct SnapshotShared {
    /// True while the thread writes a snapshot.
    pub busy: AtomicBool,
    /// Snapshots written since the start.
    pub written: AtomicU64,
    /// Snapshots that could not be written since the start.
    pub errors: AtomicU64,
    /// Frame of the last snapshot that was written.
    pub last_frame: AtomicU64,
    /// Sequence number of the last snapshot that was written, plus one. 0
    /// when none was written.
    pub last_seq_plus_one: AtomicU64,
    /// Microseconds that the last write took, with the sync of the log.
    pub last_write_micros: AtomicU64,
    /// Size of the last snapshot file in bytes.
    pub last_bytes: AtomicU64,
}

enum Message {
    /// Write a snapshot. The sender gets the result when it gave a channel.
    Write(SnapshotJob, Option<SyncSender<Result<(), String>>>),
    /// Answer when everything before this message is done.
    Barrier(SyncSender<()>),
}

/// The handle to the writer thread.
pub(crate) struct SnapshotWriter {
    tx: Option<SyncSender<Message>>,
    thread: Option<JoinHandle<()>>,
    shared: Arc<SnapshotShared>,
}

/// What one written snapshot adds to the published numbers.
struct Written {
    micros: u64,
    bytes: u64,
}

/// Writes one snapshot: the log first, then the file, then the rotation.
fn write_one(
    storage: &dyn Storage,
    log: &LogShared,
    keep: usize,
    job: &SnapshotJob,
) -> Result<Written, String> {
    let start = Instant::now();
    // A snapshot must never be newer than the log on the disk: a recovery
    // that falls back to an older snapshot needs every record up to here.
    log.sync().map_err(|e| e.to_string())?;
    let bytes = store::write(storage, &job.meta, &job.payload).map_err(|e| e.to_string())?;
    let micros = u64::try_from(start.elapsed().as_micros()).unwrap_or(u64::MAX);
    // The snapshot is safe. A rotation that fails leaves extra files; the
    // next rotation removes them.
    if let Err(error) = store::prune(storage, keep) {
        tracing::warn!(%error, "old snapshots and log segments could not be removed");
    }
    Ok(Written { micros, bytes })
}

fn run(
    storage: &dyn Storage,
    log: &LogShared,
    keep: usize,
    shared: &SnapshotShared,
    jobs: &Receiver<Message>,
) {
    while let Ok(message) = jobs.recv() {
        match message {
            Message::Write(job, reply) => {
                shared.busy.store(true, Ordering::Release);
                let result = write_one(storage, log, keep, &job);
                // The thread is free before it publishes the numbers: who
                // sees the new numbers can hand over the next snapshot.
                shared.busy.store(false, Ordering::Release);
                match &result {
                    Ok(written) => {
                        shared.last_frame.store(job.meta.frame, Ordering::Relaxed);
                        shared.last_seq_plus_one.store(job.meta.seq + 1, Ordering::Relaxed);
                        shared.last_write_micros.store(written.micros, Ordering::Relaxed);
                        shared.last_bytes.store(written.bytes, Ordering::Relaxed);
                        shared.written.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error) => {
                        tracing::error!(%error, frame = job.meta.frame, "a snapshot could not be written");
                        shared.errors.fetch_add(1, Ordering::Relaxed);
                    }
                }
                if let Some(reply) = reply {
                    // The caller may have gone away. That is fine.
                    let _ = reply.send(result.map(|_| ()));
                }
            }
            Message::Barrier(reply) => {
                let _ = reply.send(());
            }
        }
    }
}

impl SnapshotWriter {
    /// Starts the thread.
    pub(crate) fn start(
        storage: Arc<dyn Storage>,
        log: Arc<LogShared>,
        keep: usize,
    ) -> std::io::Result<SnapshotWriter> {
        // No buffer: the thread holds at most one snapshot, and a sender
        // sees at once whether the thread is free.
        let (tx, rx) = sync_channel(0);
        let shared = Arc::new(SnapshotShared::default());
        let published = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("unsup-snapshot".into())
            .spawn(move || run(&*storage, &log, keep, &published, &rx))?;
        Ok(SnapshotWriter { tx: Some(tx), thread: Some(thread), shared })
    }

    /// Returns the numbers that the thread publishes.
    pub(crate) fn shared(&self) -> &SnapshotShared {
        &self.shared
    }

    /// Returns true while the thread writes a snapshot.
    pub(crate) fn is_busy(&self) -> bool {
        self.shared.busy.load(Ordering::Acquire)
    }

    fn send(&self, message: Message) -> Result<(), String> {
        let gone = || "the snapshot thread has stopped".to_string();
        self.tx.as_ref().ok_or_else(gone)?.send(message).map_err(|_| gone())
    }

    /// Hands a snapshot to the thread and returns. The caller checked
    /// [`SnapshotWriter::is_busy`] first, so the hand-over is short.
    pub(crate) fn submit(&self, job: SnapshotJob) -> Result<(), String> {
        // The flag is set here too, so that a second `is_busy` right after
        // this call sees the work.
        self.shared.busy.store(true, Ordering::Release);
        self.send(Message::Write(job, None)).inspect_err(|_| {
            self.shared.busy.store(false, Ordering::Release);
        })
    }

    /// Hands a snapshot to the thread and waits until it is on the disk.
    pub(crate) fn write_now(&self, job: SnapshotJob) -> Result<(), String> {
        let (reply, done) = sync_channel(1);
        self.send(Message::Write(job, Some(reply)))?;
        done.recv().map_err(|_| "the snapshot thread has stopped".to_string())?
    }

    /// Waits until the thread has finished everything that it was given.
    pub(crate) fn wait_idle(&self) {
        let (reply, done) = sync_channel(1);
        if self.send(Message::Barrier(reply)).is_ok() {
            // An error means that the thread is gone: nothing is in flight.
            let _ = done.recv();
        }
    }
}

impl Drop for SnapshotWriter {
    fn drop(&mut self) {
        // Closing the channel ends the loop of the thread after the
        // snapshot that it writes now.
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            // The thread has no result. A panic in it was already printed.
            let _ = thread.join();
        }
    }
}
