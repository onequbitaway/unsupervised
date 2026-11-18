//! The directory of residents in the global realm.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{OwnerId, ResidentId, ZoneId};

use crate::message::DirUpdate;
use crate::rng::Rng;

/// Draws of [`Directory::sample_alive`] that can find a resident that left
/// the world before it counts the living ones instead.
const SAMPLE_TRIES: u32 = 32;

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
#[serde(from = "SavedDirectory")]
pub struct Directory {
    entries: BTreeMap<ResidentId, DirEntry>,
    living: u64,
    /// The ids of `entries`, in id order: a dense list from which
    /// [`Directory::sample_alive`] draws. It is derived from `entries`, so
    /// it is not saved.
    #[serde(skip)]
    ids: Vec<ResidentId>,
}

/// A [`Directory`] as it is read. The list of ids is made from the entries.
#[derive(Deserialize)]
struct SavedDirectory {
    entries: BTreeMap<ResidentId, DirEntry>,
    living: u64,
}

impl From<SavedDirectory> for Directory {
    fn from(saved: SavedDirectory) -> Directory {
        let ids = saved.entries.keys().copied().collect();
        Directory { entries: saved.entries, living: saved.living, ids }
    }
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

    /// Draws one living resident at random: each resident that is in the
    /// world now has the same chance. Returns `None` when nobody lives.
    ///
    /// Give it the generator of the realm (`realm.rng()`), so that the draw
    /// is the same in every run. The draw does not assume anything about
    /// the ids: they need not start at 1 or follow each other.
    ///
    /// The cost does not grow with the number of residents while a fair
    /// part of those that ever lived is alive. It draws from all ids and
    /// takes the first that is alive. After 32 draws without one (most
    /// residents have left the world), it counts through the living
    /// residents one time instead.
    pub fn sample_alive(&self, rng: &mut Rng) -> Option<ResidentId> {
        if self.living == 0 {
            return None;
        }
        for _ in 0..SAMPLE_TRIES {
            let id = self.ids[rng.below(self.ids.len() as u64) as usize];
            if self.is_alive(id) {
                return Some(id);
            }
        }
        // Few are alive. Every living resident still has the same chance:
        // the draws above gave each the same chance, and so does this one.
        let nth = rng.below(self.living) as usize;
        self.entries.iter().filter(|(_, entry)| entry.alive).nth(nth).map(|(&id, _)| id)
    }

    pub(crate) fn apply(&mut self, update: DirUpdate) {
        match update {
            DirUpdate::Spawned { id, owner, name, zone } => {
                let previous = self.entries.insert(id, DirEntry { zone, owner, name, alive: true });
                if previous.is_none() {
                    // Ids grow, so the place is the end, or near it when
                    // two zones told of their spawns in the same frame.
                    let at = self.ids.partition_point(|&other| other < id);
                    self.ids.insert(at, id);
                }
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

    /// Makes a directory with the given ids, told in the given order.
    fn directory(ids: &[u64]) -> Directory {
        let mut dir = Directory::default();
        for &id in ids {
            dir.apply(DirUpdate::Spawned {
                id: ResidentId(id),
                owner: OwnerId(1),
                name: format!("r{id}"),
                zone: ZoneId(0),
            });
        }
        dir
    }

    /// Draws `draws` residents and returns how often each id came.
    fn histogram(dir: &Directory, rng: &mut Rng, draws: u32) -> BTreeMap<u64, u32> {
        let mut seen = BTreeMap::new();
        for _ in 0..draws {
            let id = dir.sample_alive(rng).expect("somebody lives");
            *seen.entry(id.0).or_insert(0) += 1;
        }
        seen
    }

    #[test]
    fn sample_alive_draws_only_the_living_and_each_as_often() {
        // Ids with gaps, far from 1, told out of order.
        let ids = [900, 7, 5_000_000_000, 41, 300, 12, 8, 77_000, 13, 650];
        let mut dir = directory(&ids);
        assert_eq!(dir.ids, {
            let mut sorted: Vec<ResidentId> = ids.iter().map(|&id| ResidentId(id)).collect();
            sorted.sort_unstable();
            sorted
        });
        for gone in [41, 8, 5_000_000_000] {
            dir.apply(DirUpdate::Despawned { id: ResidentId(gone) });
        }
        let mut rng = Rng::seeded(61);
        let seen = histogram(&dir, &mut rng, 70_000);
        assert_eq!(seen.keys().copied().collect::<Vec<_>>(), [7, 12, 13, 300, 650, 900, 77_000]);
        // 10,000 draws for each of the 7 on average. A fair draw stays
        // within 4 % of that; a draw that favours one id does not.
        for (id, count) in &seen {
            assert!((9_600..=10_400).contains(count), "resident {id} was drawn {count} times");
        }
    }

    #[test]
    fn sample_alive_is_fair_when_few_are_alive() {
        // 3 of 600 are alive: 32 draws from all ids find one only in one
        // case of seven, and the count through the living takes over.
        let ids: Vec<u64> = (1..=600).map(|n| n * 3).collect();
        let mut dir = directory(&ids);
        for &id in &ids {
            if ![300, 900, 1_797].contains(&id) {
                dir.apply(DirUpdate::Despawned { id: ResidentId(id) });
            }
        }
        assert_eq!(dir.living(), 3);
        let mut rng = Rng::seeded(62);
        let seen = histogram(&dir, &mut rng, 15_000);
        assert_eq!(seen.keys().copied().collect::<Vec<_>>(), [300, 900, 1_797]);
        for (id, count) in &seen {
            assert!((4_750..=5_250).contains(count), "resident {id} was drawn {count} times");
        }
    }

    #[test]
    fn sample_alive_of_nobody_and_of_one() {
        let mut rng = Rng::seeded(63);
        let untouched = rng.clone();
        let mut dir = Directory::default();
        assert_eq!(dir.sample_alive(&mut rng), None);
        dir = directory(&[44]);
        assert_eq!(dir.sample_alive(&mut rng), Some(ResidentId(44)));
        dir.apply(DirUpdate::Despawned { id: ResidentId(44) });
        let before = rng.clone();
        assert_eq!(dir.sample_alive(&mut rng), None);
        // A draw from nobody does not use the generator.
        assert_eq!(rng, before);
        assert_ne!(rng, untouched);
        // A resident that is told again (the same id) is one entry.
        dir = directory(&[44, 44, 9]);
        assert_eq!(dir.ids, [ResidentId(9), ResidentId(44)]);
        assert_eq!((dir.len(), dir.living()), (2, 2));
    }

    #[test]
    fn sample_alive_is_the_same_after_the_directory_was_saved_and_read() {
        let mut dir = directory(&[30, 10, 20, 50, 40, 70, 60]);
        dir.apply(DirUpdate::Despawned { id: ResidentId(20) });
        dir.apply(DirUpdate::Moved { id: ResidentId(30), zone: ZoneId(2) });
        let bytes = crate::codec::encode("directory", &dir);
        let read: Directory = crate::codec::decode("directory", &bytes).unwrap();
        assert_eq!(read, dir);
        assert_eq!(read.ids, dir.ids);
        // The list of ids is not written: the bytes are those of a
        // directory without it.
        #[derive(Serialize)]
        struct Old<'a> {
            entries: &'a BTreeMap<ResidentId, DirEntry>,
            living: u64,
        }
        assert_eq!(
            bytes,
            crate::codec::encode("directory", &Old { entries: &dir.entries, living: 6 })
        );
        // The same generator draws the same residents from both.
        let (mut a, mut b) = (Rng::seeded(64), Rng::seeded(64));
        let from_made: Vec<_> = (0..200).map(|_| dir.sample_alive(&mut a)).collect();
        let from_read: Vec<_> = (0..200).map(|_| read.sample_alive(&mut b)).collect();
        assert_eq!(from_made, from_read);
        assert_eq!(a, b);
        assert!(from_made.iter().all(|id| id.is_some_and(|id| id != ResidentId(20))));
    }
}
