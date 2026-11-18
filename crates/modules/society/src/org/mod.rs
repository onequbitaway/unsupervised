//! Organizations: one primitive for a company, a gang, a union, a party, a
//! police force and a government.

mod args;
mod create;
pub(crate) mod lifecycle;
pub(crate) mod mirror;
mod observe;
pub mod offices;
pub(crate) mod state;
pub(crate) mod treasury;
pub(crate) mod verbs;

use unsup_kernel::{Builder, Scope, Stage};

use self::state::Orgs;
use crate::params::Params;
use crate::social::HOUR;

/// Registers the registry, the verbs, the observer and world generation.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.resource::<Orgs>();
        b.on_worldgen(0, offices::worldgen);
        // The verbs read the end of an invitation, so an invitation ends in
        // its frame. This only frees the memory.
        b.every(HOUR as u32, Stage::Post, 0, "society.org_invites_expire", |realm| {
            let frame = realm.frame();
            realm.res_mut::<Orgs>().expire_invites(frame);
        });
    }
    create::build(b, p);
    verbs::build(b, p);
    treasury::build(b, p);
    b.observe_global("orgs", 30, move |realm, resident, opts| {
        observe::section(realm, &p, resident, opts)
    });
}
