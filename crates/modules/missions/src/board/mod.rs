//! Leaderboards: who wins, by resident, by owner, by organization and by
//! declared model.
//!
//! The boards are computed in the global realm. Each zone reports its
//! residents and the sites that residents own on a period
//! (`report.rs`); events feed the counters (`publish.rs`); `score.rs` turns
//! both into a standing. The global realm publishes every board on a period
//! as the event `missions.leaderboard_published`, and the verb
//! `leaderboard` reads a board for one resident.

mod publish;
mod report;
mod score;
mod state;
mod verb;

use unsup_kernel::{Builder, Scope, Stage};

pub(crate) use self::publish::{credit_points, standing};
pub use self::state::{Board, Boards, Entry, Group, Standing};
use crate::params::Params;

/// Registers the reports, the counters, the publication and the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    match b.scope() {
        Scope::Zone => {
            b.resource::<report::Reporter>();
            b.system(Stage::Post, 110, "missions.zone_report", move |realm| {
                report::tick(realm, &p);
            });
        }
        Scope::Global => {
            b.resource::<Boards>();
            b.on_message::<report::ZoneReport>(|realm, from, report| {
                if let Some(zone) = from.zone() {
                    realm.res_mut::<Boards>().apply(zone.0, report);
                }
            });
            b.on_event(publish::ARREST_EVENT, publish::on_arrest);
            b.on_event("kernel.despawned", publish::on_despawned);
            b.system(Stage::Post, 110, "missions.boards_publish", move |realm| {
                publish::tick(realm, &p);
            });
        }
    }
    verb::build(b, p);
}
