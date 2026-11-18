//! Money that a contract owes to a resident that left the world.
//!
//! The life module says who inherits with the event `life.resident_died`
//! (`docs/design/game-design.md`, section 9). The engine remembers the heir
//! of a resident that still has contracts, and pays the heir. Without an
//! heir the kernel puts the money on the unclaimed account.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Event, Holder, Realm, ResidentId, Resource};

use super::model::Contracts;
use crate::pay;

/// The heirs of the residents that left the world with open contracts. A
/// resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Estates {
    heirs: BTreeMap<ResidentId, Holder>,
}

impl Resource for Estates {
    const NAME: &'static str = "society.estates";
}

/// Reads `life.resident_died`: remembers the heir when the dead resident
/// has an offer or a contract.
pub(crate) fn on_died(realm: &Realm, event: &Event) {
    let Some(dead) = event.data["resident"].as_u64().map(ResidentId) else {
        return;
    };
    let Ok(heir) = serde_json::from_value::<Holder>(event.data["heir"].clone()) else {
        return;
    };
    if heir != Holder::Resident(dead) && realm.res::<Contracts>().count_of(dead) > 0 {
        realm.res_mut::<Estates>().heirs.insert(dead, heir);
    }
}

/// Forgets the heir of a resident that has no contract any more.
pub(crate) fn settle(realm: &Realm, contracts: &Contracts, resident: ResidentId) {
    if contracts.count_of(resident) == 0 && realm.res::<Estates>().heirs.contains_key(&resident) {
        realm.res_mut::<Estates>().heirs.remove(&resident);
    }
}

/// Pays money from a holder of the global realm to a resident, or to its
/// heir when the resident left the world.
pub(crate) fn pay(realm: &Realm, from: Holder, to: ResidentId, amount: i64, reason: &str) {
    if realm.directory().is_alive(to) {
        pay::wire_to_resident(realm, from, to, amount, reason);
        return;
    }
    let heir = realm.res::<Estates>().heirs.get(&to).copied();
    match heir {
        Some(Holder::Resident(heir)) => pay::wire_to_resident(realm, from, heir, amount, reason),
        // An organization, an account and an escrow are in this ledger.
        Some(heir @ (Holder::Org(_) | Holder::Account(_) | Holder::Escrow(_))) => {
            pay::transfer(realm, from, heir, amount, reason)
        }
        // No heir that the global realm can reach: the kernel gives a wire
        // for a resident that is gone to the unclaimed account.
        Some(Holder::Site(_)) | None => pay::wire_to_resident(realm, from, to, amount, reason),
    };
}
