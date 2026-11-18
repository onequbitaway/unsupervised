//! The handle: how every other part of the server reaches the world.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, oneshot};
use unsup_kernel::Message;
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, Catalogue, FrameOutput, Observation,
    ObserveOpts, ResidentId, SimError, SpawnSpec, ViewQuery, WaitFilter, WorldView,
};

use crate::external;
use crate::journal::RollbackError;
use crate::mailbox::{Command, Counters};
use crate::stats::SimStats;

/// What a rollback of the whole world did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RollbackReport {
    /// Frame of the snapshot that the world went back to.
    pub frame: u64,
    /// Inputs after that snapshot. They are no longer in the log.
    pub dropped_inputs: u64,
}

/// The door to a running world. Clone it freely; every clone talks to the
/// same simulation thread.
///
/// Every call puts a message in the mailbox of the simulation thread and
/// waits for the answer. A call fails with `SimError::Overloaded` when the
/// mailbox is full and with `SimError::Shutdown` after the simulation
/// stopped.
///
/// `Overloaded` is the back-pressure of the runtime: the call returns at
/// once and nothing was changed. The caller tries again later or passes the
/// error on to its client (HTTP 503 or 429).
#[derive(Clone)]
pub struct SimHandle {
    pub(crate) tx: SyncSender<Command>,
    /// The simulation thread owns the sender. The handle holds a weak
    /// reference, so the stream of frames ends when the thread stops.
    pub(crate) frames: broadcast::WeakSender<Arc<FrameOutput>>,
    pub(crate) catalogue: Arc<Catalogue>,
    pub(crate) counters: Arc<Counters>,
}

impl fmt::Debug for SimHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimHandle").finish_non_exhaustive()
    }
}

impl SimHandle {
    /// Puts a call into the mailbox. It never blocks.
    fn send(&self, command: Command) -> Result<(), SimError> {
        // Count first: the simulation thread can take the call at once.
        self.counters.depth.fetch_add(1, Ordering::Relaxed);
        let error = match self.tx.try_send(command) {
            Ok(()) => return Ok(()),
            Err(TrySendError::Full(_)) => {
                self.counters.mailbox_full.fetch_add(1, Ordering::Relaxed);
                SimError::Overloaded
            }
            Err(TrySendError::Disconnected(_)) => SimError::Shutdown,
        };
        self.counters.depth.fetch_sub(1, Ordering::Relaxed);
        Err(error)
    }

    /// Sends a call to the simulation thread and waits for its answer.
    async fn call<T>(
        &self,
        make: impl FnOnce(oneshot::Sender<Result<T, SimError>>) -> Command,
    ) -> Result<T, SimError> {
        let (reply, answer) = oneshot::channel();
        self.send(make(reply))?;
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
    ///
    /// The call fails with `SimError::Overloaded` when
    /// [`RuntimeConfig::max_waiters`](crate::RuntimeConfig::max_waiters)
    /// calls wait already. A resident can have
    /// [`RuntimeConfig::max_waiters_per_resident`](crate::RuntimeConfig::max_waiters_per_resident)
    /// waiting calls; one more ends the oldest one early. A rollback ends
    /// every waiting call early.
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
    ///
    /// A cursor that is ahead of the inbox counts as "now". That happens
    /// after a rollback; the `next_notice` of the answer is the cursor to
    /// go on with.
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
    ///
    /// `AdminCommand::RollbackZone` is always refused with
    /// `AdminReply::Error`: one zone cannot go back in time without the
    /// rest of the world. Use [`SimHandle::rollback`].
    pub async fn admin(&self, cmd: AdminCommand) -> Result<AdminReply, SimError> {
        self.call(|reply| Command::Admin { command: cmd, reply }).await
    }

    /// Sends a typed message from a service outside the simulation to the
    /// global realm.
    ///
    /// The handlers that a module registered with `Builder::on_message::<M>`
    /// in the global scope get the message at the start of the next frame,
    /// with `RealmId::Global` as the sender. The message is an input of the
    /// world (`Input::Deliver`): it is in the input log before the world
    /// has it, and a recovery or a replay delivers it again in the same
    /// place. `Ok` means that the world has the message, not that a handler
    /// ran; a module answers through an event or a notice.
    ///
    /// Use the message type of the module itself. A handler panics on a
    /// payload that it cannot read, so a second type with the same `KIND`
    /// and other fields would stop the simulation. Give a message that only
    /// comes from outside a `KIND` of its own: then the handler knows that
    /// no realm sent it.
    ///
    /// # Errors
    ///
    /// `SimError::Invalid` when the message cannot be written as
    /// MessagePack and read back. `SimError::NotHosted` when another
    /// process hosts the global realm. `SimError::Overloaded` when the
    /// mailbox is full or the journal cannot record the message. In each
    /// case the world did not get the message.
    pub async fn send_global<M: Message>(&self, message: &M) -> Result<(), SimError> {
        let envelope = match external::envelope_for_global(message) {
            Ok(envelope) => envelope,
            Err(reason) => {
                tracing::warn!(kind = M::KIND, %reason, "a message from outside was refused");
                return Err(SimError::Invalid);
            }
        };
        self.call(|reply| Command::Deliver { messages: vec![envelope], reply }).await
    }

    /// Rolls the whole world back to the newest snapshot at or before
    /// `frame`, and drops every input after that snapshot from the log.
    ///
    /// The rollback is permanent: a later recovery gives the world of the
    /// snapshot plus what happened after the rollback. Every waiting call
    /// is answered early. Viewers see the frame number go back; they load
    /// the world again.
    ///
    /// # Errors
    ///
    /// The world is unchanged when the call fails: the runtime has no
    /// journal with snapshots ([`RollbackError::Unsupported`]), no snapshot
    /// is old enough ([`RollbackError::NoSnapshot`]), the storage failed
    /// ([`RollbackError::Failed`]) or the call did not reach the simulation
    /// ([`RollbackError::Sim`]).
    pub async fn rollback(&self, frame: u64) -> Result<RollbackReport, RollbackError> {
        let (reply, answer) = oneshot::channel();
        self.send(Command::Rollback { frame, reply }).map_err(RollbackError::Sim)?;
        answer.await.unwrap_or(Err(RollbackError::Sim(SimError::Shutdown)))
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
