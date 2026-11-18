//! Speech: `say`, `message`, `org_say` and `post`.
//!
//! Text of a resident never travels in a plain string field. Every notice
//! and every event of this module carries it as `Untrusted`.

mod args;
mod mail;
pub(crate) mod meter;
mod talk;

use unsup_kernel::Builder;

use crate::params::Params;

/// Registers the speech verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    talk::build(b, p);
    mail::build(b, p);
}
