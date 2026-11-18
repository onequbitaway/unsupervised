//! The connection from the coordinator to one worker.
//!
//! A thread reads the frames of the worker and puts them in a queue. The
//! coordinator takes them from the queue with a time limit. So the
//! coordinator never hangs on a worker that is silent, and it learns of a
//! dead worker also while it has nothing to ask.

use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TryRecvError, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use super::error::ClusterError;
use super::proto::{FromWorker, ToWorker};
use super::wire::{WireError, read_frame, write_frame};
use crate::mailbox::{Command, Counters};

/// The mailbox of the coordinator and its counters.
type Mailbox = (SyncSender<Command>, Arc<Counters>);

/// Where a reader thread tells the coordinator that a connection ended.
/// It is empty until the coordinator serves its mailbox.
#[derive(Clone, Default)]
pub(crate) struct Alarm {
    target: Arc<Mutex<Option<Mailbox>>>,
}

impl Alarm {
    /// Connects the alarm to the mailbox of the coordinator.
    pub(crate) fn arm(&self, mailbox: SyncSender<Command>, counters: Arc<Counters>) {
        *self.target.lock().unwrap_or_else(PoisonError::into_inner) = Some((mailbox, counters));
    }

    /// Takes the alarm off the mailbox, so that the mailbox can close.
    pub(crate) fn disarm(&self) {
        *self.target.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Wakes the coordinator. A full mailbox is fine: the coordinator is
    /// busy then and finds the dead connection at its next question.
    fn ring(&self) {
        let target = self.target.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((mailbox, counters)) = &*target {
            counters.depth.fetch_add(1, Ordering::Relaxed);
            if mailbox.try_send(Command::Cluster(super::call::ClusterCall::Check)).is_err() {
                counters.depth.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

/// The connection to one worker.
pub(crate) struct Link {
    /// Slot of the worker. `u32::MAX` before the handshake named it.
    pub slot: u32,
    stream: TcpStream,
    queue: Receiver<Result<FromWorker, WireError>>,
    reader: Option<JoinHandle<()>>,
    max_frame: u32,
    /// Bytes sent to the worker.
    pub sent: u64,
    /// Bytes received from the worker.
    pub received: Arc<AtomicU64>,
}

impl Link {
    /// Takes a connection that a worker opened and starts its reader
    /// thread.
    pub(crate) fn start(stream: TcpStream, max_frame: u32, alarm: Alarm) -> std::io::Result<Link> {
        // Frames are whole messages. Waiting for more data to fill a
        // packet only adds delay.
        stream.set_nodelay(true)?;
        stream.set_nonblocking(false)?;
        let mut input = stream.try_clone()?;
        let (tx, queue) = channel();
        let received = Arc::new(AtomicU64::new(0));
        let counter = Arc::clone(&received);
        let reader =
            std::thread::Builder::new().name("unsup-cluster-link".into()).spawn(move || {
                loop {
                    match read_frame::<_, FromWorker>(&mut input, max_frame) {
                        Ok((message, bytes)) => {
                            counter.fetch_add(bytes as u64, Ordering::Relaxed);
                            if tx.send(Ok(message)).is_err() {
                                return;
                            }
                        }
                        Err(error) => {
                            // The coordinator may be gone already.
                            let _ = tx.send(Err(error));
                            alarm.ring();
                            return;
                        }
                    }
                }
            })?;
        Ok(Link {
            slot: u32::MAX,
            stream,
            queue,
            reader: Some(reader),
            max_frame,
            sent: 0,
            received,
        })
    }

    /// Sets how long a write to the worker can take. A worker that does
    /// not read for this long counts as lost.
    pub(crate) fn limit_writes(&self, timeout: Duration) -> std::io::Result<()> {
        self.stream.set_write_timeout(Some(timeout).filter(|limit| !limit.is_zero()))
    }

    fn lost(&self, reason: impl ToString) -> ClusterError {
        ClusterError::WorkerLost { slot: self.slot, reason: reason.to_string() }
    }

    /// Sends one message.
    pub(crate) fn send(&mut self, message: &ToWorker) -> Result<(), ClusterError> {
        match write_frame(&mut self.stream, message, self.max_frame) {
            Ok(bytes) => {
                self.sent += bytes as u64;
                Ok(())
            }
            Err(error) => Err(self.lost(error)),
        }
    }

    /// Maps what the reader thread gave.
    fn taken(&self, item: Result<FromWorker, WireError>) -> Result<FromWorker, ClusterError> {
        match item {
            Ok(FromWorker::Failed { realm: Some(realm), message }) => {
                Err(ClusterError::Failed { realm, message })
            }
            Ok(FromWorker::Failed { realm: None, message }) => Err(self.lost(message)),
            Ok(message) => Ok(message),
            Err(error) => Err(self.lost(error)),
        }
    }

    /// Waits for the next message of the worker, at most `timeout`.
    pub(crate) fn recv(&mut self, timeout: Duration) -> Result<FromWorker, ClusterError> {
        match self.queue.recv_timeout(timeout) {
            Ok(item) => self.taken(item),
            Err(RecvTimeoutError::Timeout) => {
                Err(self.lost(format!("no answer in {} ms", timeout.as_millis())))
            }
            Err(RecvTimeoutError::Disconnected) => Err(self.lost(WireError::Closed)),
        }
    }

    /// Looks at the connection while the coordinator has nothing to ask.
    /// A worker speaks then only when it fails.
    pub(crate) fn check(&mut self) -> Result<(), ClusterError> {
        match self.queue.try_recv() {
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => Err(self.lost(WireError::Closed)),
            Ok(item) => {
                let message = self.taken(item)?;
                Err(ClusterError::Protocol(format!(
                    "worker {} sent `{}` when nothing was asked",
                    self.slot,
                    message.name()
                )))
            }
        }
    }

    /// Returns the bytes received from the worker.
    pub(crate) fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // The reader thread waits in a read. Closing the socket ends it.
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}
