//! Extension points of the runtime: the journal and the pacer.

use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Input, World, ZoneTiming};
use unsup_protocol::{FrameOutput, SimError, ZoneId};

/// The journal could not record an input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalError {
    /// What went wrong, for the operator.
    pub message: String,
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the journal cannot record the input: {}", self.message)
    }
}

impl std::error::Error for JournalError {}

/// Numbers that a journal reports for [`SimStats`](crate::SimStats).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalStats {
    /// Inputs in the log since the world was made.
    pub records: u64,
    /// Inputs that the operating system has but that are not forced to the
    /// disk yet. A loss of power now would lose them.
    pub lag: u64,
    /// Writes and syncs of the log that failed.
    pub write_errors: u64,
    /// Snapshots written by this journal.
    pub snapshots: u64,
    /// Snapshots that could not be written.
    pub snapshot_errors: u64,
    /// Frame of the last snapshot that is on the disk. 0 when there is
    /// none yet.
    pub last_snapshot_frame: u64,
    /// Microseconds that the simulation thread stood still to take the last
    /// snapshot.
    pub last_snapshot_capture_micros: u64,
    /// Microseconds that the background thread needed to write the last
    /// snapshot to the disk.
    pub last_snapshot_write_micros: u64,
    /// Size of the last snapshot file in bytes.
    pub last_snapshot_bytes: u64,
}

/// A rollback that a journal did: the world of an earlier snapshot.
#[derive(Debug)]
pub struct Rollback {
    /// The world of the snapshot. The runtime goes on with it.
    pub world: World,
    /// Frame of the snapshot.
    pub frame: u64,
    /// Inputs after the snapshot. They are no longer in the log.
    pub dropped_inputs: u64,
}

/// A rollback was refused or failed. The world is unchanged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RollbackError {
    /// The runtime has no journal that keeps snapshots.
    Unsupported,
    /// No usable snapshot is at or before the frame.
    NoSnapshot {
        /// The frame that was asked for.
        frame: u64,
        /// Frame of the oldest snapshot that exists, when there is one.
        oldest: Option<u64>,
    },
    /// The storage or the snapshot failed.
    Failed {
        /// What went wrong, for the operator.
        message: String,
    },
    /// The call did not reach the simulation.
    Sim(SimError),
}

impl fmt::Display for RollbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RollbackError::Unsupported => {
                f.write_str("the runtime keeps no snapshots, so it cannot roll back")
            }
            RollbackError::NoSnapshot { frame, oldest: Some(oldest) } => write!(
                f,
                "no snapshot is at or before frame {frame}; the oldest snapshot is at frame {oldest}"
            ),
            RollbackError::NoSnapshot { frame, oldest: None } => {
                write!(f, "no snapshot is at or before frame {frame}; there is no snapshot")
            }
            RollbackError::Failed { message } => write!(f, "the rollback failed: {message}"),
            RollbackError::Sim(error) => write!(f, "the rollback did not start: {error}"),
        }
    }
}

impl std::error::Error for RollbackError {}

/// Sees every input before it is applied, and the world after every frame.
///
/// This is where the append-only input log and the periodic snapshots plug
/// in; [`DurableJournal`](crate::DurableJournal) is that implementation.
/// The runtime calls the journal on the simulation thread, so an
/// implementation must not block for long; hand slow work (disk, network)
/// to another thread.
///
/// The runtime calls the methods in this order:
///
/// 1. [`record`](Journal::record) for each input of a batch;
/// 2. [`commit`](Journal::commit) once;
/// 3. `World::apply` for each input of the batch.
///
/// Only `record` and `frame_done` are required. The other methods have
/// defaults that do nothing, so a simple journal stays simple.
pub trait Journal: Send + 'static {
    /// Records one input. The runtime calls it before `World::apply`.
    ///
    /// When it fails the input is not applied: the caller gets
    /// `SimError::Overloaded`, and a frame is skipped and tried again at the
    /// next interval. So the log never lacks an input that the world has.
    fn record(&mut self, input: &Input) -> Result<(), JournalError>;

    /// Called after every frame, with the world as it is after the frame. A
    /// snapshot every N frames goes here: `world.snapshot()`.
    fn frame_done(&mut self, world: &World, frame: &FrameOutput);

    /// Makes the inputs that were recorded since the last call as durable
    /// as the journal promises. The runtime calls it after the `record`
    /// calls of a batch and before it applies the first input of the batch.
    ///
    /// When it fails, none of those inputs is applied and none must stay in
    /// the log: every caller of the batch gets `SimError::Overloaded`.
    fn commit(&mut self) -> Result<(), JournalError> {
        Ok(())
    }

    /// Called when the simulation panicked. The last `unapplied` inputs
    /// that were recorded did not change the world: the first of them
    /// caused the panic and the others were never tried. A journal takes
    /// them out of its log, so that a recovery does not panic again.
    fn discard(&mut self, unapplied: usize) {
        let _ = unapplied;
    }

    /// Called once when the runtime stops in order, with the final world. A
    /// journal writes its last snapshot here. It is not called after a
    /// panic.
    fn shutdown(&mut self, world: &World) {
        let _ = world;
    }

    /// Returns the world of the newest snapshot at or before `frame` and
    /// forgets every later input. When it fails, nothing changed.
    fn rollback(&mut self, frame: u64) -> Result<Rollback, RollbackError> {
        let _ = frame;
        Err(RollbackError::Unsupported)
    }

    /// Returns numbers about the log and the snapshots.
    fn stats(&self) -> JournalStats {
        JournalStats::default()
    }
}

/// A journal that records nothing. The default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoJournal;

impl Journal for NoJournal {
    fn record(&mut self, _input: &Input) -> Result<(), JournalError> {
        Ok(())
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

/// Decides which zones skip the next frame (time dilation).
///
/// The runtime gives the pacer the tick time of every zone in the last
/// frame, in nanoseconds, and the frame interval. The zones that the pacer
/// returns do not tick in the next frame; the list is the `skip` field of
/// the `Input::Step` that the journal records.
/// [`Dilation`](crate::Dilation) is the implementation for production.
pub trait Pacer: Send + 'static {
    /// Returns the zones that skip the next frame.
    fn skip(&mut self, timings: &[ZoneTiming], tick_interval: Duration) -> Vec<ZoneId>;

    /// Returns the number of zones that run slower than one tick per frame
    /// now. The default is 0.
    fn dilated_zones(&self) -> usize {
        0
    }
}

/// A pacer that never skips a zone. The default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDilation;

impl Pacer for NoDilation {
    fn skip(&mut self, _timings: &[ZoneTiming], _tick_interval: Duration) -> Vec<ZoneId> {
        Vec::new()
    }
}
