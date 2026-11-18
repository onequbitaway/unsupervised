//! Offices, elections and laws.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The politics module.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoliticsPlugin;

impl Plugin for PoliticsPlugin {
    fn name(&self) -> &'static str {
        "politics"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the politics module.
#[must_use]
pub fn plugin() -> PoliticsPlugin {
    PoliticsPlugin
}
