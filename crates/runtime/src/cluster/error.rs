//! Why a cluster did not start or stopped.

use std::fmt;

use unsup_protocol::{RealmId, SimError};

/// A cluster did not start, or it stopped.
///
/// A cluster never goes on without one of its processes. Every failure
/// stops all of them, and each process gets one of these values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClusterError {
    /// The configuration of the cluster is not valid.
    Config(String),
    /// A socket could not be opened.
    Io(String),
    /// On a worker: the coordinator refused the handshake.
    Refused {
        /// Why. It names what differs.
        reason: String,
    },
    /// On the coordinator: not all workers joined in time.
    JoinTimeout {
        /// The worker slots that nobody took.
        missing: Vec<u32>,
    },
    /// On the coordinator: a worker died, closed its connection or did not
    /// answer in time.
    WorkerLost {
        /// The slot of the worker.
        slot: u32,
        /// What the coordinator saw.
        reason: String,
    },
    /// On a worker: the coordinator died or closed the connection.
    CoordinatorLost {
        /// What the worker saw.
        reason: String,
    },
    /// On a worker: the coordinator stopped the cluster after a failure.
    Stopped {
        /// The failure.
        reason: String,
    },
    /// A process sent a message that the frame protocol does not allow at
    /// that point.
    Protocol(String),
    /// The stored state of a realm cannot be used.
    Recovery {
        /// The realm, when one realm is the cause.
        realm: Option<RealmId>,
        /// What is wrong.
        message: String,
    },
    /// A realm stopped: the simulation panicked, or the journal could not
    /// record a frame or a message.
    Failed {
        /// The realm.
        realm: RealmId,
        /// The panic message or the journal error.
        message: String,
    },
    /// The totals of money or goods of all processes do not balance.
    Audit(String),
    /// A call did not reach the coordinator.
    Sim(SimError),
}

impl fmt::Display for ClusterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ClusterError::Config(message) => {
                write!(f, "the cluster configuration is not valid: {message}")
            }
            ClusterError::Io(message) => write!(f, "the cluster transport failed: {message}"),
            ClusterError::Refused { reason } => {
                write!(f, "the coordinator refused this worker: {reason}")
            }
            ClusterError::JoinTimeout { missing } => {
                write!(f, "the cluster did not start: no worker joined for the slots {missing:?}")
            }
            ClusterError::WorkerLost { slot, reason } => {
                write!(f, "the cluster stopped: worker {slot} is lost: {reason}")
            }
            ClusterError::CoordinatorLost { reason } => {
                write!(f, "the worker stopped: the coordinator is lost: {reason}")
            }
            ClusterError::Stopped { reason } => {
                write!(f, "the coordinator stopped the cluster: {reason}")
            }
            ClusterError::Protocol(message) => {
                write!(f, "the cluster stopped: the frame protocol was broken: {message}")
            }
            ClusterError::Recovery { realm: Some(realm), message } => {
                write!(f, "the stored state of {realm} cannot be used: {message}")
            }
            ClusterError::Recovery { realm: None, message } => {
                write!(f, "the cluster cannot recover: {message}")
            }
            ClusterError::Failed { realm, message } => {
                write!(f, "the cluster stopped: {realm} failed: {message}")
            }
            ClusterError::Audit(message) => {
                write!(f, "the totals of the cluster do not balance: {message}")
            }
            ClusterError::Sim(error) => write!(f, "the call did not reach the cluster: {error}"),
        }
    }
}

impl std::error::Error for ClusterError {}

impl From<SimError> for ClusterError {
    fn from(error: SimError) -> ClusterError {
        ClusterError::Sim(error)
    }
}
