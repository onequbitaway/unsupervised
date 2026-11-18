//! The real-time driver of an Unsupervised world.
//!
//! [`start`] moves a [`World`](unsup_kernel::World) to one dedicated OS
//! thread and returns a [`SimHandle`]. The thread owns the world and reads
//! a mailbox; nothing else touches simulation state. The gateway and the
//! spectator service talk to the world only through the handle.
//!
//! The simulation thread:
//!
//! - takes calls from the mailbox in batches, gives their inputs to the
//!   [`Journal`], and applies them through `World::apply` after the journal
//!   committed them;
//! - steps one frame per `tick_interval`, or only on [`SimHandle::step`]
//!   when the interval is zero;
//! - asks the [`Pacer`] which zones skip the next frame;
//! - sends every frame to the subscribers of [`SimHandle::frames`];
//! - wakes [`SimHandle::wait`] calls when their resident gets a notice or
//!   finishes an action.
//!
//! # Durability
//!
//! [`DurableJournal`] is the journal for production. It writes every input
//! to an append-only log before the world applies it and takes a snapshot
//! every N frames. [`DurableJournal::open`] recovers the world after a stop
//! of any kind: the newest snapshot plus the log after it give the state
//! that the world had, with the same state hash.
//!
//! ```no_run
//! use std::sync::Arc;
//! use unsup_kernel::{Plugin, World, WorldConfig};
//! use unsup_runtime::{Dilation, DilationConfig, DurableConfig, DurableJournal};
//! use unsup_runtime::{FsStorage, RuntimeConfig, start};
//!
//! # fn plugins() -> Vec<Box<dyn Plugin>> { Vec::new() }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let storage = Arc::new(FsStorage::open("var/world")?);
//! let opened = DurableJournal::open(
//!     storage,
//!     Arc::new(plugins()),
//!     DurableConfig::default(),
//!     |plugins| World::new(plugins, WorldConfig::with_seed(1)),
//! )?;
//! let config = RuntimeConfig {
//!     journal: Box::new(opened.journal),
//!     pacer: Box::new(Dilation::new(DilationConfig::default())),
//!     ..RuntimeConfig::default()
//! };
//! let (handle, join) = start(opened.world, config);
//! # drop(handle);
//! // ... serve agents and viewers through `handle` ...
//! join.shutdown()?; // writes the final snapshot
//! # Ok(())
//! # }
//! ```
//!
//! The parts:
//!
//! - [`storage`]: named blobs and append-only segments, on the local file
//!   system or in memory.
//! - [`log`]: the format of the input log and the one rule that covers a
//!   crash, a failed write and a rollback.
//! - [`snapshot`]: the format of a snapshot file, and two reads for a
//!   host: the list of the snapshots, and the frame and state hash of the
//!   newest one.
//! - [`recover`] and [`replay`]: from storage back to a world, and the
//!   check that a log gives the recorded state hash.
//! - [`Dilation`]: the pacer that makes overloaded zones skip frames.
//! - [`SimHandle::rollback`]: back to an earlier snapshot, for the whole
//!   world. One zone alone cannot go back: money and goods cross zone
//!   borders, so the totals of the world would no longer balance. The
//!   runtime refuses `AdminCommand::RollbackZone` and says so.
//! - [`SimHandle::post_global`]: a typed message from a service outside
//!   the simulation to the global realm. It is an input like any other, so
//!   it is in the log and a replay delivers it again. A message whose kind
//!   no module handles is refused before it is in the log.
//! - [`cluster`]: the same world with its zones spread over several
//!   processes. A coordinator hosts the global realm and routes every
//!   message between realms; workers host the zones. A
//!   [`cluster::ClusterHandle`] is a [`SimHandle`].
//!
//! `README.md` in the crate directory describes the formats, the rules and
//! the limits. The examples `replay`, `throughput` and `cluster` show the
//! replay check, the load behaviour and a cluster of processes.
//!
//! # Extension points
//!
//! - [`Journal`] sees every input before it is applied
//!   ([`Journal::record`], [`Journal::commit`]) and the world after every
//!   frame ([`Journal::frame_done`]). The default, [`NoJournal`], records
//!   nothing.
//! - [`Pacer`] decides the `skip` list of every frame from the measured
//!   zone tick times. The decision is part of the `Input::Step` that the
//!   journal records, so a replay makes the same decision. The default,
//!   [`NoDilation`], never skips.
//!
//! # Load
//!
//! The mailbox is bounded. A call that finds it full fails at once with
//! `SimError::Overloaded`; it never blocks the caller. The number of
//! waiting [`SimHandle::wait`] calls is bounded too, and the cost of a
//! frame does not grow with it. [`SimStats`] has the numbers to watch.

pub mod cluster;
mod config;
mod dilation;
mod durable;
mod external;
mod handle;
mod join;
mod journal;
pub mod log;
mod mailbox;
mod observe;
mod replay;
mod replier;
mod settings;
mod sim;
pub mod snapshot;
mod stats;
pub mod storage;
mod waiters;

pub use crate::config::RuntimeConfig;
pub use crate::dilation::{Dilation, DilationConfig};
pub use crate::durable::{DurableConfig, DurableJournal, OpenError, Opened, Plugins, SyncPolicy};
pub use crate::external::PostError;
pub use crate::handle::{RollbackReport, SimHandle};
pub use crate::join::{RuntimeJoin, RuntimePanic, start};
pub use crate::journal::{
    Journal, JournalError, JournalStats, NoDilation, NoJournal, Pacer, Rollback, RollbackError,
};
pub use crate::log::{LogError, TornTail};
pub use crate::replay::{
    Checkpoint, RecoverError, RecoverOptions, Recovery, ReplayFrom, ReplayOptions, ReplayReport,
    SkippedSnapshot, recover, recover_with, replay,
};
pub use crate::snapshot::{SnapshotEntry, SnapshotMeta};
pub use crate::stats::SimStats;
pub use crate::storage::{
    Fault, FsStorage, MemStorage, Segment, Storage, StorageError, StorageErrorKind,
};
