//! What goes through the mailbox of the simulation thread: the calls of the
//! handles and the answers to them.

use std::sync::atomic::{AtomicU64, AtomicUsize};
use std::time::Duration;

use tokio::sync::oneshot;
use unsup_kernel::{Envelope, Output};
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, Observation, ObserveOpts, ResidentId,
    SimError, SpawnSpec, ViewQuery, WaitFilter, WorldView,
};

use crate::journal::RollbackError;
use crate::{RollbackReport, SimStats};

/// Where the answer to a call goes.
pub(crate) type Reply<T> = oneshot::Sender<Result<T, SimError>>;

/// One message in the mailbox of the simulation thread.
pub(crate) enum Command {
    Spawn {
        spec: SpawnSpec,
        reply: Reply<ResidentId>,
    },
    Act {
        resident: ResidentId,
        request: ActionRequest,
        reply: Reply<ActionAck>,
    },
    Observe {
        resident: ResidentId,
        opts: ObserveOpts,
        reply: Reply<Observation>,
    },
    Wait {
        resident: ResidentId,
        filter: WaitFilter,
        since: Option<u64>,
        timeout: Duration,
        reply: Reply<Observation>,
    },
    View {
        query: ViewQuery,
        reply: Reply<WorldView>,
    },
    Admin {
        command: AdminCommand,
        reply: Reply<AdminReply>,
    },
    /// Messages from a service outside the simulation.
    Deliver {
        messages: Vec<Envelope>,
        reply: Reply<()>,
    },
    Stats {
        reply: Reply<SimStats>,
    },
    Step {
        frames: u32,
        reply: Reply<u64>,
    },
    Rollback {
        frame: u64,
        reply: oneshot::Sender<Result<RollbackReport, RollbackError>>,
    },
    Shutdown,
}

/// What the handles and the simulation thread both count.
#[derive(Default)]
pub(crate) struct Counters {
    /// Calls in the mailbox now.
    pub depth: AtomicUsize,
    /// Calls that found the mailbox full.
    pub mailbox_full: AtomicU64,
}

/// Where the answer to an input goes.
pub(crate) enum InputReply {
    Spawn(Reply<ResidentId>),
    Act(Reply<ActionAck>),
    Admin(Reply<AdminReply>),
    Deliver(Reply<()>),
}

impl InputReply {
    /// Returns the answer for an input that was not applied.
    pub(crate) fn refuse(self, error: SimError) -> Answer {
        match self {
            InputReply::Spawn(reply) => Answer::Spawn(reply, Err(error)),
            InputReply::Act(reply) => Answer::Act(reply, Err(error)),
            InputReply::Admin(reply) => Answer::Admin(reply, Err(error)),
            InputReply::Deliver(reply) => Answer::Delivered(reply, Err(error)),
        }
    }

    /// Returns the answer for an input that the world took.
    pub(crate) fn answer(self, output: Output) -> Answer {
        match (self, output) {
            (InputReply::Spawn(reply), Output::Spawned(id)) => Answer::Spawn(reply, Ok(id)),
            (InputReply::Act(reply), Output::Ack(ack)) => Answer::Act(reply, Ok(ack)),
            (InputReply::Admin(reply), Output::Admin(answer)) => Answer::Admin(reply, Ok(answer)),
            (InputReply::Deliver(reply), Output::Delivered) => Answer::Delivered(reply, Ok(())),
            (reply, Output::Error(error)) => reply.refuse(error),
            (reply, _) => reply.refuse(SimError::Invalid),
        }
    }
}

/// An answer that is ready. The answers of a batch are sent together when
/// the batch is done, so that the world is not interleaved with wake-ups
/// of other threads.
pub(crate) enum Answer {
    Spawn(Reply<ResidentId>, Result<ResidentId, SimError>),
    Act(Reply<ActionAck>, Result<ActionAck, SimError>),
    Admin(Reply<AdminReply>, Result<AdminReply, SimError>),
    Delivered(Reply<()>, Result<(), SimError>),
    Observation(Reply<Observation>, Result<Observation, SimError>),
    View(Reply<WorldView>, Result<WorldView, SimError>),
    Stats(Reply<SimStats>, Result<SimStats, SimError>),
}

impl Answer {
    /// Gives the answer to the caller.
    pub(crate) fn send(self) {
        // A reply fails only when the caller went away. That is fine.
        match self {
            Answer::Spawn(reply, answer) => drop(reply.send(answer)),
            Answer::Act(reply, answer) => drop(reply.send(answer)),
            Answer::Admin(reply, answer) => drop(reply.send(answer)),
            Answer::Delivered(reply, answer) => drop(reply.send(answer)),
            Answer::Observation(reply, answer) => drop(reply.send(answer)),
            Answer::View(reply, answer) => drop(reply.send(answer)),
            Answer::Stats(reply, answer) => drop(reply.send(answer)),
        }
    }
}
