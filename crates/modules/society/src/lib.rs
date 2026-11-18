//! Organizations, contracts, speech and relationships.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The society module.
#[derive(Clone, Copy, Debug, Default)]
pub struct SocietyPlugin;

impl Plugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        "society"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the society module.
#[must_use]
pub fn plugin() -> SocietyPlugin {
    SocietyPlugin
}
