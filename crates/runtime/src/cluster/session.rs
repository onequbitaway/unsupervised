//! The running part of a worker: the worlds of its zones and the loop
//! that answers the coordinator.

use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use unsup_kernel::{Dest, Input, TickClock, World};
use unsup_protocol::{Observation, ObserveOpts, RealmId, ResidentId, SimError, ViewQuery, ZoneId};

use super::cell::{Cell, CellError, CellStore, Start};
use super::config::WorkerConfig;
use super::error::ClusterError;
use super::proto::{
    Applied, Bounce, Delivered, FromWorker, Inspect, Inspection, Packet, Peeked, SealMark,
    ToWorker, ZoneBatch, ZoneReady, ZoneState, ZoneStepped, open,
};
use super::seals::SealPoint;
use super::totals::ClusterTotals;
use super::wire::{Blob, Hash, read_frame, write_frame};
use super::worker::lost;
use crate::durable::Plugins;
use crate::journal::JournalStats;

/// Adds the numbers of one journal to a sum.
pub(crate) fn add_journal(sum: &mut JournalStats, one: &JournalStats) {
    sum.records += one.records;
    sum.lag += one.lag;
    sum.write_errors += one.write_errors;
    sum.snapshots += one.snapshots;
    sum.snapshot_errors += one.snapshot_errors;
    sum.last_snapshot_frame = sum.last_snapshot_frame.max(one.last_snapshot_frame);
    sum.last_snapshot_capture_micros =
        sum.last_snapshot_capture_micros.max(one.last_snapshot_capture_micros);
    sum.last_snapshot_write_micros =
        sum.last_snapshot_write_micros.max(one.last_snapshot_write_micros);
    sum.last_snapshot_bytes += one.last_snapshot_bytes;
}

/// The running part of a worker.
pub(crate) struct Session {
    stream: TcpStream,
    max_frame: u32,
    /// One cell per zone, in zone order.
    cells: Vec<Cell>,
}

/// What one zone did with its batch.
struct Taken {
    arrived: Vec<ResidentId>,
    bounced: Vec<Bounce>,
}

/// A failure of one zone.
fn failed(cell: &Cell, error: &CellError) -> ClusterError {
    ClusterError::Failed { realm: cell.realm, message: error.message() }
}

/// Gives one zone its batch and marks the seal.
fn deliver(
    cell: &mut Cell,
    batch: Option<ZoneBatch>,
    seal: Option<SealMark>,
) -> Result<Taken, ClusterError> {
    let unreadable = |cell: &Cell, e: String| ClusterError::Failed {
        realm: cell.realm,
        message: format!("a message of the coordinator cannot be read: {e}"),
    };
    let before = cell.world.residents();
    let mut bounced = Vec::new();
    if let Some(batch) = batch {
        let mut pending = Vec::with_capacity(batch.arrivals.len() + batch.messages.len());
        for arrival in &batch.arrivals {
            pending.push(open(&arrival.0).map_err(|e| unreadable(cell, e))?);
        }
        // What a zone gives back after the messages before a probe has no
        // message that it can be traced to. `u32::MAX` marks it.
        let flush = |cell: &mut Cell, pending: &mut Vec<_>, bounced: &mut Vec<Bounce>, index| {
            cell.deliver(std::mem::take(pending)).map_err(|e| failed(cell, &e))?;
            // A world that does not host every zone passes a message for
            // all zones on to the others. The coordinator gave it to them
            // already, so that copy ends here.
            let back = cell.take();
            let back = back.iter().filter(|out| out.to != Dest::AllZones);
            bounced.extend(back.map(|out| Bounce { index, packet: Packet::of(out) }));
            Ok::<(), ClusterError>(())
        };
        for routed in batch.messages {
            let envelope = open(&routed.bytes.0).map_err(|e| unreadable(cell, e))?;
            if routed.probe {
                flush(cell, &mut pending, &mut bounced, u32::MAX)?;
                pending.push(envelope);
                flush(cell, &mut pending, &mut bounced, routed.index)?;
            } else {
                pending.push(envelope);
            }
        }
        flush(cell, &mut pending, &mut bounced, u32::MAX)?;
    }
    // New rows are added at the end of the resident table, so the rows
    // after the old length are the residents that arrived, in order.
    let arrived = if cell.world.residents() > before {
        cell.residents().split_off(before)
    } else {
        Vec::new()
    };
    if let Some(mark) = seal {
        cell.seal(mark.epoch, mark.frame_done).map_err(|e| failed(cell, &e))?;
    }
    Ok(Taken { arrived, bounced })
}

impl Session {
    /// Makes the session of a connection that passed the handshake.
    pub(crate) fn new(stream: TcpStream, max_frame: u32) -> Session {
        Session { stream, max_frame, cells: Vec::new() }
    }

    /// Sends a message that needs no answer. The coordinator may be gone.
    pub(crate) fn say(&mut self, message: &FromWorker) {
        let _ = write_frame(&mut self.stream, message, self.max_frame);
    }

    fn send(&mut self, message: &FromWorker) -> Result<(), ClusterError> {
        write_frame(&mut self.stream, message, self.max_frame).map(|_| ()).map_err(|e| lost(&e))
    }

    fn read(&mut self) -> Result<ToWorker, ClusterError> {
        match read_frame::<_, ToWorker>(&mut self.stream, self.max_frame) {
            Ok((ToWorker::Stop { reason }, _)) => Err(ClusterError::Stopped { reason }),
            Ok((message, _)) => Ok(message),
            Err(error) => Err(lost(&error)),
        }
    }

    fn cell(&mut self, zone: ZoneId) -> Result<&mut Cell, ClusterError> {
        self.cells.iter_mut().find(|cell| cell.realm == RealmId::Zone(zone)).ok_or_else(|| {
            ClusterError::Protocol(format!(
                "the coordinator asked for zone {zone}, which this worker does not host"
            ))
        })
    }

    /// The second part of the handshake: what is stored, then the worlds.
    fn start(
        &mut self,
        zones: &[ZoneId],
        plugins: &Plugins,
        config: &WorkerConfig,
    ) -> Result<(), ClusterError> {
        let store = |zone: ZoneId| {
            config.durable.as_ref().map(|durable| CellStore::of(durable, RealmId::Zone(zone)))
        };
        let mut states = Vec::with_capacity(zones.len());
        for &zone in zones {
            let seals = match store(zone) {
                Some(store) => store.seals().map_err(|message| ClusterError::Recovery {
                    realm: Some(RealmId::Zone(zone)),
                    message,
                })?,
                None => Vec::new(),
            };
            states.push(ZoneState { zone, seals });
        }
        self.send(&FromWorker::State { zones: states.clone() })?;
        let ToWorker::Begin { recover } = self.read()? else {
            return Err(ClusterError::Protocol("the coordinator did not send `Begin`".into()));
        };

        // Recovery replays the log of each zone. The zones do it in
        // parallel.
        let cells: Vec<Result<Cell, ClusterError>> = states
            .par_iter()
            .map(|state| {
                let realm = RealmId::Zone(state.zone);
                let start = match recover {
                    None => Start::Fresh,
                    Some(epoch) => {
                        Start::Recover(find_seal(&state.seals, epoch).ok_or_else(|| {
                            ClusterError::Recovery {
                                realm: Some(realm),
                                message: format!("the realm has no seal {epoch}"),
                            }
                        })?)
                    }
                };
                Cell::open(realm, plugins, &config.world, store(state.zone), start)
            })
            .collect();
        self.cells = cells.into_iter().collect::<Result<_, _>>()?;
        self.cells.sort_by_key(|cell| cell.realm);

        // The kernel never reads the wall clock. The worker lends it one
        // to measure zone ticks; the pacer of the coordinator reads the
        // result.
        let epoch = Instant::now();
        let clock: TickClock =
            Arc::new(move || u64::try_from(epoch.elapsed().as_nanos()).unwrap_or(u64::MAX));
        let mut ready = Vec::with_capacity(self.cells.len());
        for cell in &mut self.cells {
            cell.set_clock(Some(Arc::clone(&clock)));
            ready.push(ZoneReady {
                // The cells are in zone order. The coordinator can give the
                // zones of a worker in any order.
                zone: cell.realm.zone().unwrap_or_default(),
                frame: cell.world.frame(),
                residents: cell.residents(),
                packets: cell.take().iter().map(Packet::of).collect(),
            });
        }
        self.send(&FromWorker::Ready { zones: ready })
    }

    /// Serves the coordinator. Returns the worlds after a stop in order.
    pub(crate) fn serve(
        &mut self,
        zones: &[ZoneId],
        plugins: &Plugins,
        config: &WorkerConfig,
    ) -> Result<Vec<World>, ClusterError> {
        self.start(zones, plugins, config)?;
        loop {
            let answer = match self.read()? {
                ToWorker::Step { skip } => self.step(&skip)?,
                ToWorker::Deliver { batches, seal } => self.deliver(batches, seal)?,
                ToWorker::Apply { zone, input } => self.apply(zone, input)?,
                ToWorker::Observe { zone, resident, opts } => {
                    FromWorker::Observed(self.cell(zone)?.world.observe(resident, &opts))
                }
                ToWorker::Peek { zone, resident, since, now } => {
                    FromWorker::Peeked(peek(&self.cell(zone)?.world, resident, since, now))
                }
                ToWorker::ObserveMany { items } => {
                    let mut observations = Vec::with_capacity(items.len());
                    for (zone, resident, since) in items {
                        observations.push(observe_since(&self.cell(zone)?.world, resident, since));
                    }
                    FromWorker::ObservedMany(observations)
                }
                ToWorker::View { zone } => {
                    FromWorker::Viewed(self.cell(zone)?.world.view(&ViewQuery::Zone { zone }))
                }
                ToWorker::Inspect(what) => FromWorker::Inspected(self.inspect(what)),
                ToWorker::Shutdown { seal } => {
                    if let Some(mark) = seal {
                        for cell in &mut self.cells {
                            cell.seal(mark.epoch, mark.frame_done).map_err(|e| failed(cell, &e))?;
                        }
                    }
                    let worlds =
                        std::mem::take(&mut self.cells).into_iter().map(Cell::finish).collect();
                    self.say(&FromWorker::Bye);
                    return Ok(worlds);
                }
                ToWorker::Welcome { .. } | ToWorker::Refuse { .. } | ToWorker::Begin { .. } => {
                    return Err(ClusterError::Protocol(
                        "the coordinator sent a handshake message to a running worker".into(),
                    ));
                }
                // `read` turns it into an error.
                ToWorker::Stop { reason } => return Err(ClusterError::Stopped { reason }),
            };
            self.send(&answer)?;
        }
    }

    fn census(&self) -> Vec<(ZoneId, u32)> {
        self.cells
            .iter()
            .filter_map(|cell| Some((cell.realm.zone()?, cell.world.residents() as u32)))
            .collect()
    }

    fn step(&mut self, skip: &[ZoneId]) -> Result<FromWorker, ClusterError> {
        let zones: Vec<Result<ZoneStepped, ClusterError>> = self
            .cells
            .par_iter_mut()
            .map(|cell| {
                let frame = cell.step(skip.to_vec()).map_err(|e| failed(cell, &e))?;
                let zone = cell.realm.zone().unwrap_or_default();
                let elapsed = cell
                    .world
                    .timings()
                    .iter()
                    .find(|timing| timing.zone == zone)
                    .map_or(0, |timing| timing.elapsed);
                let packets = cell.take().iter().map(Packet::of).collect();
                Ok(ZoneStepped { zone, frame, packets, elapsed })
            })
            .collect();
        Ok(FromWorker::Stepped { zones: zones.into_iter().collect::<Result<_, _>>()? })
    }

    fn deliver(
        &mut self,
        batches: Vec<ZoneBatch>,
        seal: Option<SealMark>,
    ) -> Result<FromWorker, ClusterError> {
        let mut by_cell: Vec<Option<ZoneBatch>> = self.cells.iter().map(|_| None).collect();
        for batch in batches {
            let realm = RealmId::Zone(batch.zone);
            let Some(at) = self.cells.iter().position(|cell| cell.realm == realm) else {
                return Err(ClusterError::Protocol(format!(
                    "the coordinator sent messages for zone {}, which this worker does not host",
                    batch.zone
                )));
            };
            by_cell[at] = Some(batch);
        }
        let taken: Vec<Result<Taken, ClusterError>> = self
            .cells
            .par_iter_mut()
            .zip(by_cell)
            .map(|(cell, batch)| deliver(cell, batch, seal))
            .collect();
        let mut delivered = Delivered { residents: Vec::new(), ..Delivered::default() };
        for (cell, taken) in self.cells.iter().zip(taken) {
            let Taken { arrived, bounced } = taken?;
            let zone = cell.realm.zone().unwrap_or_default();
            if !arrived.is_empty() {
                delivered.arrived.push((zone, arrived));
            }
            delivered.bounced.extend(bounced);
            delivered.dropped += cell.world.dropped_messages();
            add_journal(&mut delivered.journal, &cell.journal_stats());
        }
        delivered.residents = self.census();
        Ok(FromWorker::Delivered(delivered))
    }

    fn apply(&mut self, zone: ZoneId, input: Input) -> Result<FromWorker, ClusterError> {
        let cell = self.cell(zone)?;
        let output = match cell.apply(input) {
            Ok(output) => Applied::of(output),
            Err(CellError::Journal(message)) => {
                tracing::error!(%message, %zone, "the journal refused an input; its caller gets `overloaded`");
                Applied::Refused
            }
            Err(error) => return Err(failed(cell, &error)),
        };
        let packets = cell.take().iter().map(Packet::of).collect();
        let residents = cell.world.residents() as u32;
        let dropped = self.cells.iter().map(|cell| cell.world.dropped_messages()).sum();
        Ok(FromWorker::Applied { output, packets, residents, dropped })
    }

    fn inspect(&self, what: Inspect) -> Inspection {
        inspect(self.cells.iter(), what)
    }
}

/// Finds the seal of an epoch.
fn find_seal(seals: &[SealPoint], epoch: u64) -> Option<SealPoint> {
    seals.iter().copied().find(|seal| seal.epoch == epoch)
}

/// Answers a question about the worlds of a process.
pub(crate) fn inspect<'a>(cells: impl Iterator<Item = &'a Cell>, what: Inspect) -> Inspection {
    match what {
        Inspect::Hashes => Inspection::Hashes(
            cells.map(|cell| (cell.realm, Hash(cell.world.state_hash()))).collect(),
        ),
        Inspect::Snapshots => Inspection::Snapshots(
            cells.map(|cell| (cell.realm, Blob(cell.world.snapshot()))).collect(),
        ),
        Inspect::Audit => {
            let mut totals = ClusterTotals::default();
            for cell in cells {
                match cell.world.audit() {
                    Ok(world) => totals.add_world(&world),
                    Err(error) => {
                        return Inspection::Audit(Err(format!("{}: {error}", cell.realm)));
                    }
                }
            }
            Inspection::Audit(Ok(totals))
        }
    }
}

/// The start of a `wait` call, as the single process does it.
fn peek(
    world: &World,
    resident: ResidentId,
    since: Option<u64>,
    now: bool,
) -> Result<Peeked, SimError> {
    let next = world.next_notice(resident)?;
    // Without a cursor only news from now on counts. A cursor that is
    // ahead of the inbox counts as "now" too.
    let since = since.map_or(next, |since| since.min(next));
    let observation =
        if since < next || now {
            Some(world.observe(
                resident,
                &ObserveOpts { since_notice: since, ..ObserveOpts::default() },
            )?)
        } else {
            None
        };
    Ok(Peeked { since, observation })
}

/// Builds the observation that a waiting call gets.
fn observe_since(world: &World, resident: ResidentId, since: u64) -> Result<Observation, SimError> {
    let since = since.min(world.next_notice(resident)?);
    world.observe(resident, &ObserveOpts { since_notice: since, ..ObserveOpts::default() })
}
