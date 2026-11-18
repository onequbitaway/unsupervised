//! Jobs, firms, production, shops, property and taxes.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The economy module.
#[derive(Clone, Copy, Debug, Default)]
pub struct EconomyPlugin;

impl Plugin for EconomyPlugin {
    fn name(&self) -> &'static str {
        "economy"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the economy module.
#[must_use]
pub fn plugin() -> EconomyPlugin {
    EconomyPlugin
}
