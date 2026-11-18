//! Inputs between two frames: a spawn, an action, a command of an
//! administrator, a message from outside.
//!
//! The coordinator applies one input at a time. It sends the input to the
//! world of the right realm, waits for the answer, and then gives every
//! message that the input made to its target, one after the other. The
//! caller gets its answer after that. So the worlds see inputs and their
//! messages in one order, the order of the single process.

use unsup_kernel::{Dest, Envelope, Input, Output};
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, RealmId, ResidentId, SimError, SpawnSpec,
    ZoneId,
};

use super::super::cell::CellError;
use super::super::error::ClusterError;
use super::super::lookup::Where;
use super::super::proto::{Applied, FromWorker, Packet, Routed, ToWorker, ZoneBatch};
use super::super::wire::Blob;
use super::core::{Core, unexpected};
use crate::external::{self, PostError};

/// A fatal failure of the cluster, or the answer for the caller.
type Answer<T> = Result<Result<T, SimError>, ClusterError>;

impl Core {
    /// Applies one input in the world of a zone and routes what it made.
    fn in_zone(&mut self, zone: ZoneId, input: Input) -> Result<Applied, ClusterError> {
        let slot = self.slot(zone)?;
        let (output, packets) = match self.ask(slot, &ToWorker::Apply { zone, input })? {
            FromWorker::Applied { output, packets, residents, dropped } => {
                if let Some(count) = self.residents.get_mut(zone.0 as usize) {
                    *count = residents;
                }
                self.dropped[slot] = dropped;
                (output, packets)
            }
            other => return Err(unexpected(slot, "Applied", &other)),
        };
        if !matches!(output, Applied::Refused) {
            self.dirty = true;
        }
        self.settle(RealmId::Zone(zone), packets)?;
        Ok(output)
    }

    /// Applies one input in the world of the global realm and routes what
    /// it made. A journal that refuses the input is not fatal: nothing
    /// changed.
    fn in_global(&mut self, input: Input) -> Result<Option<Output>, ClusterError> {
        let output = match self.global.apply(input) {
            Ok(output) => output,
            Err(CellError::Journal(message)) => {
                tracing::error!(%message, "the journal of the global realm refused an input");
                return Ok(None);
            }
            Err(error) => return Err(Core::global_failed(&error)),
        };
        self.dirty = true;
        let packets = self.global.take().iter().map(Packet::of).collect();
        self.settle(RealmId::Global, packets)?;
        Ok(Some(output))
    }

    /// Gives one message to one zone, alone, and takes what comes back.
    fn give_zone(
        &mut self,
        zone: ZoneId,
        bytes: Blob,
        probe: bool,
        arrival: bool,
    ) -> Result<(), ClusterError> {
        let slot = self.slot(zone)?;
        let mut batch = ZoneBatch { zone, ..ZoneBatch::default() };
        if arrival {
            batch.arrivals.push(bytes);
            self.routed.migrations += 1;
        } else {
            batch.messages.push(Routed { index: 0, probe, bytes });
            self.routed.messages += 1;
        }
        let delivered = self.deliver(slot, vec![batch], None)?;
        for (zone, residents) in &delivered.arrived {
            for &resident in residents {
                self.lookup.moved(resident, *zone);
            }
        }
        self.take_back(delivered.bounced)
    }

    /// Routes what one input made, in sending order. A resident that
    /// leaves its zone is in its new zone before the next message goes
    /// out, as in one process.
    fn settle(&mut self, from: RealmId, packets: Vec<Packet>) -> Result<(), ClusterError> {
        for packet in packets {
            if packet.arrive {
                let envelope = packet.open().map_err(ClusterError::Protocol)?;
                for relayed in self.relay(Vec::new(), vec![envelope])? {
                    let Dest::Zone(zone) = relayed.to else { continue };
                    self.give_zone(zone, Blob(relayed.to_bytes()), false, true)?;
                }
                continue;
            }
            match packet.to {
                Dest::Global => {
                    let envelope: Envelope = packet.open().map_err(ClusterError::Protocol)?;
                    self.give_global(vec![envelope])?;
                }
                Dest::Zone(zone) => self.give_zone(zone, packet.bytes, false, false)?,
                Dest::Resident(resident) => match self.lookup.find(&self.global.world, resident) {
                    Where::Alive(zone) => self.give_zone(zone, packet.bytes, false, false)?,
                    Where::Gone(zone) => self.give_zone(zone, packet.bytes, true, false)?,
                    Where::Unknown => {
                        return Err(ClusterError::Protocol(format!(
                            "a message is for resident {resident}, which is not in the directory"
                        )));
                    }
                },
                Dest::AllZones => {
                    let sender = from.zone();
                    for slot in 0..self.links.len() {
                        let batches: Vec<ZoneBatch> = self.zones_of[slot]
                            .iter()
                            .filter(|&&zone| Some(zone) != sender)
                            .map(|&zone| ZoneBatch {
                                zone,
                                arrivals: Vec::new(),
                                messages: vec![Routed {
                                    index: 0,
                                    probe: false,
                                    bytes: packet.bytes.clone(),
                                }],
                            })
                            .collect();
                        if batches.is_empty() {
                            continue;
                        }
                        self.routed.messages += batches.len() as u64;
                        let delivered = self.deliver(slot, batches, None)?;
                        self.take_back(delivered.bounced)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Creates a resident. Every resident is made in the spawn zone: one
    /// world gives out the ids.
    pub(crate) fn spawn(&mut self, spec: SpawnSpec) -> Answer<ResidentId> {
        let zone = self.spawn_zone;
        Ok(match self.in_zone(zone, Input::Spawn(spec))? {
            Applied::Spawned(id) => {
                self.lookup.moved(id, zone);
                Ok(id)
            }
            Applied::Error(error) => Err(error),
            Applied::Refused => Err(SimError::Overloaded),
            _ => Err(SimError::Invalid),
        })
    }

    /// Sends one action request of a resident to the zone of the resident.
    pub(crate) fn act(
        &mut self,
        resident: ResidentId,
        request: ActionRequest,
    ) -> Answer<ActionAck> {
        let Where::Alive(zone) = self.lookup.find(&self.global.world, resident) else {
            return Ok(Err(SimError::UnknownResident));
        };
        Ok(match self.in_zone(zone, Input::Act { resident, request })? {
            Applied::Ack(ack) => Ok(ack),
            Applied::Error(error) => Err(error),
            Applied::Refused => Err(SimError::Overloaded),
            _ => Err(SimError::Invalid),
        })
    }

    /// Runs a command of an administrator.
    pub(crate) fn admin(&mut self, command: AdminCommand) -> Answer<AdminReply> {
        let resident = match &command {
            AdminCommand::Freeze { resident }
            | AdminCommand::Unfreeze { resident }
            | AdminCommand::Despawn { resident } => Some(*resident),
            // The coordinator answers a zone rollback before it gets here.
            AdminCommand::SetParam { .. } | AdminCommand::RollbackZone { .. } => None,
        };
        let zone = resident.and_then(|resident| {
            match self.lookup.find(&self.global.world, resident) {
                // The world of the zone in which a resident left the
                // world says that it is unknown, as one process does.
                Where::Alive(zone) | Where::Gone(zone) => Some(zone),
                Where::Unknown => None,
            }
        });
        let Some(zone) = zone else {
            // A parameter belongs to the global realm. A command for a
            // resident that never lived goes there too: the kernel words
            // the refusal, and it is in the log as in one process.
            return Ok(match self.in_global(Input::Admin(command))? {
                Some(Output::Admin(reply)) => Ok(reply),
                Some(Output::Error(error)) => Err(error),
                Some(_) => Err(SimError::Invalid),
                None => Err(SimError::Overloaded),
            });
        };
        Ok(match self.in_zone(zone, Input::Admin(command))? {
            Applied::Admin(reply) => Ok(reply),
            Applied::Error(error) => Err(error),
            Applied::Refused => Err(SimError::Overloaded),
            _ => Err(SimError::Invalid),
        })
    }

    /// Takes messages of a service outside the simulation for the global
    /// realm.
    pub(crate) fn post(
        &mut self,
        messages: Vec<Envelope>,
    ) -> Result<Result<(), PostError>, ClusterError> {
        if let Some(kind) = external::unhandled(self.global.world.blueprint(), &messages) {
            return Ok(Err(PostError::NoHandler { kind }));
        }
        Ok(match self.in_global(Input::Deliver { messages })? {
            Some(Output::Delivered) => Ok(()),
            Some(Output::Error(error)) => Err(PostError::Sim(error)),
            Some(_) => Err(PostError::Sim(SimError::Invalid)),
            None => Err(PostError::Sim(SimError::Overloaded)),
        })
    }
}
