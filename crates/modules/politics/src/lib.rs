//! Offices, elections and laws.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`
//! for the kernel and `docs/design/modules/politics.md` for the calendar
//! and the rules of this module.
//!
//! Residents govern themselves. Offices are won in elections on a fixed
//! calendar. Officials change the rules of the world, because a law is
//! data: it sets a policy parameter, and the simulation obeys.
//!
//! # Map of the crate
//!
//! - `offices`: every office and post as a role of a government
//!   organization. The society module owns organizations and memberships;
//!   this module changes them only through `unsup_society::offices`.
//! - `calendar`: voting days and the windows of an election.
//! - `election`: `register_candidate`, `donate`, `campaign`, `vote`, the
//!   calendar system and the count with its seeded tie-break.
//! - `law`: `propose_law`, `vote_law`, `sign_law`, `veto_law` and the
//!   enactment with `Realm::set_param`.
//! - `appoint`: `appoint` and `dismiss`.
//! - `recall`: `start_recall` and `sign_recall`.
//! - `salary`: the daily pay of the officials from the treasuries.
//! - `publish`: public state for viewers as `_published` events.
//! - `observe`: the observation section `politics`.
//! - `estate`: what happens when a resident leaves the world.
//! - `interrupted`: the notice for a request that is still handled after
//!   its action stopped.
//!
//! All state of the module is one resource of the global realm. The
//! holders of the offices are in the registry of the society module.
//!
//! # For other modules
//!
//! - [`roles`] has the names of the roles. With
//!   `unsup_society::offices::holders(realm, OrgId::city(c), roles::JUDGE)`
//!   another module reads who holds a post.
//! - A zone checks a power with `realm.can(idx, OrgId::city(c), "law.arrest")`.
//! - The events of the module carry flat data, so that a contract clause
//!   of the society module can wait for them.
//!
//! The world must load the society module. It makes the government
//! organizations that this module fills.

mod appoint;
mod calendar;
mod election;
mod estate;
mod interrupted;
mod law;
mod model;
mod observe;
mod offices;
mod params;
mod pay;
mod publish;
mod recall;
mod remote;
mod salary;
mod state;
mod text;

use unsup_kernel::{Builder, Plugin, Scope};

use crate::params::Params;
use crate::state::Politics;

pub mod roles {
    //! The names of the roles that this module defines in the government
    //! organizations. `MAYOR`, `COUNCILLOR`, `POLICE_CHIEF`,
    //! `POLICE_OFFICER` and `JUDGE` are roles of `OrgId::city(c)`.
    //! `PRESIDENT` and `LEGISLATOR` are roles of `OrgId::NATION`.
    //! `CENTRAL_BANKER` is a role of `OrgId::CENTRAL_BANK`.
    pub use crate::offices::{
        CENTRAL_BANKER, COUNCILLOR, JUDGE, LEGISLATOR, MAYOR, POLICE_CHIEF, POLICE_OFFICER,
        PRESIDENT,
    };
}

/// Order of the world generation hook. The society module makes the
/// government organizations with order 0; this module fills them after.
const WORLDGEN_ORDER: i32 = 10;

/// The politics module.
#[derive(Clone, Copy, Debug, Default)]
pub struct PoliticsPlugin;

impl Plugin for PoliticsPlugin {
    fn name(&self) -> &'static str {
        remote::MODULE
    }

    fn build(&self, b: &mut Builder) {
        let p = Params::load(b);
        if b.scope() == Scope::Global {
            b.resource::<Politics>();
            b.on_worldgen(WORLDGEN_ORDER, move |realm, _context| {
                election::schedule::ensure_ready(realm, &p, &mut realm.res_mut::<Politics>());
            });
        }
        election::build(b, p);
        law::build(b, p);
        appoint::build(b, p);
        recall::build(b, p);
        salary::build(b, p);
        estate::build(b);
        interrupted::build(b);
        publish::build(b, p);
        observe::build(b, p);
    }
}

/// Returns the plugin of the politics module.
#[must_use]
pub fn plugin() -> PoliticsPlugin {
    PoliticsPlugin
}
