//! Numbers about a running cluster.

use serde::{Deserialize, Serialize};
use unsup_protocol::ZoneId;

/// Numbers about one worker.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStats {
    /// Slot of the worker.
    pub slot: u32,
    /// The zones that the worker hosts.
    pub zones: Vec<ZoneId>,
    /// Residents in those zones after the last frame or input.
    pub residents: u64,
    /// Bytes that the coordinator sent to the worker.
    pub bytes_sent: u64,
    /// Bytes that the coordinator received from the worker.
    pub bytes_received: u64,
    /// Time from the step order of the last frame to the answer of this
    /// worker, in microseconds.
    pub last_step_micros: u64,
}

/// Numbers about a running cluster. Counters count since the start of the
/// coordinator. Times are in microseconds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterStats {
    /// Number of the last frame.
    pub frame: u64,
    /// Number of the last seal: the point to which a recovery goes back.
    pub epoch: u64,
    /// One entry per worker, by slot.
    pub workers: Vec<WorkerStats>,
    /// Residents that changed zone.
    pub migrations: u64,
    /// Messages between realms that the coordinator routed, without the
    /// migrations.
    pub messages: u64,
    /// Messages that the kernel dropped in any world of the cluster. It
    /// counts messages for a resident that no longer exists, and also
    /// messages for a resident that the world of the sender has never
    /// seen (see the limits in `README.md`).
    pub dropped_messages: u64,
    /// Time of the last frame in which the zones ticked: from the step
    /// order to the last answer.
    pub last_zones_micros: u64,
    /// Time of the last frame in which the global realm took its messages
    /// and ticked.
    pub last_global_micros: u64,
    /// Time of the last frame in which the messages and the residents
    /// that migrate went to their zones.
    pub last_exchange_micros: u64,
    /// Sum of the three phases over all frames.
    pub zones_micros: u64,
    /// See `zones_micros`.
    pub global_micros: u64,
    /// See `zones_micros`.
    pub exchange_micros: u64,
}
