//! The directory of the coordinator: in which zone a resident is.
//!
//! The kernel keeps the directory in the global realm. Zones tell it about
//! every spawn, migration and despawn, and it is exact after the tick of
//! the global realm in each frame. Between two frames it does not yet know
//! what the inputs since the last frame did. The coordinator keeps those
//! changes here until the next tick of the global realm.

use std::collections::HashMap;

use unsup_kernel::World;
use unsup_protocol::{RealmId, ResidentId, ZoneId};

/// What the directory knows about a resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Where {
    /// The resident is in this zone.
    Alive(ZoneId),
    /// The resident left the world in this zone. The world of that zone
    /// applies the rule of the kernel for a message to it.
    Gone(ZoneId),
    /// No resident ever had this id.
    Unknown,
}

/// The changes of the directory since the last tick of the global realm.
#[derive(Default)]
pub(crate) struct Lookup {
    recent: HashMap<ResidentId, ZoneId>,
}

impl Lookup {
    /// Notes that a resident is in a zone now: it was made there or it
    /// arrived there between two frames.
    pub(crate) fn moved(&mut self, resident: ResidentId, zone: ZoneId) {
        self.recent.insert(resident, zone);
    }

    /// Forgets the changes. The global realm ticked: its directory has
    /// them.
    pub(crate) fn clear(&mut self) {
        self.recent.clear();
    }

    /// Finds a resident. `global` is the world that hosts the global realm.
    pub(crate) fn find(&self, global: &World, resident: ResidentId) -> Where {
        if let Some(&zone) = self.recent.get(&resident) {
            return Where::Alive(zone);
        }
        let Some(realm) = global.realm(RealmId::Global) else {
            return Where::Unknown;
        };
        match realm.directory().get(resident) {
            Some(entry) if entry.alive => Where::Alive(entry.zone),
            Some(entry) => Where::Gone(entry.zone),
            None => Where::Unknown,
        }
    }
}
