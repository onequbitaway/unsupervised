//! The durable world of a server: where it is kept, how it starts, and the
//! journal that the runtime writes it with.
//!
//! The runtime has the parts: an input log, snapshots, recovery and
//! rollback (`crates/runtime/README.md`). This module puts them together
//! for the server. It opens the storage of the configuration, recovers
//! the world that the storage holds or makes a new one, and wraps the
//! journal so that the whole world is audited at an interval.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use anyhow::Context;
use serde::Serialize;
use serde_json::{Value, json};
use unsup_gateway::Alerts;
use unsup_kernel::{Input, Plugin, World, WorldConfig};
use unsup_protocol::FrameOutput;
use unsup_runtime::{
    DurableConfig, DurableJournal, FsStorage, Journal, JournalError, JournalStats, MemStorage,
    Plugins, ReplayReport, Rollback, RollbackError, Storage,
};

use crate::config::WorldStore;

/// The directory of the snapshots in the storage of the runtime.
const SNAPSHOT_DIR: &str = "snapshots";
/// The directory of the input log in the storage of the runtime.
const LOG_DIR: &str = "log";

/// Writes a state hash as text.
pub(crate) fn hash_text(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// How the world of this run began.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldStart {
    /// True when the server recovered the world from its data directory.
    /// False for a new world.
    pub recovered: bool,
    /// Number of the last frame of the world at the start.
    pub frame: u64,
    /// The state hash of the world at the start, as hexadecimal text.
    pub state_hash: String,
    /// Frame of the snapshot that the recovery started from.
    pub snapshot_frame: u64,
    /// Inputs of the log that the recovery applied after the snapshot. 0
    /// after a clean stop: its last snapshot holds everything.
    pub replayed_inputs: u64,
    /// How many of those inputs were frames.
    pub replayed_frames: u64,
    /// True when the log ended with a record that a crash cut off. The
    /// recovery ignored it.
    pub torn_tail: bool,
    /// Snapshots that the recovery could not use, with the reason.
    pub skipped_snapshots: Vec<String>,
}

impl WorldStart {
    fn new_world(world: &World) -> WorldStart {
        WorldStart {
            recovered: false,
            frame: world.frame(),
            state_hash: hash_text(&world.state_hash()),
            snapshot_frame: world.frame(),
            replayed_inputs: 0,
            replayed_frames: 0,
            torn_tail: false,
            skipped_snapshots: Vec::new(),
        }
    }

    fn recovered(report: &ReplayReport) -> WorldStart {
        WorldStart {
            recovered: true,
            frame: report.frame,
            state_hash: hash_text(&report.state_hash),
            snapshot_frame: report.snapshot_frame,
            replayed_inputs: report.inputs,
            replayed_frames: report.frames,
            torn_tail: report.torn_tail.is_some(),
            skipped_snapshots: report
                .skipped
                .iter()
                .map(|skipped| format!("{}: {}", skipped.name, skipped.reason))
                .collect(),
        }
    }
}

/// The last audit of the whole world.
#[derive(Debug, Default)]
struct AuditState {
    /// Frame of the last audit.
    frame: u64,
    /// Number of audits.
    audits: u64,
    /// Number of audits that failed.
    failures: u64,
    /// What the last failed audit said.
    last_failure: Option<String>,
    /// Money of the last good audit: held, created, destroyed, in cents.
    money: Option<(i64, i64, i64)>,
    /// Failures that came before the alert list was there.
    waiting: Vec<String>,
    alerts: Option<Arc<Alerts>>,
}

/// Runs the audit of the whole world and keeps the result for the
/// dashboard. A failed audit is an alert.
///
/// Every realm audits its own ledger each tick. This audit is the one
/// that also counts the money and the goods on the way between realms.
#[derive(Debug, Default)]
pub(crate) struct AuditBoard {
    state: Mutex<AuditState>,
}

impl AuditBoard {
    /// Audits the world and notes the result.
    pub(crate) fn run(&self, world: &World) {
        let result = world.audit();
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.frame = world.frame();
        state.audits += 1;
        match result {
            Ok(totals) => {
                let money = totals.money;
                state.money = Some((money.held, money.minted, money.burned));
            }
            Err(error) => {
                let message = format!(
                    "The audit of the world failed at frame {}: {error}. Money or goods do not \
                     add up. Stop the server and run `unsupervised replay`.",
                    world.frame()
                );
                state.failures += 1;
                state.last_failure = Some(message.clone());
                match &state.alerts {
                    Some(alerts) => alerts.raise("audit_failed", message),
                    None => {
                        tracing::error!(%message, "audit failed");
                        state.waiting.push(message);
                    }
                }
            }
        }
    }

    /// Gives the board the alert list. Failures from before go there now.
    pub(crate) fn attach(&self, alerts: Arc<Alerts>) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        for message in state.waiting.drain(..) {
            alerts.raise("audit_failed", message);
        }
        state.alerts = Some(alerts);
    }

    /// Returns the result of the last audit, for the dashboard.
    pub(crate) fn report(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        json!({
            "frame": state.frame,
            "audits": state.audits,
            "failures": state.failures,
            "last_failure": state.last_failure,
            "money": state.money.map(|(held, minted, burned)| json!({
                "held": held, "minted": minted, "burned": burned,
            })),
        })
    }
}

/// The durable journal of the runtime, with an audit of the whole world
/// every `every` frames.
struct AuditedJournal {
    inner: DurableJournal,
    every: u64,
    board: Arc<AuditBoard>,
}

impl Journal for AuditedJournal {
    fn record(&mut self, input: &Input) -> Result<(), JournalError> {
        self.inner.record(input)
    }

    fn frame_done(&mut self, world: &World, frame: &FrameOutput) {
        self.inner.frame_done(world, frame);
        if self.every > 0 && frame.frame.is_multiple_of(self.every) {
            self.board.run(world);
        }
    }

    fn commit(&mut self) -> Result<(), JournalError> {
        self.inner.commit()
    }

    fn discard(&mut self, unapplied: usize) {
        self.inner.discard(unapplied);
    }

    fn shutdown(&mut self, world: &World) {
        // The final snapshot of a clean stop.
        self.inner.shutdown(world);
    }

    fn rollback(&mut self, frame: u64) -> Result<Rollback, RollbackError> {
        self.inner.rollback(frame)
    }

    fn stats(&self) -> JournalStats {
        self.inner.stats()
    }
}

/// A world that is ready for the runtime.
pub(crate) struct OpenedWorld {
    pub(crate) world: World,
    pub(crate) journal: Box<dyn Journal>,
    pub(crate) storage: Arc<dyn Storage>,
    pub(crate) start: WorldStart,
    pub(crate) board: Arc<AuditBoard>,
}

/// Opens the storage of a world. A directory is made when it does not
/// exist.
fn open_storage(store: &WorldStore) -> anyhow::Result<Arc<dyn Storage>> {
    match store {
        WorldStore::Memory => Ok(Arc::new(MemStorage::new())),
        WorldStore::Dir(dir) => {
            let storage = FsStorage::open(dir.clone()).map_err(|error| {
                anyhow::anyhow!("cannot open the world directory {}: {error}", dir.display())
            })?;
            Ok(Arc::new(storage))
        }
    }
}

/// Reads the parameter override files.
pub(crate) fn read_params(files: &[std::path::PathBuf]) -> anyhow::Result<Vec<String>> {
    files
        .iter()
        .map(|file| {
            std::fs::read_to_string(file)
                .with_context(|| format!("cannot read the parameter file {}", file.display()))
        })
        .collect()
}

/// Recovers the world that the storage holds, or makes a new world from
/// `config` when the storage is empty.
pub(crate) fn open(
    store: &WorldStore,
    config: WorldConfig,
    durable: DurableConfig,
    audit_every: u64,
) -> anyhow::Result<OpenedWorld> {
    let storage = open_storage(store)?;
    let plugins: Plugins = Arc::new(crate::plugins());
    let list = |dir: &str| {
        storage.list(dir).map_err(|error| anyhow::anyhow!("cannot read the world storage: {error}"))
    };
    let empty = list(SNAPSHOT_DIR)?.is_empty() && list(LOG_DIR)?.is_empty();
    // A new world is made here, so that a configuration that is not valid
    // is an error and not a panic inside the runtime.
    let fresh = if empty {
        let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
        let world = World::try_new(&refs, config.clone())
            .map_err(|error| anyhow::anyhow!("cannot build the world: {error}"))?;
        Some(world)
    } else {
        None
    };
    let opened = DurableJournal::open(Arc::clone(&storage), plugins, durable, move |plugins| {
        // The runtime asks for a new world only for an empty storage.
        fresh.unwrap_or_else(|| World::new(plugins, config))
    })
    .map_err(|error| anyhow::anyhow!("{error}"))?;

    let start = match &opened.recovery {
        Some(report) => WorldStart::recovered(report),
        None => WorldStart::new_world(&opened.world),
    };
    let board = Arc::new(AuditBoard::default());
    // A world from the disk is audited before anybody uses it.
    if start.recovered {
        board.run(&opened.world);
    }
    let journal =
        AuditedJournal { inner: opened.journal, every: audit_every, board: Arc::clone(&board) };
    Ok(OpenedWorld { world: opened.world, journal: Box::new(journal), storage, start, board })
}

/// Returns the snapshots that the storage holds, oldest first: the frame
/// of each and the number of inputs in it. A rollback can go to each.
pub(crate) fn snapshots(storage: &dyn Storage) -> Vec<Value> {
    // The name of a snapshot is `<sequence number>-<frame>.snap`.
    let parse = |name: &str| {
        let stem = Path::new(name).file_name()?.to_str()?.strip_suffix(".snap")?;
        let (seq, frame) = stem.split_once('-')?;
        Some((seq.parse::<u64>().ok()?, frame.parse::<u64>().ok()?))
    };
    storage
        .list(SNAPSHOT_DIR)
        .unwrap_or_default()
        .iter()
        .filter_map(|name| parse(name))
        .map(|(seq, frame)| json!({ "frame": frame, "inputs": seq }))
        .collect()
}
