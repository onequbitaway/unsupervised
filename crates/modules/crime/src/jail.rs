//! The jail: where the engine holds a resident, and what keeps an inmate
//! alive.
//!
//! The conditions `detained` and `jailed` block every verb that does not
//! list them. An inmate cannot buy food or go to a bed, so the jail feeds
//! it and lets it rest: once in each game hour, an inmate with low food
//! gets a meal and an inmate with low energy gets rest. The meal is not a
//! good of the ledger.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, EventMeta, Idx, Realm, Scope, Stage, Tick};
use unsup_life::names::{TICKS_PER_HOUR, site};

use crate::ids::Handles;
use crate::names::{cond, event, notice};
use crate::places::{LawSite, LawSites};
use crate::powers;

/// Returns the jail of the city of this zone, if the city has one.
pub(crate) fn of_city(realm: &Realm) -> Option<LawSite> {
    realm.res::<LawSites>().jail(powers::city_of(realm))
}

/// Stops what a resident does and moves it into the jail of the city. A
/// resident that is in a jail already stays where it is. In a city
/// without a jail the resident stays in place. Returns the jail.
pub(crate) fn lock_up(realm: &Realm, idx: Idx) -> Option<LawSite> {
    realm.clear_queue(idx);
    realm.interrupt(idx, "You are in custody.");
    let jail = of_city(realm)?;
    let inside = realm.sites().at(realm.pos(idx)).is_some_and(|place| place.kind == site::JAIL);
    if !inside {
        // A resident that leaves the world in this tick cannot move. It
        // stays where it is.
        let _ = realm.set_pos(idx, jail.pos);
    }
    Some(jail)
}

/// Adds jail time to a resident and locks it up. A resident that serves a
/// sentence already serves the new one after it. Returns the tick of this
/// zone at which the resident is free.
pub(crate) fn add_time(realm: &Realm, h: &Handles, idx: Idx, ticks: i64) -> Tick {
    let now = realm.tick();
    let from = match realm.condition_until(idx, h.jailed) {
        Some(Some(until)) if until.0 > now.0 => until,
        _ => now,
    };
    let until = from + u64::try_from(ticks.max(0)).unwrap_or(0);
    realm.set_condition(idx, h.jailed, Some(until));
    lock_up(realm, idx);
    until
}

/// Feeds the inmates of this zone and lets them rest.
fn care(realm: &Realm, h: &Handles) {
    let meal_below = realm.param(h.p.jail_meal_below);
    let meal = realm.param(h.p.jail_meal_food);
    let rest_below = realm.param(h.p.jail_rest_below);
    let rest = realm.param(h.p.jail_rest_energy);
    for idx in realm.indices() {
        if !realm.has_condition(idx, h.jailed) && !realm.has_condition(idx, h.detained) {
            continue;
        }
        if realm.attr(idx, h.food) < meal_below {
            realm.attr_add(idx, h.food, meal);
        }
        if realm.attr(idx, h.energy) < rest_below {
            realm.attr_add(idx, h.energy, rest);
        }
    }
}

/// Registers the care for inmates and the end of a sentence.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() != Scope::Zone {
        return;
    }
    // After the hourly pass of the life module (stage `Pre`), so that an
    // inmate never stays at 0 food for one more hour.
    b.every(TICKS_PER_HOUR, Stage::Update, 0, "crime.jail_care", move |realm| care(realm, &h));
    b.on_event("kernel.condition_ended", move |realm, event| {
        if event.data.get("condition").and_then(|v| v.as_str()) != Some(cond::JAILED) {
            return;
        }
        // Only the end of the time is the end of a sentence.
        if event.data.get("expired").and_then(|v| v.as_bool()) != Some(true) {
            return;
        }
        let Some(resident) = event.actor else {
            return;
        };
        let Some(idx) = realm.idx_of(resident) else {
            return;
        };
        // A resident that waits for another verdict stays under arrest.
        let free = !realm.has_condition(idx, h.detained);
        realm.notify(
            resident,
            notice::RELEASED,
            None,
            json!({ "reason": "sentence_served", "free": free }),
        );
        realm.emit(
            EventMeta::public(event::SERVED, h.drama(realm, h.p.drama_served))
                .actor(resident)
                .at(realm.pos(idx)),
            &json!({ "resident": resident, "free": free }),
        );
    });
}
