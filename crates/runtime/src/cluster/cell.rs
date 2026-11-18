//! One realm of a cluster: a world that hosts exactly that realm, with its
//! journal and its seals.
//!
//! The kernel routes a message between two realms of one world by itself,
//! in an order that depends on which realms share the world. A cluster
//! must give every realm its messages in the order of the single process,
//! whatever the split over processes is. So each realm has a world of its
//! own, every message between realms leaves through `World::take_external`,
//! and the coordinator routes it (see `plan.rs`).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;

use unsup_kernel::{Envelope, Input, Output, TickClock, World, WorldConfig};
use unsup_protocol::{FrameOutput, RealmId, ResidentId};

use super::config::ClusterDurable;
use super::error::ClusterError;
use super::fingerprint::plugin_refs;
use super::scoped::Scoped;
use super::seals::{self, SealLog, SealPoint, SealSync};
use crate::durable::{DurableConfig, DurableJournal, Plugins, SyncPolicy};
use crate::journal::{Journal, JournalStats, NoJournal};
use crate::log::{LogIndex, read_from};
use crate::replay::panic_message;
use crate::snapshot;
use crate::storage::Storage;

/// A realm could not take an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CellError {
    /// The journal did not record the input. The world did not get it.
    Journal(String),
    /// The simulation panicked. The world must not be used again.
    Panic(String),
}

impl CellError {
    /// Returns the text of the failure.
    pub(crate) fn message(&self) -> String {
        match self {
            CellError::Journal(message) => format!("the journal failed: {message}"),
            CellError::Panic(message) => format!("the simulation panicked: {message}"),
        }
    }
}

/// How a realm starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// A new world: world generation runs.
    Fresh,
    /// The stored world, as it was at this seal.
    Recover(SealPoint),
}

/// The storage of one realm and how its journal writes.
pub(crate) struct CellStore {
    storage: Arc<dyn Storage>,
    config: DurableConfig,
}

impl CellStore {
    /// Returns the storage of one realm of a process.
    pub(crate) fn of(durable: &ClusterDurable, realm: RealmId) -> CellStore {
        CellStore {
            storage: Scoped::of_realm(Arc::clone(&durable.storage), realm),
            config: durable.config.clone(),
        }
    }

    /// Returns the seals to which the realm can recover, oldest first.
    ///
    /// A seal counts only when the log still holds every input before it
    /// and a snapshot at or before it exists: a loss of power can cut the
    /// end of the log, and old snapshots are deleted.
    ///
    /// # Errors
    ///
    /// Fails when the storage cannot be read, and when it holds a world
    /// that ran frames but no seal of it can be used. The cluster must not
    /// start a new world over such a storage.
    pub(crate) fn seals(&self) -> Result<Vec<SealPoint>, String> {
        let storage = &*self.storage;
        let entries = snapshot::list(storage).map_err(|e| e.to_string())?;
        let index = LogIndex::load(storage).map_err(|e| e.to_string())?;
        if entries.is_empty() && index.is_empty() {
            return Ok(Vec::new());
        }
        // The log can be read from the newest snapshot on. Everything
        // before that snapshot was forced to the disk before it.
        let oldest = entries.first().map_or(0, |entry| entry.seq);
        let newest = entries.last().map_or(0, |entry| entry.seq);
        let mut ran = entries.iter().any(|entry| entry.frame > 0);
        let end = read_from(storage, &index, newest, u64::MAX, &mut |_, input| {
            ran |= matches!(input, Input::Step { .. });
            true
        })
        .map_err(|e| e.to_string())?
        .next_seq
        .max(newest);
        let usable: Vec<SealPoint> = seals::read(storage)
            .map_err(|e| e.to_string())?
            .into_iter()
            .filter(|point| !entries.is_empty() && (oldest..=end).contains(&point.seq))
            .collect();
        if usable.is_empty() && ran {
            return Err("the storage holds a world that ran frames, but no seal of it can be \
                        used. The cluster does not start a new world over it. Restore the file \
                        `seals` or remove the directory of the realm"
                .into());
        }
        Ok(usable)
    }

    /// Returns true when the storage holds a world.
    fn has_world(&self) -> Result<bool, String> {
        let storage = &*self.storage;
        let snapshots = snapshot::list(storage).map_err(|e| e.to_string())?;
        let index = LogIndex::load(storage).map_err(|e| e.to_string())?;
        Ok(!snapshots.is_empty() || !index.is_empty())
    }
}

/// One realm: its world, its journal and its seals.
pub(crate) struct Cell {
    /// The realm.
    pub realm: RealmId,
    /// The world. It hosts only this realm.
    pub world: World,
    journal: Box<dyn Journal>,
    seals: Option<SealLog>,
    /// Frames between two snapshots of the journal. 0: none.
    snapshot_every: u64,
    /// The frame that the world ran last, for the journal.
    last_frame: FrameOutput,
}

/// Returns the configuration of the world of one realm.
fn config_of(base: &WorldConfig, realm: RealmId) -> WorldConfig {
    WorldConfig { hosted: Some(vec![realm]), ..base.clone() }
}

impl Cell {
    /// Makes or recovers the world of a realm.
    pub(crate) fn open(
        realm: RealmId,
        plugins: &Plugins,
        base: &WorldConfig,
        store: Option<CellStore>,
        start: Start,
    ) -> Result<Cell, ClusterError> {
        let fail = |message: String| ClusterError::Recovery { realm: Some(realm), message };
        let config = config_of(base, realm);
        let Some(store) = store else {
            if start != Start::Fresh {
                return Err(fail(
                    "the process has no storage, so it has no world to recover".into(),
                ));
            }
            let world = World::new(&plugin_refs(plugins), config);
            return Ok(Cell {
                realm,
                world,
                journal: Box::new(NoJournal),
                seals: None,
                snapshot_every: 0,
                last_frame: FrameOutput::default(),
            });
        };

        // Where the log is cut. A new cluster cuts a world that never got
        // its first seal back to the world of its first snapshot.
        let cut = match start {
            Start::Recover(point) => Some(point.seq),
            Start::Fresh if store.has_world().map_err(fail)? => Some(0),
            Start::Fresh => None,
        };
        let kept: Vec<SealPoint> = match start {
            Start::Recover(point) => seals::read(&*store.storage)
                .map_err(|e| fail(e.to_string()))?
                .into_iter()
                .filter(|seal| seal.epoch <= point.epoch)
                .collect(),
            Start::Fresh => Vec::new(),
        };
        let sync = match store.config.sync {
            SyncPolicy::EveryRecord | SyncPolicy::EveryBatch => SealSync::Always,
            SyncPolicy::Interval(interval) => SealSync::Every(interval),
        };
        let snapshot_every = store.config.snapshot_every;
        let durable = DurableConfig { drop_from: cut, ..store.config };
        let mut created = false;
        let opened =
            DurableJournal::open(Arc::clone(&store.storage), Arc::clone(plugins), durable, |p| {
                created = true;
                World::new(p, config.clone())
            })
            .map_err(|e| fail(e.to_string()))?;

        let (frame, seq) = (opened.world.frame(), opened.journal.stats().records);
        match start {
            Start::Recover(_) if created => {
                return Err(fail("the storage holds no world".into()));
            }
            Start::Recover(point) if (frame, seq) != (point.frame, point.seq) => {
                return Err(fail(format!(
                    "the seal {} is at frame {} after {} inputs, but the log gives frame {frame} \
                     after {seq} inputs",
                    point.epoch, point.frame, point.seq
                )));
            }
            Start::Fresh if (frame, seq) != (0, 0) => {
                return Err(fail(format!(
                    "the storage holds a world at frame {frame} that has no seal; \
                     it cannot be the start of a new cluster"
                )));
            }
            _ => {}
        }
        if opened.world.config() != &config {
            return Err(fail("the stored world has another configuration".into()));
        }
        let mut world = opened.world;
        if matches!(start, Start::Recover(_)) {
            // The replay of the log filled the outbox for other realms
            // again. At a seal every realm had taken every message that
            // was sent to it, so these are copies of messages that have
            // arrived. They must not go out a second time.
            drop(world.take_external());
        }
        let seals = SealLog::start(
            store.storage,
            kept,
            crate::settings::settings().cluster.seals_keep,
            sync,
        )
        .map_err(|e| fail(e.to_string()))?;
        Ok(Cell {
            realm,
            world,
            journal: Box::new(opened.journal),
            seals: Some(seals),
            snapshot_every,
            last_frame: FrameOutput::default(),
        })
    }

    /// Lends the world a clock to measure zone ticks.
    pub(crate) fn set_clock(&mut self, clock: Option<TickClock>) {
        self.world.set_clock(clock);
    }

    /// Records one input and applies it.
    pub(crate) fn apply(&mut self, input: Input) -> Result<Output, CellError> {
        self.journal
            .record(&input)
            .and_then(|()| self.journal.commit())
            .map_err(|e| CellError::Journal(e.message))?;
        match catch_unwind(AssertUnwindSafe(|| self.world.apply(input))) {
            Ok(output) => Ok(output),
            Err(panic) => {
                // The log must not keep the input that caused the panic.
                self.journal.discard(1);
                Err(CellError::Panic(panic_message(&*panic)))
            }
        }
    }

    /// Runs one frame and returns the output of the world.
    pub(crate) fn step(
        &mut self,
        skip: Vec<unsup_protocol::ZoneId>,
    ) -> Result<FrameOutput, CellError> {
        match self.apply(Input::Step { skip })? {
            Output::Frame(frame) => {
                self.last_frame = FrameOutput {
                    frame: frame.frame,
                    clock: frame.clock,
                    ..FrameOutput::default()
                };
                Ok(frame)
            }
            // `Input::Step` always gives a frame.
            _ => Err(CellError::Panic("the world gave no frame for a step".into())),
        }
    }

    /// Gives the world messages of other realms. No message, no input.
    pub(crate) fn deliver(&mut self, messages: Vec<Envelope>) -> Result<(), CellError> {
        if messages.is_empty() {
            return Ok(());
        }
        self.apply(Input::Deliver { messages }).map(|_| ())
    }

    /// Takes what the realm sent to other realms.
    pub(crate) fn take(&mut self) -> Vec<Envelope> {
        self.world.take_external()
    }

    /// Returns the residents of a zone realm, in table order.
    pub(crate) fn residents(&self) -> Vec<ResidentId> {
        self.world.realm(self.realm).map_or_else(Vec::new, |realm| realm.ids().to_vec())
    }

    /// Writes down that the realm is at a consistent point of the cluster.
    pub(crate) fn seal(&mut self, epoch: u64, frame_done: bool) -> Result<(), CellError> {
        let frame = self.world.frame();
        let point = SealPoint { epoch, frame, seq: self.journal.stats().records };
        // The journal takes a snapshot in these frames. A snapshot is on
        // the disk when it is written, so its seal must be there too: a
        // recovery after a loss of power then finds a seal for the newest
        // snapshot, also when the log is forced to the disk rarely.
        let with_snapshot =
            frame_done && self.snapshot_every > 0 && frame.is_multiple_of(self.snapshot_every);
        if let Some(seals) = &mut self.seals {
            let written = seals.append(point).and_then(|()| match with_snapshot {
                true => seals.sync(),
                false => Ok(()),
            });
            written.map_err(|e| CellError::Journal(e.to_string()))?;
        }
        if frame_done {
            // A snapshot is taken here, at a seal, so that a recovery to
            // this seal finds a snapshot that is not newer than it.
            self.journal.frame_done(&self.world, &self.last_frame);
        }
        Ok(())
    }

    /// Returns the numbers of the journal.
    pub(crate) fn journal_stats(&self) -> JournalStats {
        self.journal.stats()
    }

    /// Stops the realm in order: the journal writes its final snapshot.
    /// Returns the world.
    pub(crate) fn finish(mut self) -> World {
        if let Some(seals) = &mut self.seals
            && let Err(error) = seals.sync()
        {
            tracing::error!(%error, realm = %self.realm, "the seals could not be forced to the disk");
        }
        self.journal.shutdown(&self.world);
        self.world.set_clock(None);
        self.world
    }
}
