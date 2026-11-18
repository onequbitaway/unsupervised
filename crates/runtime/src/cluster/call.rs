//! The calls that only a cluster takes.

use tokio::sync::oneshot;
use unsup_protocol::{RealmId, SimError};

use super::error::ClusterError;
use super::stats::ClusterStats;
use super::totals::ClusterTotals;

/// Where the answer to a cluster call goes.
pub(crate) type ClusterReply<T> = oneshot::Sender<Result<T, SimError>>;

/// A call of a [`ClusterHandle`](super::ClusterHandle) that a plain
/// runtime does not have, or a signal inside the coordinator.
pub(crate) enum ClusterCall {
    /// A connection ended. The coordinator looks at all of them.
    Check,
    /// Numbers about the cluster.
    Stats { reply: ClusterReply<ClusterStats> },
    /// The state hash of the world of every realm.
    Hashes { reply: ClusterReply<Vec<(RealmId, [u8; 32])>> },
    /// The snapshot of the world of every realm.
    Snapshots { reply: ClusterReply<Vec<(RealmId, Vec<u8>)>> },
    /// The audit of the union of all worlds.
    Audit { reply: oneshot::Sender<Result<ClusterTotals, ClusterError>> },
}
