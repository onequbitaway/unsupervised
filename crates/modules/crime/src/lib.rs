//! Crime, police, courts and jail.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`.

use unsup_kernel::{Builder, Plugin};

/// The crime module.
#[derive(Clone, Copy, Debug, Default)]
pub struct CrimePlugin;

impl Plugin for CrimePlugin {
    fn name(&self) -> &'static str {
        "crime"
    }

    fn build(&self, _b: &mut Builder) {}
}

/// Returns the plugin of the crime module.
#[must_use]
pub fn plugin() -> CrimePlugin {
    CrimePlugin
}
