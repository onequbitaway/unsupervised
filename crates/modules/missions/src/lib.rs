//! Missions, seasons and leaderboards.
//!
//! This crate is a plugin for the kernel. See `docs/design/architecture.md`
//! for the kernel and `docs/design/modules/missions.md` for the rules of
//! this module.
//!
//! # Map of the crate
//!
//! - `spec`, `check`: the definition of a mission as data, and the checks.
//! - `model`: the open missions and their takers, in the global realm.
//! - `post`, `sponsor`: how a resident and a human sponsor publish.
//! - `accept`, `claim`, `list`: the other verbs.
//! - `remote`, `interrupted`: verbs that start in a zone and end in the
//!   global realm, and what a resident learns when one is interrupted.
//! - `engine`, `lifecycle`: count events, pay takers, close missions, and
//!   handle a resident that goes.
//! - `board`: the leaderboards.
//! - `season`: seasons, the hall of fame and the soft reset.
//!
//! # For other crates
//!
//! - The server sends a [`SponsorMission`] to the global realm with
//!   `SimHandle::send_global` when a human sponsor publishes a mission.
//! - [`Missions`], [`Boards`] and [`Seasons`] are resources of the global
//!   realm, for reads.

mod accept;
mod board;
mod check;
mod claim;
mod engine;
mod interrupted;
mod lifecycle;
mod list;
mod model;
mod observe;
mod params;
mod pay;
mod post;
mod remote;
mod season;
mod spec;
mod sponsor;
mod text;
mod view;

use unsup_kernel::{Builder, Plugin, Scope, Stage, VerbSpec};

pub use crate::board::{Board, Boards, Entry, Group, Standing};
pub use crate::model::Missions;
pub use crate::params::{NOTORIETY, SEASON_SINK, SPONSOR_FAUCET, SPONSOR_SINK, STANDING};
pub use crate::season::{SeasonMark, SeasonRecord, Seasons};
pub use crate::spec::{ActorRule, MissionSpec, Mode, ObjectiveSpec};
pub use crate::sponsor::SponsorMission;

/// The missions module.
#[derive(Clone, Copy, Debug, Default)]
pub struct MissionsPlugin;

/// The blocking conditions in which a resident can still read missions and
/// boards: in jail, under arrest and in hospital.
const READ_ALLOWED_IN: &[&str] = &["jailed", "detained", "hospitalized"];

/// Makes the description of a verb of this module that occupies the
/// resident.
pub(crate) const fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "missions", summary, free: false, allowed_in: &[] }
}

/// Makes the description of a verb of this module that only reads. It is
/// free, and it works in jail and in hospital.
pub(crate) const fn read_spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "missions", summary, free: true, allowed_in: READ_ALLOWED_IN }
}

impl Plugin for MissionsPlugin {
    fn name(&self) -> &'static str {
        "missions"
    }

    fn build(&self, b: &mut Builder) {
        let p = params::Params::load(b);
        if b.scope() == Scope::Global {
            b.resource::<Missions>();
            // These come before the handlers of the engine, so that a
            // resident that goes is out of its missions first.
            b.on_event("kernel.despawned", lifecycle::on_despawned);
            b.on_event("life.resident_died", lifecycle::on_died);
            b.on_event("society.org_left", lifecycle::on_org_left);
            for prefix in check::WATCHED {
                b.on_event(prefix, move |realm, event| engine::on_event(realm, &p, event));
            }
            b.on_message::<SponsorMission>(move |realm, _from, message| {
                sponsor::on_message(realm, &p, &message);
            });
            b.system(Stage::Update, 0, "missions.deadlines", move |realm| {
                engine::tick(realm, &p);
            });
            b.system(Stage::Update, 10, "missions.seasons", move |realm| {
                season::tick(realm, &p);
            });
        }
        post::build(b, p);
        accept::build(b, p);
        claim::build(b, p);
        list::build(b, p);
        interrupted::build(b);
        board::build(b, p);
        season::build(b, p);
        b.observe_global("missions", 60, move |realm, resident, opts| {
            observe::section(realm, &p, resident, opts)
        });
    }
}

/// Returns the plugin of the missions module.
#[must_use]
pub fn plugin() -> MissionsPlugin {
    MissionsPlugin
}
