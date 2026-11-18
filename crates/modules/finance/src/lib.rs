//! Bank, loans, the exchange, shares, the central bank and statistics.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The finance module.
#[derive(Clone, Copy, Debug, Default)]
pub struct FinancePlugin;

impl Plugin for FinancePlugin {
    fn name(&self) -> &'static str {
        "finance"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the finance module.
#[must_use]
pub fn plugin() -> FinancePlugin {
    FinancePlugin
}
