//! How the runtime drives the world.

use std::fmt;
use std::time::Duration;

use crate::journal::{Journal, NoDilation, NoJournal, Pacer};
use crate::settings::settings;

/// How the runtime drives the world.
///
/// The defaults come from `data/runtime.toml`. Build a value with the
/// struct update syntax: `RuntimeConfig { batch: 1_024,
/// ..RuntimeConfig::default() }`.
pub struct RuntimeConfig {
    /// Real time between two frames. Zero means manual stepping: frames
    /// advance only through [`SimHandle::step`](crate::SimHandle::step).
    /// The default is one second.
    pub tick_interval: Duration,
    /// Greatest number of calls that wait in the mailbox of the simulation
    /// thread. A call that finds the mailbox full fails with
    /// `SimError::Overloaded`.
    pub mailbox: usize,
    /// Greatest number of calls that the simulation thread takes from the
    /// mailbox in a row. The journal commits their inputs in one step, then
    /// the thread applies them and checks whether a frame is due.
    pub batch: usize,
    /// Number of frames that a slow subscriber of
    /// [`SimHandle::frames`](crate::SimHandle::frames) can fall behind
    /// before it loses frames.
    pub frame_buffer: usize,
    /// Sees every input before it is applied and the world after every
    /// frame. See the [crate documentation](crate#extension-points).
    pub journal: Box<dyn Journal>,
    /// Decides which zones skip a frame.
    pub pacer: Box<dyn Pacer>,
    /// Greatest number of [`SimHandle::wait`](crate::SimHandle::wait) calls
    /// that wait at the same time. One more call fails with
    /// `SimError::Overloaded`.
    pub max_waiters: usize,
    /// Greatest number of waiting calls for one resident. One more call
    /// ends the oldest one, as if its time were up.
    pub max_waiters_per_resident: usize,
    /// Number of frames in the window of the frame time percentiles of
    /// [`SimStats`](crate::SimStats).
    pub stats_window: usize,
    /// Length of the window of `SimStats::inputs_per_second`.
    pub rate_window: Duration,
    /// A batch with this many answers or more is handed to the reply
    /// thread, so that the simulation thread can take the next batch at
    /// once. Smaller batches are answered by the simulation thread itself,
    /// which is faster when the load is light.
    pub reply_handoff: usize,
    /// Greatest number of batches of answers that wait for the reply
    /// thread. When it is reached, the simulation thread waits.
    pub reply_queue: usize,
}

impl Default for RuntimeConfig {
    /// The numbers of `data/runtime.toml`, no journal and no time dilation.
    fn default() -> RuntimeConfig {
        let s = &settings().runtime;
        RuntimeConfig {
            tick_interval: Duration::from_millis(s.tick_interval_ms),
            mailbox: s.mailbox,
            batch: s.batch,
            frame_buffer: s.frame_buffer,
            journal: Box::new(NoJournal),
            pacer: Box::new(NoDilation),
            max_waiters: s.max_waiters,
            max_waiters_per_resident: s.max_waiters_per_resident,
            stats_window: s.stats_window_frames,
            rate_window: Duration::from_millis(s.rate_window_ms),
            reply_handoff: s.reply_handoff,
            reply_queue: s.reply_queue,
        }
    }
}

impl RuntimeConfig {
    /// Makes a configuration for tests and tools: frames advance only
    /// through [`SimHandle::step`](crate::SimHandle::step).
    #[must_use]
    pub fn manual() -> RuntimeConfig {
        RuntimeConfig { tick_interval: Duration::ZERO, ..RuntimeConfig::default() }
    }

    /// Makes a configuration that steps one frame per `tick_interval`.
    #[must_use]
    pub fn real_time(tick_interval: Duration) -> RuntimeConfig {
        RuntimeConfig { tick_interval, ..RuntimeConfig::default() }
    }
}

impl fmt::Debug for RuntimeConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeConfig")
            .field("tick_interval", &self.tick_interval)
            .field("mailbox", &self.mailbox)
            .field("batch", &self.batch)
            .field("frame_buffer", &self.frame_buffer)
            .field("max_waiters", &self.max_waiters)
            .field("max_waiters_per_resident", &self.max_waiters_per_resident)
            .field("stats_window", &self.stats_window)
            .field("rate_window", &self.rate_window)
            .field("reply_handoff", &self.reply_handoff)
            .field("reply_queue", &self.reply_queue)
            .finish_non_exhaustive()
    }
}
