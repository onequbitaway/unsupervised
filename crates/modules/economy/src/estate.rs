//! What happens to the ties of a resident that defaults on a loan or
//! leaves the world.
//!
//! The events come to the global realm. Sites live in zones, so the global
//! realm tells every zone, and each zone changes its own sites.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Dest, Event, Holder, Message, Realm, ResidentId, Resource, SiteId, Value};

use crate::events;
use crate::hiring;
use crate::ownership;
use crate::site;
use crate::tenancy;

/// The event of the finance module that starts a foreclosure.
pub const LOAN_DEFAULTED: &str = "finance.loan_defaulted";
/// The event of the life module for a death, with the heir.
pub const RESIDENT_DIED: &str = "life.resident_died";
/// The event of the kernel for a resident that left the world.
pub const DESPAWNED: &str = "kernel.despawned";

/// The city takes every site of a resident.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Foreclose {
    resident: ResidentId,
}

impl Message for Foreclose {
    const KIND: &'static str = "economy.foreclose";
}

/// A resident left the world. Its sites go to the heir, or to the city.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Estate {
    resident: ResidentId,
    heir: Option<Holder>,
}

impl Message for Estate {
    const KIND: &'static str = "economy.estate";
}

/// Residents that the kernel removed in this frame, in the global realm.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Departed {
    pending: Vec<ResidentId>,
}

impl Resource for Departed {
    const NAME: &'static str = "economy.departed";
}

fn resident_of(event: &Event) -> Option<ResidentId> {
    event.data.get("resident").and_then(Value::as_u64).map(ResidentId)
}

/// Global handler of `finance.loan_defaulted`.
pub(crate) fn on_loan_defaulted(realm: &Realm, event: &Event) {
    if let Some(resident) = resident_of(event) {
        realm.send(Dest::AllZones, &Foreclose { resident });
    }
}

/// Global handler of `life.resident_died`: the sites go to the heir that
/// the life module found.
pub(crate) fn on_resident_died(realm: &Realm, event: &Event) {
    let Some(resident) = resident_of(event) else {
        return;
    };
    let heir = event
        .data
        .get("heir")
        .and_then(|value| serde_json::from_value::<Option<Holder>>(value.clone()).ok())
        .flatten();
    realm.send(Dest::AllZones, &Estate { resident, heir });
}

/// Global handler of `kernel.despawned`. The estate waits for the system
/// below, so that the estate of a `life.resident_died` of the same frame
/// goes out first, whatever the order of the two events.
pub(crate) fn on_despawned(realm: &Realm, event: &Event) {
    if let Some(resident) = resident_of(event) {
        realm.res_mut::<Departed>().pending.push(resident);
    }
}

/// Global system: what a resident that left the world still owns has no
/// heir. After a death the zones get the estate with the heir first, so
/// this estate finds no site.
pub(crate) fn settle_departed(realm: &Realm) {
    let pending = std::mem::take(&mut realm.res_mut::<Departed>().pending);
    for resident in pending {
        realm.send(Dest::AllZones, &Estate { resident, heir: None });
    }
}

fn sites_of(realm: &Realm, resident: ResidentId) -> Vec<SiteId> {
    realm.sites().owned_by(Holder::Resident(resident)).map(|s| s.id).collect()
}

/// Zone handler of [`Foreclose`].
pub(crate) fn on_foreclose(realm: &Realm, order: &Foreclose) {
    for id in sites_of(realm, order.resident) {
        ownership::seize(realm, id, "loan_default");
    }
}

/// Zone handler of [`Estate`].
pub(crate) fn on_estate(realm: &Realm, estate: &Estate) {
    let city = site::city_holder(realm);
    let heir = match estate.heir {
        Some(Holder::Resident(heir)) if heir != estate.resident => Some(Holder::Resident(heir)),
        Some(Holder::Org(org)) => Some(Holder::Org(org)),
        _ => None,
    };
    let to = heir.unwrap_or(city);
    for id in sites_of(realm, estate.resident) {
        let pos = hiring::site_pos(realm, id);
        if site::is_government(Some(to)) {
            // Nobody runs the business of a site that falls to a government.
            ownership::close_business(realm, id, "estate");
        }
        ownership::transfer(realm, id, to);
        events::property_inherited(realm, id, pos, estate.resident, to);
    }
    tenancy::end_everywhere(realm, estate.resident);
    hiring::remove_everywhere(realm, estate.resident);
}
