//! The directory of residents in the global realm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{OwnerId, ResidentId, ZoneId};

use crate::message::DirUpdate;

/// What the global realm knows about one resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirEntry {
    /// The zone in which the resident is, or was when it left the world.
    pub zone: ZoneId,
    /// The owner of the resident.
    pub owner: OwnerId,
    /// Display name.
    pub name: String,
    /// False after the resident was removed from the world.
    pub alive: bool,
}

/// Every resident that ever lived, with its zone, owner and name.
///
/// The directory is state of the global realm. Zones tell the global realm
/// about spawns, migrations and despawns by message, so the directory shows
/// the world as it was at the end of the zone ticks of the current frame.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    entries: BTreeMap<ResidentId, DirEntry>,
    living: u64,
}

impl Directory {
    /// Returns the entry of a resident, alive or not.
    #[must_use]
    pub fn get(&self, id: ResidentId) -> Option<&DirEntry> {
        self.entries.get(&id)
    }

    /// Returns the zone of a living resident.
    #[must_use]
    pub fn zone_of(&self, id: ResidentId) -> Option<ZoneId> {
        self.entries.get(&id).filter(|e| e.alive).map(|e| e.zone)
    }

    /// Returns true when the resident is in the world.
    #[must_use]
    pub fn is_alive(&self, id: ResidentId) -> bool {
        self.entries.get(&id).is_some_and(|e| e.alive)
    }

    /// Returns the number of residents that ever lived.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when no resident ever lived.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Returns the number of residents in the world now.
    #[must_use]
    pub fn living(&self) -> u64 {
        self.living
    }

    /// Returns every entry in id order.
    pub fn iter(&self) -> impl Iterator<Item = (ResidentId, &DirEntry)> + '_ {
        self.entries.iter().map(|(&id, entry)| (id, entry))
    }

    /// Returns the residents of an owner, alive or not, in id order. It
    /// scans the whole directory.
    #[must_use]
    pub fn of_owner(&self, owner: OwnerId) -> Vec<ResidentId> {
        self.entries.iter().filter(|(_, e)| e.owner == owner).map(|(&id, _)| id).collect()
    }

    pub(crate) fn apply(&mut self, update: DirUpdate) {
        match update {
            DirUpdate::Spawned { id, owner, name, zone } => {
                let previous = self.entries.insert(id, DirEntry { zone, owner, name, alive: true });
                if !previous.is_some_and(|e| e.alive) {
                    self.living += 1;
                }
            }
            DirUpdate::Moved { id, zone } => {
                if let Some(entry) = self.entries.get_mut(&id) {
                    entry.zone = zone;
                }
            }
            DirUpdate::Despawned { id } => {
                if let Some(entry) = self.entries.get_mut(&id)
                    && entry.alive
                {
                    entry.alive = false;
                    self.living -= 1;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_follows_spawn_move_and_despawn() {
        let mut dir = Directory::default();
        assert!(dir.is_empty());
        for (id, owner) in [(1, 10), (2, 10), (3, 11)] {
            dir.apply(DirUpdate::Spawned {
                id: ResidentId(id),
                owner: OwnerId(owner),
                name: format!("r{id}"),
                zone: ZoneId(0),
            });
        }
        dir.apply(DirUpdate::Moved { id: ResidentId(2), zone: ZoneId(3) });
        dir.apply(DirUpdate::Moved { id: ResidentId(99), zone: ZoneId(3) });
        dir.apply(DirUpdate::Despawned { id: ResidentId(1) });
        dir.apply(DirUpdate::Despawned { id: ResidentId(1) });
        assert_eq!((dir.len(), dir.living()), (3, 2));
        assert_eq!(dir.zone_of(ResidentId(2)), Some(ZoneId(3)));
        assert_eq!(dir.zone_of(ResidentId(1)), None);
        assert!(!dir.is_alive(ResidentId(1)) && dir.is_alive(ResidentId(3)));
        assert_eq!(dir.get(ResidentId(1)).map(|e| e.name.as_str()), Some("r1"));
        assert_eq!(dir.of_owner(OwnerId(10)), [ResidentId(1), ResidentId(2)]);
        assert_eq!(dir.iter().count(), 3);
    }
}
