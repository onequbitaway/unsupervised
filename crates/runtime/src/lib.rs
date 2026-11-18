//! The real-time driver of an Unsupervised world.
//!
//! [`start`] moves a [`World`] to one dedicated OS thread and returns a
//! [`SimHandle`]. The thread owns the world and reads a mailbox; nothing
//! else touches simulation state. The gateway and the spectator service
//! talk to the world only through the handle.
//!
//! The simulation thread:
//!
//! - takes inputs from the mailbox in small batches and applies them through
//!   [`World::apply`];
//! - steps one frame per `tick_interval`, or only on [`SimHandle::step`]
//!   when the interval is zero;
//! - gives every input to the [`Journal`] before it applies it;
//! - asks the [`Pacer`] which zones skip the next frame;
//! - sends every frame to the subscribers of [`SimHandle::frames`];
//! - wakes [`SimHandle::wait`] calls when their resident gets a notice or
//!   finishes an action.
//!
//! # Extension points
//!
//! The input log, snapshots, replay and time dilation are not implemented
//! here. They plug in without a change to the simulation loop:
//!
//! - [`Journal`] sees every input before it is applied
//!   ([`Journal::record`]) and the world after every frame
//!   ([`Journal::frame_done`]). An append-only input log and periodic
//!   snapshots are one implementation of it. Replay is a loop that feeds a
//!   log to `World::apply`; it needs no runtime thread.
//! - [`Pacer`] decides the `skip` list of every frame from the measured
//!   zone tick times. The decision is part of the `Input::Step` that the
//!   journal records, so a replay makes the same decision.

mod journal;
mod sim;

use std::fmt;
use std::sync::Arc;
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use unsup_kernel::World;
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, Catalogue, FrameOutput, Observation,
    ObserveOpts, ResidentId, SimError, SpawnSpec, ViewQuery, WaitFilter, WorldView,
};

pub use crate::journal::{Journal, JournalError, NoDilation, NoJournal, Pacer};
use crate::sim::Command;

/// How the runtime drives the world.
pub struct RuntimeConfig {
    /// Real time between two frames. Zero means manual stepping: frames
    /// advance only through [`SimHandle::step`]. The default is one second.
    pub tick_interval: Duration,
    /// Greatest number of calls that wait in the mailbox of the simulation
    /// thread. A call that finds the mailbox full fails with
    /// `SimError::Overloaded`.
    pub mailbox: usize,
    /// Greatest number of calls that the simulation thread handles in a row
    /// before it checks whether a frame is due.
    pub batch: usize,
    /// Number of frames that a slow subscriber of [`SimHandle::frames`] can
    /// fall behind before it loses frames.
    pub frame_buffer: usize,
    /// Sees every input before it is applied and the world after every
    /// frame. See the [crate documentation](crate#extension-points).
    pub journal: Box<dyn Journal>,
    /// Decides which zones skip a frame.
    pub pacer: Box<dyn Pacer>,
}

impl Default for RuntimeConfig {
    fn default() -> RuntimeConfig {
        RuntimeConfig {
            tick_interval: Duration::from_secs(1),
            mailbox: 16_384,
            batch: 256,
            frame_buffer: 64,
            journal: Box::new(NoJournal),
            pacer: Box::new(NoDilation),
        }
    }
}

impl RuntimeConfig {
    /// Makes a configuration for tests and tools: frames advance only
    /// through [`SimHandle::step`].
    #[must_use]
    pub fn manual() -> RuntimeConfig {
        RuntimeConfig { tick_interval: Duration::ZERO, ..RuntimeConfig::default() }
    }

    /// Makes a configuration that steps one frame per `tick_interval`.
    #[must_use]
    pub fn real_time(tick_interval: Duration) -> RuntimeConfig {
        RuntimeConfig { tick_interval, ..RuntimeConfig::default() }
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("tick_interval", &self.tick_interval)
            .field("mailbox", &self.mailbox)
            .field("batch", &self.batch)
            .field("frame_buffer", &self.frame_buffer)
            .finish_non_exhaustive()
    }
}

/// Numbers about the running simulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimStats {
    /// Number of the last frame.
    pub frame: u64,
    /// Residents in the zones that this process hosts.
    pub residents: u64,
    /// Inputs applied since the start, frames included.
    pub inputs_applied: u64,
    /// Frames that this runtime ran since the start.
    pub frames_run: u64,
    /// Mean time of a frame in microseconds, since the start.
    pub mean_frame_micros: u64,
    /// Longest time of a frame in microseconds, since the start.
    pub worst_frame_micros: u64,
    /// Time of the last frame in microseconds.
    pub last_frame_micros: u64,
    /// Calls to [`SimHandle::wait`] that wait now.
    pub waiting: u64,
    /// Inputs that were refused because the journal could not record them.
    pub journal_errors: u64,
}

/// The door to a running world. Clone it freely; every clone talks to the
/// same simulation thread.
///
/// Every call puts a message in the mailbox of the simulation thread and
/// waits for the answer. A call fails with `SimError::Overloaded` when the
/// mailbox is full and with `SimError::Shutdown` after the simulation
/// stopped.
#[derive(Clone)]
pub struct SimHandle {
    tx: SyncSender<Command>,
    /// The simulation thread owns the sender. The handle holds a weak
    /// reference, so the stream of frames ends when the thread stops.
    frames: broadcast::WeakSender<Arc<FrameOutput>>,
    catalogue: Arc<Catalogue>,
}

impl fmt::Debug for SimHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimHandle").finish_non_exhaustive()
    }
}

impl SimHandle {
    /// Sends a call to the simulation thread and waits for its answer.
    async fn call<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, SimError>>) -> Command,
    ) -> Result<T, SimError> {
        let (reply, answer) = oneshot::channel();
        match self.tx.try_send(make(reply)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => return Err(SimError::Overloaded),
            Err(TrySendError::Disconnected(_)) => return Err(SimError::Shutdown),
        }
        // The simulation thread drops the reply sender when it stops.
        answer.await.unwrap_or(Err(SimError::Shutdown))
    }

    /// Creates a resident.
    pub async fn spawn(&self, spec: SpawnSpec) -> Result<ResidentId, SimError> {
        self.call(|reply| Command::Spawn { spec, reply }).await
    }

    /// Sends one action request of a resident.
    pub async fn act(&self, r: ResidentId, req: ActionRequest) -> Result<ActionAck, SimError> {
        self.call(|reply| Command::Act { resident: r, request: req, reply }).await
    }

    /// Builds what a resident can see now.
    pub async fn observe(&self, r: ResidentId, opts: ObserveOpts) -> Result<Observation, SimError> {
        self.call(|reply| Command::Observe { resident: r, opts, reply }).await
    }

    /// Waits until a resident gets news that the filter lets through, or
    /// until `timeout` has passed, and returns an observation. Only news
    /// that arrives after the call counts.
    ///
    /// The observation holds the notices and reports since the call. After a
    /// timeout it holds none.
    pub async fn wait(
        &self,
        r: ResidentId,
        filter: WaitFilter,
        timeout: Duration,
    ) -> Result<Observation, SimError> {
        self.wait_since(r, filter, None, timeout).await
    }

    /// Like [`SimHandle::wait`], with a notice cursor: news with a sequence
    /// number at or above `since` counts, also when it arrived before the
    /// call. A client that passes the `next_notice` of its last observation
    /// cannot miss news between two calls.
    pub async fn wait_since(
        &self,
        r: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
    ) -> Result<Observation, SimError> {
        self.call(|reply| Command::Wait { resident: r, filter, since, timeout, reply }).await
    }

    /// Answers a question of a viewer.
    pub async fn view(&self, q: ViewQuery) -> Result<WorldView, SimError> {
        self.call(|reply| Command::View { query: q, reply }).await
    }

    /// Runs a command of an administrator.
    pub async fn admin(&self, cmd: AdminCommand) -> Result<AdminReply, SimError> {
        self.call(|reply| Command::Admin { command: cmd, reply }).await
    }

    /// Returns numbers about the running simulation.
    pub async fn stats(&self) -> Result<SimStats, SimError> {
        self.call(|reply| Command::Stats { reply }).await
    }

    /// Runs `frames` frames now and returns the number of the last frame.
    /// Tests and tools use it; with a `tick_interval` of zero it is the only
    /// way to advance the world.
    pub async fn step(&self, frames: u32) -> Result<u64, SimError> {
        self.call(|reply| Command::Step { frames, reply }).await
    }

    /// Subscribes to the frames of the world. A subscriber gets every frame
    /// from now on; one that falls more than `frame_buffer` frames behind
    /// loses the oldest. The stream ends when the simulation stops; a
    /// subscription that is made after the stop is closed from the start.
    #[must_use]
    pub fn frames(&self) -> broadcast::Receiver<Arc<FrameOutput>> {
        match self.frames.upgrade() {
            Some(sender) => sender.subscribe(),
            // The simulation stopped. A receiver whose sender is gone
            // reports `Closed` at once.
            None => broadcast::channel(1).1,
        }
    }

    /// Returns the public catalogue: verbs with argument schemas, items,
    /// attributes, conditions, and the parameter values at the start of the
    /// runtime.
    #[must_use]
    pub fn catalogue(&self) -> Arc<Catalogue> {
        Arc::clone(&self.catalogue)
    }
}

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
/// Dropping the value stops the thread too.
#[must_use = "dropping the RuntimeJoin stops the simulation"]
pub struct RuntimeJoin {
    tx: SyncSender<Command>,
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
        let _ = self.tx.send(Command::Shutdown);
        Some(thread.join().map_err(|panic| {
            let message = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| (*s).to_string()))
                .unwrap_or_else(|| "no message".to_string());
            RuntimePanic { message }
        }))
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
    let thread = std::thread::Builder::new()
        .name("unsup-sim".into())
        .spawn(move || sim::run(world, config, &rx, &frames))
        .expect("the operating system starts the simulation thread");
    (
        SimHandle { tx: tx.clone(), frames: weak_frames, catalogue },
        RuntimeJoin { tx, thread: Some(thread) },
    )
}
