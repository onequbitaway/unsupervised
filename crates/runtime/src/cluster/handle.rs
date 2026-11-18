//! The handle and the owner of a running cluster.

use std::fmt;
use std::ops::Deref;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;

use tokio::sync::oneshot;
use unsup_kernel::World;
use unsup_protocol::{RealmId, SimError};

use super::call::{ClusterCall, ClusterReply};
use super::error::ClusterError;
use super::stats::ClusterStats;
use super::totals::ClusterTotals;
use crate::handle::SimHandle;
use crate::mailbox::{Command, Counters};
use crate::replay::panic_message;

/// The door to a running cluster. Clone it freely.
///
/// A `ClusterHandle` is a [`SimHandle`]: it has the same methods (`spawn`,
/// `act`, `observe`, `wait`, `wait_since`, `view`, `admin`, `post_global`,
/// `stats`, `step`, `frames`, `catalogue`), and [`ClusterHandle::sim`]
/// gives a `SimHandle` for a service that takes one. The coordinator
/// sends each call to the process that hosts the resident or the realm.
/// A service does not see whether it talks to one process or to a cluster.
///
/// The differences are in `README.md`: `SimHandle::rollback` answers
/// `RollbackError::Unsupported`, and an observation has no section of the
/// global realm.
#[derive(Clone)]
pub struct ClusterHandle {
    pub(crate) sim: SimHandle,
}

impl fmt::Debug for ClusterHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterHandle").finish_non_exhaustive()
    }
}

impl Deref for ClusterHandle {
    type Target = SimHandle;

    fn deref(&self) -> &SimHandle {
        &self.sim
    }
}

impl ClusterHandle {
    /// Returns the handle as a [`SimHandle`], for a service that works
    /// with one process or with a cluster.
    #[must_use]
    pub fn sim(&self) -> SimHandle {
        self.sim.clone()
    }

    /// Sends a cluster call to the coordinator and waits for its answer.
    async fn call<T>(
        &self,
        make: impl FnOnce(ClusterReply<T>) -> ClusterCall,
    ) -> Result<T, SimError> {
        let (reply, answer) = oneshot::channel();
        self.sim.send(Command::Cluster(make(reply)))?;
        // The coordinator drops the reply sender when the cluster stops.
        answer.await.unwrap_or(Err(SimError::Shutdown))
    }

    /// Returns numbers about the cluster: the workers, the router and the
    /// phases of a frame.
    pub async fn cluster_stats(&self) -> Result<ClusterStats, SimError> {
        self.call(|reply| ClusterCall::Stats { reply }).await
    }

    /// Returns `World::state_hash` of the world of every realm: the global
    /// realm first, then the zones in zone order.
    ///
    /// Each world hosts one realm. Two clusters with the same seed and the
    /// same inputs give the same hashes, however the zones are split over
    /// the workers. The hash of such a world is not the hash of a world
    /// that hosts every realm.
    pub async fn hashes(&self) -> Result<Vec<(RealmId, [u8; 32])>, SimError> {
        self.call(|reply| ClusterCall::Hashes { reply }).await
    }

    /// Returns `World::snapshot` of the world of every realm: the global
    /// realm first, then the zones in zone order. Tools and tests use it.
    /// It copies the whole state of the cluster.
    pub async fn snapshots(&self) -> Result<Vec<(RealmId, Vec<u8>)>, SimError> {
        self.call(|reply| ClusterCall::Snapshots { reply }).await
    }

    /// Audits the ledgers of every world and the union of all processes:
    /// nothing was created or lost outside faucets and sinks, also not on
    /// the way between two processes. Returns the totals.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Audit`] when a world or the union does not balance.
    /// [`ClusterError::Sim`] when the call did not reach the coordinator.
    pub async fn audit(&self) -> Result<ClusterTotals, ClusterError> {
        let (reply, answer) = oneshot::channel();
        self.sim.send(Command::Cluster(ClusterCall::Audit { reply }))?;
        answer.await.unwrap_or(Err(ClusterError::Sim(SimError::Shutdown)))
    }
}

/// The owner of the coordinator thread.
///
/// [`ClusterJoin::shutdown`] stops the cluster in order: every process
/// writes its final snapshots and the workers end. Dropping the value does
/// the same.
#[must_use = "dropping the ClusterJoin stops the cluster"]
pub struct ClusterJoin {
    pub(crate) tx: SyncSender<Command>,
    pub(crate) counters: Arc<Counters>,
    pub(crate) thread: Option<JoinHandle<Result<World, ClusterError>>>,
}

impl fmt::Debug for ClusterJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterJoin").field("running", &self.thread.is_some()).finish()
    }
}

impl ClusterJoin {
    /// Stops the cluster after the calls that are already in the mailbox
    /// and returns the world of the global realm. Calls that wait get
    /// `SimError::Shutdown`.
    ///
    /// # Errors
    ///
    /// The error that stopped the cluster, when it had stopped already: a
    /// worker was lost, a realm failed, the frame protocol was broken.
    pub fn shutdown(mut self) -> Result<World, ClusterError> {
        self.stop().unwrap_or_else(|| {
            Err(ClusterError::Protocol("the coordinator thread was already joined".into()))
        })
    }

    /// Returns true when the coordinator has stopped, for example after a
    /// worker died.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn stop(&mut self) -> Option<Result<World, ClusterError>> {
        let thread = self.thread.take()?;
        // A blocking send: the mailbox may be full, and the stop request
        // must not be lost. It fails only when the thread is gone already.
        self.counters.depth.fetch_add(1, Ordering::Relaxed);
        let _ = self.tx.send(Command::Shutdown);
        Some(thread.join().unwrap_or_else(|panic| {
            Err(ClusterError::Failed {
                realm: RealmId::Global,
                message: format!("the coordinator panicked: {}", panic_message(&*panic)),
            })
        }))
    }
}

impl Drop for ClusterJoin {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
