//! What the coordinator owns, and how it talks to the workers and to the
//! global realm.

use std::time::Duration;

use unsup_kernel::{Dest, Envelope};
use unsup_protocol::{RealmId, ZoneId};

use super::super::cell::{Cell, CellError};
use super::super::error::ClusterError;
use super::super::link::Link;
use super::super::lookup::Lookup;
use super::super::proto::{Bounce, Delivered, FromWorker, SealMark, ToWorker, ZoneBatch};
use crate::journal::JournalStats;

/// Counters of the router.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Routed {
    /// Residents that changed zone.
    pub migrations: u64,
    /// Messages that went to a realm, without the migrations.
    pub messages: u64,
}

/// Everything that the coordinator owns about the world.
pub(crate) struct Core {
    /// The global realm.
    pub global: Cell,
    /// The connection to each worker, by slot.
    pub links: Vec<Link>,
    /// The zones of each worker, by slot.
    pub zones_of: Vec<Vec<ZoneId>>,
    /// The slot of the worker of each zone, by zone number.
    pub slot_of: Vec<usize>,
    /// The zone in which new residents are made.
    pub spawn_zone: ZoneId,
    /// The directory changes since the last tick of the global realm.
    pub lookup: Lookup,
    /// How long a worker has to answer.
    pub reply_timeout: Duration,
    /// Number of the next seal. The first seal of a new world is 0.
    pub next_epoch: u64,
    /// An input changed a world since the last seal.
    pub dirty: bool,
    /// Residents per zone, as the workers last reported.
    pub residents: Vec<u32>,
    /// Messages that the kernel dropped in the worlds of each worker.
    pub dropped: Vec<u64>,
    /// The journal numbers of each worker.
    pub journals: Vec<JournalStats>,
    /// Counters of the router.
    pub routed: Routed,
}

/// Says that a worker answered with the wrong message.
pub(crate) fn unexpected(slot: usize, wanted: &str, got: &FromWorker) -> ClusterError {
    ClusterError::Protocol(format!(
        "worker {slot} answered `{}` where `{wanted}` must come",
        got.name()
    ))
}

impl Core {
    /// Returns the number of zones of the world.
    pub(crate) fn zones(&self) -> usize {
        self.slot_of.len()
    }

    /// Takes the number of the next seal.
    pub(crate) fn take_epoch(&mut self) -> u64 {
        let epoch = self.next_epoch;
        self.next_epoch += 1;
        epoch
    }

    /// Returns the slot of the worker that hosts a zone.
    pub(crate) fn slot(&self, zone: ZoneId) -> Result<usize, ClusterError> {
        self.slot_of
            .get(zone.0 as usize)
            .copied()
            .ok_or_else(|| ClusterError::Protocol(format!("zone {zone} is not in the world")))
    }

    /// Sends one message to a worker and waits for its answer.
    pub(crate) fn ask(
        &mut self,
        slot: usize,
        message: &ToWorker,
    ) -> Result<FromWorker, ClusterError> {
        self.links[slot].send(message)?;
        self.links[slot].recv(self.reply_timeout)
    }

    /// Notes what a worker reports with every delivery.
    pub(crate) fn note(&mut self, slot: usize, delivered: &Delivered) {
        for &(zone, residents) in &delivered.residents {
            if let Some(count) = self.residents.get_mut(zone.0 as usize) {
                *count = residents;
            }
        }
        self.dropped[slot] = delivered.dropped;
        self.journals[slot] = delivered.journal;
    }

    /// Waits for the answer of a worker to a `Deliver`.
    pub(crate) fn delivered(&mut self, slot: usize) -> Result<Delivered, ClusterError> {
        match self.links[slot].recv(self.reply_timeout)? {
            FromWorker::Delivered(delivered) => {
                self.note(slot, &delivered);
                Ok(delivered)
            }
            other => Err(unexpected(slot, "Delivered", &other)),
        }
    }

    /// Gives zones of one worker their batches and waits for the answer.
    pub(crate) fn deliver(
        &mut self,
        slot: usize,
        batches: Vec<ZoneBatch>,
        seal: Option<SealMark>,
    ) -> Result<Delivered, ClusterError> {
        self.links[slot].send(&ToWorker::Deliver { batches, seal })?;
        self.delivered(slot)
    }

    /// Maps a failure of the global realm.
    pub(crate) fn global_failed(error: &CellError) -> ClusterError {
        ClusterError::Failed { realm: RealmId::Global, message: error.message() }
    }

    /// Gives the global realm messages. It keeps them for its next tick.
    pub(crate) fn give_global(&mut self, messages: Vec<Envelope>) -> Result<(), ClusterError> {
        self.routed.messages += messages.len() as u64;
        self.global.deliver(messages).map_err(|e| Core::global_failed(&e))?;
        match self.global.take().first() {
            None => Ok(()),
            Some(envelope) => Err(ClusterError::Protocol(format!(
                "the global realm gave back a message `{}` for {:?}",
                envelope.kind(),
                envelope.to
            ))),
        }
    }

    /// Gives the global realm its messages and shows its world the
    /// residents that migrate. Returns those residents, in the same order,
    /// ready for their zones.
    ///
    /// The world of the global realm treats a resident that it has never
    /// seen as gone, and drops what the global realm sends to it. A world
    /// learns of a resident when the resident passes through it on the way
    /// to a zone that it does not host. So every resident that changes
    /// zone passes through this world. It is the directory that the kernel
    /// reads.
    pub(crate) fn relay(
        &mut self,
        mut messages: Vec<Envelope>,
        arrivals: Vec<Envelope>,
    ) -> Result<Vec<Envelope>, ClusterError> {
        if messages.is_empty() && arrivals.is_empty() {
            return Ok(Vec::new());
        }
        let count = arrivals.len();
        self.routed.messages += messages.len() as u64;
        messages.extend(arrivals);
        self.global.deliver(messages).map_err(|e| Core::global_failed(&e))?;
        let relayed = self.global.take();
        let all_arrive = relayed.iter().all(|envelope| matches!(envelope.to, Dest::Zone(_)));
        if relayed.len() != count || !all_arrive {
            return Err(ClusterError::Protocol(format!(
                "the global realm passed on {} messages for {count} residents that migrate",
                relayed.len()
            )));
        }
        Ok(relayed)
    }

    /// Takes what zones gave back when they got their messages: wires for
    /// residents that are gone, which the kernel sends on to the global
    /// realm. They go there in the routing order.
    pub(crate) fn take_back(&mut self, mut bounced: Vec<Bounce>) -> Result<(), ClusterError> {
        if bounced.is_empty() {
            return Ok(());
        }
        bounced.sort_by_key(|bounce| bounce.index);
        let mut wires = Vec::with_capacity(bounced.len());
        for bounce in &bounced {
            if bounce.packet.to != Dest::Global {
                return Err(ClusterError::Protocol(format!(
                    "a zone gave back a message for {:?}: the directory sent it to the wrong zone",
                    bounce.packet.to
                )));
            }
            wires.push(bounce.packet.open().map_err(ClusterError::Protocol)?);
        }
        self.give_global(wires)
    }
}
