//! The messages between the coordinator and a worker.
//!
//! The coordinator asks and the worker answers: every [`ToWorker`] message
//! has exactly one [`FromWorker`] answer, except `Refuse` and `Stop`, which
//! end the connection. A worker speaks first only in the handshake
//! (`Hello`) and when it fails (`Failed`).

use serde::{Deserialize, Serialize};
use unsup_kernel::{Dest, Envelope, Input, Output};
use unsup_protocol::{
    ActionAck, AdminReply, FrameOutput, Observation, ObserveOpts, RealmId, ResidentId, SimError,
    WorldView, ZoneId,
};

use super::seals::SealPoint;
use super::totals::ClusterTotals;
use super::wire::{Blob, Hash};
use crate::journal::JournalStats;

/// Version of the protocol. Both sides must have the same number.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// The kind of the kernel message that carries a resident to another zone.
pub(crate) const ARRIVE_KIND: &str = "kernel.arrive";

/// One envelope of the kernel on the wire: the bytes of
/// `Envelope::to_bytes` and what the router must know to route it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Packet {
    /// Where the envelope goes.
    pub to: Dest,
    /// True when the envelope carries a resident that migrates.
    pub arrive: bool,
    /// The envelope.
    pub bytes: Blob,
}

impl Packet {
    /// Wraps an envelope.
    pub(crate) fn of(envelope: &Envelope) -> Packet {
        Packet {
            to: envelope.to,
            arrive: envelope.kind() == ARRIVE_KIND,
            bytes: Blob(envelope.to_bytes()),
        }
    }

    /// Reads the envelope.
    pub(crate) fn open(&self) -> Result<Envelope, String> {
        open(&self.bytes.0)
    }
}

/// Reads an envelope from its transport bytes.
pub(crate) fn open(bytes: &[u8]) -> Result<Envelope, String> {
    Envelope::from_bytes(bytes).map_err(|e| e.to_string())
}

/// One message for a zone, with its place in the routing order.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Routed {
    /// Position of the message in the routing order of the frame.
    pub index: u32,
    /// True when the directory says that the resident of the message is
    /// gone. The zone then gives back what the kernel makes of it.
    pub probe: bool,
    /// The envelope.
    pub bytes: Blob,
}

/// Everything that one zone gets at one point of the frame protocol.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct ZoneBatch {
    /// The zone.
    pub zone: ZoneId,
    /// Residents that arrive, in the order of the zones that they left.
    pub arrivals: Vec<Blob>,
    /// Messages, in the routing order.
    pub messages: Vec<Routed>,
}

impl ZoneBatch {
    /// Returns true when the batch holds nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.arrivals.is_empty() && self.messages.is_empty()
    }
}

/// A consistent point that every realm marks in its storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SealMark {
    /// Number of the seal. It grows by one with every seal.
    pub epoch: u64,
    /// True when the seal ends a frame. The journal then looks whether a
    /// snapshot is due.
    pub frame_done: bool,
}

/// A question about the state of the worlds of a process.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Inspect {
    /// The state hash of every world.
    Hashes,
    /// The snapshot of every world.
    Snapshots,
    /// The ledger totals of every world.
    Audit,
}

/// The answer to an [`Inspect`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Inspection {
    /// `World::state_hash` per realm.
    Hashes(Vec<(RealmId, Hash)>),
    /// `World::snapshot` per realm.
    Snapshots(Vec<(RealmId, Blob)>),
    /// The sum of `World::audit` over the realms, or the first failure.
    Audit(Result<ClusterTotals, String>),
}

/// What the coordinator sends.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum ToWorker {
    /// The handshake passed. These are the zones of the worker.
    Welcome { zones: Vec<ZoneId> },
    /// The handshake failed. The connection ends.
    Refuse { reason: String },
    /// Make the worlds (`None`) or recover them to the seal of this epoch.
    Begin { recover: Option<u64> },
    /// Run one frame in every zone.
    Step { skip: Vec<ZoneId> },
    /// Give the zones their arrivals and messages, then mark the seal.
    Deliver { batches: Vec<ZoneBatch>, seal: Option<SealMark> },
    /// Apply one input to the world of a zone.
    Apply { zone: ZoneId, input: Input },
    /// Build the observation of a resident.
    Observe { zone: ZoneId, resident: ResidentId, opts: ObserveOpts },
    /// The start of a `wait` call: the notice cursor, and the observation
    /// when news is there already or `now` is set.
    Peek { zone: ZoneId, resident: ResidentId, since: Option<u64>, now: bool },
    /// The observations of waiting calls: zone, resident, notice cursor.
    ObserveMany { items: Vec<(ZoneId, ResidentId, u64)> },
    /// Everything in one zone, for a viewer.
    View { zone: ZoneId },
    /// A question about the state of the worlds.
    Inspect(Inspect),
    /// Stop in order: mark the seal, write the final snapshots, answer
    /// `Bye`.
    Shutdown { seal: Option<SealMark> },
    /// The cluster failed. Stop at once.
    Stop { reason: String },
}

/// The stored state of one zone, as a worker finds it at the start.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ZoneState {
    /// The zone.
    pub zone: ZoneId,
    /// The seals that the zone can recover to, oldest first.
    pub seals: Vec<SealPoint>,
}

/// A zone that is ready to run.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ZoneReady {
    /// The zone.
    pub zone: ZoneId,
    /// Frame of the world.
    pub frame: u64,
    /// The residents of the zone.
    pub residents: Vec<ResidentId>,
    /// What world generation sent. Empty after a recovery.
    pub packets: Vec<Packet>,
}

/// What one zone did in a frame.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct ZoneStepped {
    /// The zone.
    pub zone: ZoneId,
    /// The output of the world for viewers.
    pub frame: FrameOutput,
    /// What the zone sent, in sending order.
    pub packets: Vec<Packet>,
    /// Time of the zone tick in nanoseconds.
    pub elapsed: u64,
}

/// A message that a zone gave back when it got it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Bounce {
    /// `Routed::index` of the message that caused it.
    pub index: u32,
    /// What the kernel made of the message.
    pub packet: Packet,
}

/// What a worker did with a `Deliver`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct Delivered {
    /// The residents that arrived, per zone, in arrival order.
    pub arrived: Vec<(ZoneId, Vec<ResidentId>)>,
    /// Messages that came back out.
    pub bounced: Vec<Bounce>,
    /// Residents per zone of the worker now.
    pub residents: Vec<(ZoneId, u32)>,
    /// Messages that the kernel dropped in the worlds of the worker, since
    /// their start.
    pub dropped: u64,
    /// The numbers of the journals of the worker, summed.
    pub journal: JournalStats,
}

/// The answer of a world to one input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum Applied {
    /// `Output::Spawned`.
    Spawned(ResidentId),
    /// `Output::Ack`.
    Ack(ActionAck),
    /// `Output::Admin`.
    Admin(AdminReply),
    /// `Output::Delivered`.
    Delivered,
    /// `Output::Error`.
    Error(SimError),
    /// The journal of the zone did not record the input. The world did
    /// not get it.
    Refused,
}

impl Applied {
    /// Maps the output of `World::apply`.
    pub(crate) fn of(output: Output) -> Applied {
        match output {
            Output::Spawned(id) => Applied::Spawned(id),
            Output::Ack(ack) => Applied::Ack(ack),
            Output::Admin(reply) => Applied::Admin(reply),
            Output::Delivered => Applied::Delivered,
            Output::Error(error) => Applied::Error(error),
            // A frame is never applied as a single input of a zone.
            Output::Frame(_) => Applied::Error(SimError::Invalid),
        }
    }
}

/// The start of a `wait` call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Peeked {
    /// The notice cursor of the call.
    pub since: u64,
    /// The observation since the cursor, when the coordinator asked for it
    /// or news is there.
    pub observation: Option<Observation>,
}

/// What a worker sends.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum FromWorker {
    /// The first message of a connection.
    Hello { protocol: u32, world: Hash, plugins: Hash, slot: u32 },
    /// The answer to `Welcome`.
    State { zones: Vec<ZoneState> },
    /// The answer to `Begin`.
    Ready { zones: Vec<ZoneReady> },
    /// The answer to `Step`.
    Stepped { zones: Vec<ZoneStepped> },
    /// The answer to `Deliver`.
    Delivered(Delivered),
    /// The answer to `Apply`.
    Applied { output: Applied, packets: Vec<Packet>, residents: u32, dropped: u64 },
    /// The answer to `Observe`.
    Observed(Result<Observation, SimError>),
    /// The answer to `Peek`.
    Peeked(Result<Peeked, SimError>),
    /// The answer to `ObserveMany`.
    ObservedMany(Vec<Result<Observation, SimError>>),
    /// The answer to `View`.
    Viewed(Result<WorldView, SimError>),
    /// The answer to `Inspect`.
    Inspected(Inspection),
    /// The worker cannot go on. The connection ends.
    Failed { realm: Option<RealmId>, message: String },
    /// The answer to `Shutdown`.
    Bye,
}

impl FromWorker {
    /// Returns the name of the message, for an error text.
    pub(crate) fn name(&self) -> &'static str {
        match self {
            FromWorker::Hello { .. } => "Hello",
            FromWorker::State { .. } => "State",
            FromWorker::Ready { .. } => "Ready",
            FromWorker::Stepped { .. } => "Stepped",
            FromWorker::Delivered(_) => "Delivered",
            FromWorker::Applied { .. } => "Applied",
            FromWorker::Observed(_) => "Observed",
            FromWorker::Peeked(_) => "Peeked",
            FromWorker::ObservedMany(_) => "ObservedMany",
            FromWorker::Viewed(_) => "Viewed",
            FromWorker::Inspected(_) => "Inspected",
            FromWorker::Failed { .. } => "Failed",
            FromWorker::Bye => "Bye",
        }
    }
}
