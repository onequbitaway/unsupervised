//! What keeps a resident alive when its owner is away.
//!
//! The design asks for two default routines for every new resident: eat
//! when hungry and food is carried, and sleep when tired. This file has
//! three parts:
//!
//! - [`install`] gives a new resident the two routines when it appears
//!   (`Realm::set_routines`). The owner can replace them with the verb
//!   `set_routines`.
//! - [`default_routines`] returns the same two routines for a process
//!   that wants to send them again.
//! - The reflexes do the same work for a resident without routines. In
//!   the hourly pass of the needs, a resident that is idle eats one unit
//!   of carried food when its food is low, and dozes when its energy is
//!   low. Dozing gives less energy than sleep and does not lift the energy
//!   out of the low range, so a resident that only dozes stays alive but
//!   in a bad mood.
//!
//! The parameter `life.spawn_routines` turns the routines of a new
//! resident off. The parameter `life.reflexes` turns the reflexes off.

use unsup_kernel::protocol::{ActionRequest, Condition, Routine};
use unsup_kernel::serde_json::json;
use unsup_kernel::{Blueprint, Idx, Realm};

use crate::eat;
use crate::handles::Handles;
use crate::names::{attr, tag};

/// Greatest number of food items that the routine `eat` can name. The
/// kernel allows 32 parts in one routine condition; three are used by the
/// rest of the condition.
const ROUTINE_FOODS_MAX: usize = 29;

/// Builds the two default routines with their thresholds.
fn routines(defs: &Blueprint, eat_below: i64, sleep_below: i64) -> Vec<Routine> {
    let foods: Vec<Condition> = defs
        .items_with_tag(tag::FOOD)
        .take(ROUTINE_FOODS_MAX)
        .map(|(_, item)| Condition::HasItem { item: item.name.clone(), qty: 1 })
        .collect();
    vec![
        Routine {
            name: "eat when hungry".to_string(),
            when: Condition::All {
                of: vec![
                    Condition::AttrBelow { attr: attr::FOOD.to_string(), value: eat_below },
                    Condition::Any { of: foods },
                ],
            },
            then: ActionRequest::new("eat", json!({})),
        },
        Routine {
            name: "sleep when tired".to_string(),
            when: Condition::AttrBelow { attr: attr::ENERGY.to_string(), value: sleep_below },
            then: ActionRequest::new("sleep", json!({})),
        },
    ]
}

/// Returns the default routines of a resident: eat when hungry and food is
/// carried, and sleep when tired.
///
/// A new resident has them already. A process that wants them back after
/// the owner changed them sends them with the kernel verb `set_routines`:
/// `{ "routines": default_routines(world.blueprint()) }`. The thresholds
/// are the start values of the parameters `life.routine_eat_below` and
/// `life.routine_sleep_below`.
#[must_use]
pub fn default_routines(defs: &Blueprint) -> Vec<Routine> {
    let value = |name: &str| defs.param_id(name).map_or(0, |id| defs.param(id).value);
    routines(defs, value("life.routine_eat_below"), value("life.routine_sleep_below"))
}

/// Gives a new resident the default routines, so that it eats and sleeps
/// when its owner is away. The thresholds are the values of the parameters
/// in the city of the resident.
pub(crate) fn install(realm: &Realm, h: &Handles, idx: Idx) {
    if realm.param(h.p.spawn_routines) == 0 {
        return;
    }
    let list = routines(
        realm.defs(),
        realm.param(h.p.routine_eat_below),
        realm.param(h.p.routine_sleep_below),
    );
    // The kernel refuses the list only when `kernel.routines_max` is below
    // two. The resident then starts without routines and has its reflexes.
    let _refused = realm.set_routines(idx, &list);
}

/// Returns true when a resident does nothing and has nothing planned.
fn is_idle(realm: &Realm, idx: Idx) -> bool {
    realm.activity(idx).is_none() && realm.queue_len(idx) == 0
}

/// Runs the reflexes of the residents that the hourly pass found hungry or
/// tired.
pub(crate) fn run(realm: &Realm, h: &Handles, hungry: &[Idx], tired: &[Idx]) {
    for &idx in hungry {
        if is_idle(realm, idx)
            && let Some(item) = eat::best_food(realm, h, idx)
        {
            // The resident carries the food, so the meal cannot fail.
            let _meal = eat::eat_one(realm, h, idx, item);
        }
    }
    let doze = realm.param(h.p.reflex_doze_energy);
    for &idx in tired {
        if is_idle(realm, idx) {
            realm.attr_add(idx, h.energy, doze);
        }
    }
}
