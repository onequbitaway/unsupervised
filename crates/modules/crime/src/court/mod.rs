//! The court: the case registry of the global realm and everything that
//! it decides.
//!
//! - `registry`: the state.
//! - `intake`: reports of the zones become cases, evidence, warrants and
//!   custody.
//! - `trial`: the charge, the jury and the verdict.
//! - `clerk`: deadlines, and residents that leave the world.
//! - `bounty`: the escrow for arrests.
//! - `verbs`, `docket`: the verbs of officers, judges, jurors and readers.

use unsup_kernel::{Builder, Scope, Stage};
use unsup_society::Orgs;

use crate::ids::Handles;
use crate::msg::Report;

pub(crate) mod bounty;
pub(crate) mod clerk;
pub(crate) mod docket;
pub(crate) mod intake;
pub mod registry;
pub(crate) mod trial;
pub(crate) mod verbs;

/// Registers the registry, its handlers and the verbs of the court.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Global {
        b.resource::<registry::Cases>();
        // The registry of the society module says if a city has a judge.
        // In a world without that module it is empty.
        b.resource::<Orgs>();
        b.on_message::<Report>(move |realm, _from, report| intake::on_report(realm, &h, report));
        b.system(Stage::Update, 0, "crime.clerk", move |realm| clerk::run(realm, &h));
        b.on_event("kernel.despawned", move |realm, event| {
            if let Some(resident) = event.actor {
                clerk::on_despawn(realm, &h, resident);
            }
        });
    }
    bounty::register(b, h);
    verbs::register(b, h);
    docket::register(b, h);
}
