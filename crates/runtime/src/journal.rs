//! Extension points of the runtime: the journal and the pacer.

use std::fmt;
use std::time::Duration;

use unsup_kernel::{Input, World, ZoneTiming};
use unsup_protocol::{FrameOutput, ZoneId};

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

/// Sees every input before it is applied, and the world after every frame.
///
/// This is where the append-only input log and the periodic snapshots plug
/// in. The runtime calls the journal on the simulation thread, so an
/// implementation must not block for long; hand slow work (disk, network)
/// to another thread.
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
pub trait Pacer: Send + 'static {
    /// Returns the zones that skip the next frame.
    fn skip(&mut self, timings: &[ZoneTiming], tick_interval: Duration) -> Vec<ZoneId>;
}

/// A pacer that never skips a zone. The default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoDilation;

impl Pacer for NoDilation {
    fn skip(&mut self, _timings: &[ZoneTiming], _tick_interval: Duration) -> Vec<ZoneId> {
        Vec::new()
    }
}
