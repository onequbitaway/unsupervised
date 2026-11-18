//! Organizations, contracts, speech and relationships.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`
//! for the kernel and `docs/design/modules/society.md` for the rules of
//! this module.
//!
//! # Map of the crate
//!
//! - `org`: organizations, roles, grants and treasuries. The registry in
//!   the global realm is the source of truth for memberships; `org::mirror`
//!   is the only code of the game that writes them to the zones.
//! - `speech`: `say`, `message`, `org_say` and `post`.
//! - `contract`: offers, escrow, clauses and the engine that pays, refunds
//!   and punishes by itself.
//! - `family`: private labels, marriage and households.
//! - `rep`: the attribute `rep.standing` and the verb `opine`.
//!
//! # For other modules
//!
//! - [`offices`] fills the government organizations: roles and their
//!   holders. The politics module appoints officials with
//!   [`offices::set_holders`].
//! - [`Orgs`] is the registry, a resource of the global realm, for reads.
//! - [`Contracts`] says how many offers and contracts are open.
//! - [`permissions`] has the permission strings. A zone checks a
//!   permission with `realm.can`; this crate keeps that copy current.

mod contract;
mod family;
mod org;
mod params;
mod pay;
pub mod permissions {
    //! Permission strings and the rules for the powers of the state.
    pub use crate::perms::{
        GOV_APPOINT, ORG_INVITE, ORG_MANAGE, PERMISSIONS, TREASURY_SPEND, holds_by_itself,
        is_government, is_known, is_sovereign, scoped,
    };
}
mod perms;
mod remote;
mod rep;
mod social;
mod speech;
mod text;

use unsup_kernel::{Builder, EventMeta, Kin, Plugin, Scope, VerbSpec};

pub use crate::contract::model::Contracts;
pub use crate::org::offices;
pub use crate::org::state::{Org, Orgs};
pub use crate::social::Social;

/// The society module.
#[derive(Clone, Copy, Debug, Default)]
pub struct SocietyPlugin;

/// The blocking conditions in which a resident can still talk: in jail, under
/// arrest and in hospital.
pub(crate) const TALK_ALLOWED_IN: &[&str] = &["jailed", "detained", "hospitalized"];

/// Makes the description of a verb of this module that occupies the
/// resident.
pub(crate) const fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "society", summary, free: false, allowed_in: &[] }
}

impl Plugin for SocietyPlugin {
    fn name(&self) -> &'static str {
        "society"
    }

    fn build(&self, b: &mut Builder) {
        let p = params::Params::load(b);
        let standing = rep::declare(b);
        b.component::<Kin>();
        b.component::<Social>();
        org::build(b, p);
        speech::build(b, p);
        contract::build(b, p);
        family::build(b, p);
        rep::build(b, p, standing);
        if b.scope() == Scope::Global {
            // A resident that left the world leaves every registry.
            b.on_event("kernel.despawned", move |realm, event| {
                let Some(dead) = event.actor else {
                    return;
                };
                org::lifecycle::on_death(realm, &p, &mut realm.res_mut::<Orgs>(), dead);
                contract::on_death(realm, &p, dead);
                family::on_death(realm, &p, dead);
            });
        }
    }
}

/// Returns the plugin of the society module.
#[must_use]
pub fn plugin() -> SocietyPlugin {
    SocietyPlugin
}

/// Makes the description of a public event without a place.
pub(crate) fn public(kind: &'static str, drama: u8) -> EventMeta {
    EventMeta::public(kind, drama)
}
