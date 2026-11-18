//! One frame of the cluster, and the exchange at its end.
//!
//! A frame of one process (`World::apply` in the kernel) has three steps.
//! The cluster does the same three steps over the network:
//!
//! 1. Every worker ticks its zones and sends what they sent.
//! 2. The global realm gets the messages of the zones for it, in zone
//!    order, and ticks.
//! 3. Residents that left a zone arrive in their new zone. Then every
//!    other message goes to its zone: first those of the global realm,
//!    then those of the zones in zone order.
//!
//! The coordinator waits for every answer of a step before it starts the
//! next one. The order of the messages is fixed by the zone numbers and
//! the sending order, never by the time at which an answer comes.

use std::time::{Duration, Instant};

use unsup_kernel::{Dest, Envelope};
use unsup_protocol::{FrameOutput, RealmId, ZoneId};

use super::super::error::ClusterError;
use super::super::plan::Plan;
use super::super::proto::{FromWorker, Packet, SealMark, ToWorker, ZoneStepped};
use super::core::{Core, unexpected};

/// How long the phases of one frame took.
#[derive(Clone, Debug, Default)]
pub(crate) struct FrameTimes {
    /// From the step order to the last answer of a worker.
    pub zones: Duration,
    /// The messages for the global realm and its tick.
    pub global: Duration,
    /// Arrivals and messages to the zones, and the seal.
    pub exchange: Duration,
    /// From the step order to the answer of each worker.
    pub workers: Vec<Duration>,
}

/// What one frame gave.
pub(crate) struct Framed {
    /// The output of the whole world for viewers.
    pub output: FrameOutput,
    /// Tick time of each zone in nanoseconds, by zone number.
    pub elapsed: Vec<u64>,
    /// How long the phases took.
    pub times: FrameTimes,
}

impl Core {
    /// Routes what the realms sent and gives it to them: the messages for
    /// the global realm first, then the tick of the global realm when
    /// `step` is set, then arrivals and messages to the zones. All realms
    /// then mark the seal.
    ///
    /// `global_out` is what the global realm sent before this exchange.
    /// `zone_out` is what each zone sent, in zone order.
    ///
    /// Returns the output of the global realm when it ticked, and the
    /// times of the two phases.
    fn exchange(
        &mut self,
        mut global_out: Vec<Envelope>,
        zone_out: Vec<(ZoneId, Vec<Packet>)>,
        step: Option<Vec<ZoneId>>,
        frame_done: bool,
    ) -> Result<(Option<FrameOutput>, Duration, Duration), ClusterError> {
        let start = Instant::now();
        let protocol = ClusterError::Protocol;
        let mut for_global = Vec::new();
        let mut arrivals = Vec::new();
        let mut rest = Vec::new();
        for (zone, packets) in zone_out {
            for packet in packets {
                if packet.arrive {
                    arrivals.push(packet.open().map_err(protocol)?);
                } else if packet.to == Dest::Global {
                    for_global.push(packet.open().map_err(protocol)?);
                } else {
                    rest.push((RealmId::Zone(zone), packet));
                }
            }
        }
        let relayed = self.relay(for_global, arrivals)?;
        let global_frame = match step {
            Some(skip) => {
                let frame = self.global.step(skip).map_err(|e| Core::global_failed(&e))?;
                // The directory of the global realm is exact now.
                self.lookup.clear();
                global_out.extend(self.global.take());
                Some(frame)
            }
            None => None,
        };
        let global_done = Instant::now();

        let mut plan = Plan::new(self.zones());
        let planned = (|| {
            for envelope in &relayed {
                let Dest::Zone(zone) = envelope.to else { continue };
                plan.arrival(zone, envelope.to_bytes())?;
            }
            let world = &self.global.world;
            let lookup = &self.lookup;
            for envelope in &global_out {
                plan.message(RealmId::Global, envelope.to, envelope.to_bytes(), |resident| {
                    lookup.find(world, resident)
                })?;
            }
            for (from, packet) in rest {
                plan.message(from, packet.to, packet.bytes.0, |resident| {
                    lookup.find(world, resident)
                })?;
            }
            Ok(())
        })();
        planned
            .map_err(|e: super::super::plan::PlanError| ClusterError::Protocol(e.to_string()))?;
        self.routed.migrations += plan.migrations;
        self.routed.messages += plan.messages;

        // Every worker gets its part, also an empty one: it marks the seal.
        let epoch = self.take_epoch();
        let seal = Some(SealMark { epoch, frame_done });
        for slot in 0..self.links.len() {
            let batches = plan.take(&self.zones_of[slot]);
            self.links[slot].send(&ToWorker::Deliver { batches, seal })?;
        }
        let mut bounced = Vec::new();
        for slot in 0..self.links.len() {
            bounced.extend(self.delivered(slot)?.bounced);
        }
        self.take_back(bounced)?;
        self.global.seal(epoch, frame_done).map_err(|e| Core::global_failed(&e))?;
        self.dirty = false;
        Ok((global_frame, global_done - start, global_done.elapsed()))
    }

    /// The exchange after world generation: every realm gets what the
    /// others sent when they were made, and no realm ticks. It ends with
    /// the first seal.
    pub(crate) fn first_exchange(
        &mut self,
        zone_out: Vec<(ZoneId, Vec<Packet>)>,
    ) -> Result<(), ClusterError> {
        let global_out = self.global.take();
        self.exchange(global_out, zone_out, None, false).map(|_| ())
    }

    /// Runs one frame in the whole cluster.
    pub(crate) fn frame(&mut self, skip: Vec<ZoneId>) -> Result<Framed, ClusterError> {
        let start = Instant::now();
        let order = ToWorker::Step { skip: skip.clone() };
        for link in &mut self.links {
            link.send(&order)?;
        }
        let mut stepped: Vec<Option<ZoneStepped>> = (0..self.zones()).map(|_| None).collect();
        let mut workers = Vec::with_capacity(self.links.len());
        for slot in 0..self.links.len() {
            let zones = match self.links[slot].recv(self.reply_timeout)? {
                FromWorker::Stepped { zones } => zones,
                other => return Err(unexpected(slot, "Stepped", &other)),
            };
            workers.push(start.elapsed());
            for zone in zones {
                let index = zone.zone.0 as usize;
                match stepped.get_mut(index) {
                    Some(entry) if entry.is_none() && self.slot_of[index] == slot => {
                        *entry = Some(zone);
                    }
                    _ => {
                        return Err(ClusterError::Protocol(format!(
                            "worker {slot} stepped zone {}, which is not its zone",
                            zone.zone
                        )));
                    }
                }
            }
        }
        let zones_time = start.elapsed();

        let mut output = FrameOutput::default();
        let mut zone_events = Vec::new();
        let mut elapsed = Vec::with_capacity(stepped.len());
        let mut zone_out = Vec::with_capacity(stepped.len());
        for (index, zone) in stepped.into_iter().enumerate() {
            let Some(zone) = zone else {
                return Err(ClusterError::Protocol(format!("no worker stepped zone {index}")));
            };
            elapsed.push(zone.elapsed);
            zone_events.extend(zone.frame.events);
            output.woke.extend(zone.frame.woke);
            output.zones.extend(zone.frame.zones);
            zone_out.push((zone.zone, zone.packets));
        }

        let (global_frame, global_time, exchange_time) =
            self.exchange(Vec::new(), zone_out, Some(skip), true)?;
        // The global realm ticked: `exchange` got `Some(skip)`.
        let global_frame = global_frame.unwrap_or_default();
        output.frame = global_frame.frame;
        output.clock = global_frame.clock;
        // The events of the global realm come first, as in one process.
        output.events = global_frame.events;
        output.events.extend(zone_events);
        output.woke.extend(global_frame.woke);
        output.woke.sort_unstable();
        output.woke.dedup();
        let times =
            FrameTimes { zones: zones_time, global: global_time, exchange: exchange_time, workers };
        Ok(Framed { output, elapsed, times })
    }
}
