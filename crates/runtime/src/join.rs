//! Starting and stopping the simulation thread.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::thread::JoinHandle;

use tokio::sync::broadcast;
use unsup_kernel::World;

use crate::config::RuntimeConfig;
use crate::handle::SimHandle;
use crate::mailbox::{Command, Counters};
use crate::replay::panic_message;
use crate::sim;

/// The simulation thread stopped because a plugin or the kernel panicked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimePanic {
    /// The panic message, when it was text.
    pub message: String,
}

impl fmt::Display for RuntimePanic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the simulation thread panicked: {}", self.message)
    }
}

impl std::error::Error for RuntimePanic {}

/// The owner of the simulation thread.
///
/// [`RuntimeJoin::shutdown`] stops the thread and gives the world back.
/// Dropping the value stops the thread too. Both are a clean stop: the
/// journal writes its final snapshot.
#[must_use = "dropping the RuntimeJoin stops the simulation"]
pub struct RuntimeJoin {
    tx: SyncSender<Command>,
    counters: Arc<Counters>,
    thread: Option<JoinHandle<World>>,
}

impl fmt::Debug for RuntimeJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeJoin").field("running", &self.thread.is_some()).finish()
    }
}

impl RuntimeJoin {
    /// Stops the simulation after the inputs that are already in the mailbox
    /// and returns the world. Calls that wait get `SimError::Shutdown`.
    pub fn shutdown(mut self) -> Result<World, RuntimePanic> {
        self.stop().unwrap_or_else(|| {
            Err(RuntimePanic { message: "the thread was already joined".into() })
        })
    }

    /// Returns true when the simulation thread has ended, for example after
    /// a panic.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn stop(&mut self) -> Option<Result<World, RuntimePanic>> {
        let thread = self.thread.take()?;
        // A blocking send: the mailbox may be full, and the stop request
        // must not be lost. It fails only when the thread is gone already.
        self.counters.depth.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Command::Shutdown);
        Some(thread.join().map_err(|panic| RuntimePanic { message: panic_message(&*panic) }))
    }
}

impl Drop for RuntimeJoin {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

/// Starts the simulation thread for a world.
///
/// The thread owns the world from now on. The handle is the only way to
/// reach it; the join value stops the thread and gives the world back.
///
/// # Panics
///
/// Panics when the operating system cannot start a thread.
pub fn start(world: World, config: RuntimeConfig) -> (SimHandle, RuntimeJoin) {
    let (tx, rx) = sync_channel(config.mailbox.max(1));
    let (frames, _) = broadcast::channel(config.frame_buffer.max(1));
    let catalogue = Arc::new(world.catalogue());
    let weak_frames = frames.downgrade();
    let counters = Arc::new(Counters::default());
    let shared = Arc::clone(&counters);
    let thread = std::thread::Builder::new()
        .name("unsup-sim".into())
        .spawn(move || sim::run(world, config, &rx, &frames, &shared))
        .expect("the operating system starts the simulation thread");
    (
        SimHandle {
            tx: tx.clone(),
            frames: weak_frames,
            catalogue,
            counters: Arc::clone(&counters),
        },
        RuntimeJoin { tx, counters, thread: Some(thread) },
    )
}
