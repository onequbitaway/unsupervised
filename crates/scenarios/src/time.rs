//! Time: frames, the calendar and the audit of every frame.

use unsup_kernel::protocol::Clock;
use unsup_kernel::{TICKS_PER_DAY, WorldTotals};

use crate::game::Game;

impl Game {
    /// Runs frames. After every frame the whole world is audited: money and
    /// goods are only made and destroyed by faucets and sinks.
    ///
    /// # Panics
    ///
    /// Panics when the audit fails.
    pub fn step(&mut self, frames: u64) {
        for _ in 0..frames {
            let frame = self.world.world.step();
            self.events.extend(frame.events);
            if let Err(e) = self.world.world.audit() {
                panic!("the audit failed after frame {}: {e}", self.frame());
            }
            if let Some(every) = self.restore_every
                && self.frame().is_multiple_of(every)
            {
                self.world.snapshot_restore();
            }
        }
    }

    /// Returns the number of frames that ran.
    #[must_use]
    pub fn frame(&self) -> u64 {
        self.world.world.frame()
    }

    /// Returns the calendar time.
    #[must_use]
    pub fn clock(&self) -> Clock {
        self.world.world.layout().clock_at(self.frame())
    }

    /// Returns the day of the world, from 1.
    #[must_use]
    pub fn day(&self) -> u64 {
        self.frame() / TICKS_PER_DAY + 1
    }

    /// Returns the frame at which a time of a day of the world starts. Days
    /// count from 1.
    #[must_use]
    pub fn frame_at(day: u64, hour: u64, minute: u64) -> u64 {
        (day - 1) * TICKS_PER_DAY + hour * 60 + minute
    }

    /// Runs frames until a frame number.
    ///
    /// # Panics
    ///
    /// Panics when that frame is in the past.
    pub fn run_to_frame(&mut self, frame: u64) {
        let now = self.frame();
        assert!(frame >= now, "frame {frame} is in the past: the world is at frame {now}");
        self.step(frame - now);
    }

    /// Runs frames until a time of a day of the world. Days count from 1.
    pub fn run_to(&mut self, day: u64, hour: u64, minute: u64) {
        self.run_to_frame(Game::frame_at(day, hour, minute));
    }

    /// Audits the whole world and returns the totals.
    ///
    /// # Panics
    ///
    /// Panics when money or goods were made or lost outside faucets and
    /// sinks.
    pub fn audit(&self) -> WorldTotals {
        self.world.assert_conserved()
    }

    /// Returns the state hash as hexadecimal text.
    #[must_use]
    pub fn hash(&self) -> String {
        self.world.hash()
    }
}
