//! Missions, seasons and leaderboards.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The missions module.
#[derive(Clone, Copy, Debug, Default)]
pub struct MissionsPlugin;

impl Plugin for MissionsPlugin {
    fn name(&self) -> &'static str {
        "missions"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the missions module.
#[must_use]
pub fn plugin() -> MissionsPlugin {
    MissionsPlugin
}
