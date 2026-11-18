//! The residents and the map of one zone.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

use axum::extract::ws::Utf8Bytes;
use unsup_protocol::{Pos, ResidentId, ResidentView, SiteView, Tick, ZoneDelta, ZoneId, ZoneView};

/// The tiles and sites of one zone, as the simulation gave them.
#[derive(Clone, Debug, Default)]
pub(crate) struct ZoneMap {
    /// Number of tile columns.
    pub width: u32,
    /// Number of tile rows.
    pub height: u32,
    /// Tile kinds, row by row.
    pub tiles: Vec<u8>,
    /// Sites in the zone.
    pub sites: Vec<SiteView>,
    /// The frame of the cache when the map was read.
    pub read_at: u64,
}

/// A zone delta in which a resident left the zone, after the cache
/// resolved it: it holds only residents that are in the zone after the
/// delta, and the newest entry of each.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Resolved {
    /// Residents that are in the zone now and were not before, or that came
    /// back, with the view of their last entry.
    pub entered: Vec<ResidentView>,
    /// Residents that were in the zone and are not any more.
    pub left: Vec<ResidentId>,
    /// The last position of every resident that moved and is in the zone.
    pub moved: Vec<(ResidentId, Pos)>,
    /// Changes of activity of residents that are in the zone.
    pub activity: Vec<(ResidentId, Option<String>)>,
}

/// A resident that is in no zone after its zone delta. It left the world
/// unless another zone delta of the same frame takes it.
#[derive(Clone, Debug)]
pub(crate) struct Gone {
    /// The last view of the resident.
    pub view: ResidentView,
    /// The zone that it left.
    pub zone: ZoneId,
}

/// Everything that the cache knows about one zone.
#[derive(Debug)]
pub(crate) struct ZoneCache {
    /// Id, city, name and bounds. The `residents` field is not kept
    /// current; use [`ZoneCache::view`].
    info: ZoneView,
    /// False when another process hosts the zone.
    pub hosted: bool,
    /// Local tick of the zone after its last delta.
    pub tick: Tick,
    /// The residents of the zone.
    pub residents: HashMap<ResidentId, ResidentView>,
    /// Tiles and sites.
    pub map: ZoneMap,
    /// True while one request reads the map again.
    pub refreshing: AtomicBool,
    /// Grows with every change of the residents.
    version: u64,
    /// The `snapshot` message of the zone at one version, so that many
    /// viewers share one serialization.
    memo: Mutex<Option<(u64, Utf8Bytes)>>,
}

impl ZoneCache {
    /// Makes a zone without residents and without a map.
    pub fn new(info: ZoneView) -> ZoneCache {
        ZoneCache {
            info,
            hosted: false,
            tick: Tick(0),
            residents: HashMap::new(),
            map: ZoneMap::default(),
            refreshing: AtomicBool::new(false),
            version: 0,
            memo: Mutex::new(None),
        }
    }

    /// Returns the public view of the zone with the current number of
    /// residents.
    pub fn view(&self) -> ZoneView {
        ZoneView { residents: self.population(), ..self.info.clone() }
    }

    /// Returns the static part of the zone: id, city, name, bounds.
    pub fn info(&self) -> &ZoneView {
        &self.info
    }

    /// Returns the number of residents.
    pub fn population(&self) -> u32 {
        u32::try_from(self.residents.len()).unwrap_or(u32::MAX)
    }

    /// Returns the residents in ascending id order.
    pub fn sorted_residents(&self) -> Vec<ResidentView> {
        let mut residents: Vec<ResidentView> = self.residents.values().cloned().collect();
        residents.sort_unstable_by_key(|r| r.id);
        residents
    }

    /// Replaces the residents and the map with what the simulation says.
    pub fn load(&mut self, residents: Vec<ResidentView>, map: ZoneMap) {
        self.hosted = true;
        self.residents = residents.into_iter().map(|r| (r.id, r)).collect();
        self.map = map;
        self.version += 1;
    }

    /// Returns the memo of the `snapshot` message when it is current.
    pub fn snapshot_memo(&self) -> Option<Utf8Bytes> {
        let memo = self.memo.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        memo.as_ref().filter(|(version, _)| *version == self.version).map(|(_, text)| text.clone())
    }

    /// Keeps the `snapshot` message of the current version.
    pub fn set_snapshot_memo(&self, text: Utf8Bytes) {
        let mut memo = self.memo.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *memo = Some((self.version, text));
    }

    /// Applies one delta of the simulation.
    ///
    /// `index` says in which zone every resident is. A resident that is in
    /// no zone after the delta goes to `gone`; the caller removes it from
    /// there when another zone of the same frame takes it.
    ///
    /// A delta without a departure is exact and is applied as it is. A
    /// delta with a departure is not: the kernel writes the moves and the
    /// activities of all residents to the delta before it removes a row,
    /// so the lists can hold entries of residents that left, and more than
    /// one entry of a resident. The lists do not say in which order things
    /// happened. The cache resolves such a delta with two facts:
    ///
    /// - A resident is in the zone after the delta when the number of times
    ///   it entered, plus one when it was there before, is greater than the
    ///   number of times it left.
    /// - The last `moved` entry of a resident is its position when that
    ///   position is in the zone. A position outside the zone is the step
    ///   with which the resident left; a resident that came back after it
    ///   has its position in the `entered` view.
    ///
    /// Returns the resolved delta in that case, for the viewers.
    pub fn apply(
        &mut self,
        delta: &ZoneDelta,
        index: &mut HashMap<ResidentId, ZoneId>,
        gone: &mut Vec<Gone>,
    ) -> Option<Resolved> {
        let zone = self.info.id;
        self.tick = delta.tick;
        self.version += 1;
        if !delta.left.is_empty() {
            let resolved = self.resolve(delta, index, gone);
            self.apply_changes(&resolved.moved, &resolved.activity);
            return Some(resolved);
        }
        for view in &delta.entered {
            self.residents.insert(view.id, view.clone());
            index.insert(view.id, zone);
        }
        self.apply_changes(&delta.moved, &delta.activity);
        None
    }

    /// Sets positions and activities of residents that are in the zone.
    fn apply_changes(
        &mut self,
        moved: &[(ResidentId, Pos)],
        activity: &[(ResidentId, Option<String>)],
    ) {
        for (id, pos) in moved {
            if let Some(resident) = self.residents.get_mut(id) {
                resident.pos = *pos;
            }
        }
        for (id, verb) in activity {
            if let Some(resident) = self.residents.get_mut(id) {
                resident.activity.clone_from(verb);
            }
        }
    }

    /// Removes a resident that left. It is gone when the index still says
    /// that it is in this zone.
    fn remove(
        &mut self,
        id: ResidentId,
        index: &mut HashMap<ResidentId, ZoneId>,
        gone: &mut Vec<Gone>,
    ) -> bool {
        let Some(view) = self.residents.remove(&id) else {
            return false;
        };
        if index.get(&id) == Some(&self.info.id) {
            index.remove(&id);
            gone.push(Gone { view, zone: self.info.id });
        }
        true
    }

    /// Applies `entered` and `left` of a delta with a departure, and
    /// returns the delta with only what is true after it. See
    /// [`ZoneCache::apply`].
    fn resolve(
        &mut self,
        delta: &ZoneDelta,
        index: &mut HashMap<ResidentId, ZoneId>,
        gone: &mut Vec<Gone>,
    ) -> Resolved {
        let zone = self.info.id;
        // Per resident: times entered minus times left, and the last view.
        let mut net: HashMap<ResidentId, (i64, Option<&ResidentView>)> = HashMap::new();
        // The order of first mention, so that the output does not depend on
        // the order of a hash map.
        let mut order: Vec<ResidentId> = Vec::new();
        for id in &delta.left {
            let entry = net.entry(*id).or_insert_with(|| {
                order.push(*id);
                (0, None)
            });
            entry.0 -= 1;
        }
        for view in &delta.entered {
            let entry = net.entry(view.id).or_insert_with(|| {
                order.push(view.id);
                (0, None)
            });
            entry.0 += 1;
            entry.1 = Some(view);
        }
        let mut resolved = Resolved::default();
        for id in order {
            let Some(&(count, last)) = net.get(&id) else { continue };
            let before = self.residents.contains_key(&id);
            let after = count + i64::from(before) > 0;
            match (after, last) {
                (true, Some(view)) => {
                    self.residents.insert(id, view.clone());
                    index.insert(id, zone);
                    resolved.entered.push(view.clone());
                }
                (true, None) => {}
                (false, _) => {
                    if self.remove(id, index, gone) {
                        resolved.left.push(id);
                    }
                }
            }
        }
        // The last entry of each resident, when the resident is here and
        // the position is in the zone.
        let mut last_move: HashMap<ResidentId, usize> = HashMap::new();
        for (at, (id, _)) in delta.moved.iter().enumerate() {
            last_move.insert(*id, at);
        }
        resolved.moved = delta
            .moved
            .iter()
            .enumerate()
            .filter(|(at, (id, pos))| {
                last_move.get(id) == Some(at)
                    && self.residents.contains_key(id)
                    && self.info.bounds.contains(*pos)
            })
            .map(|(_, entry)| *entry)
            .collect();
        resolved.activity = delta
            .activity
            .iter()
            .filter(|(id, _)| self.residents.contains_key(id))
            .cloned()
            .collect();
        resolved
    }
}

#[cfg(test)]
mod tests {
    use unsup_protocol::{CityId, Rect};

    use super::*;

    /// Zone 1 covers the columns 0 to 7.
    fn zone(id: u32) -> ZoneCache {
        ZoneCache::new(ZoneView {
            id: ZoneId(id),
            city: CityId(0),
            name: format!("zone {id}"),
            bounds: Rect::sized(0, 0, 8, 8),
            residents: 0,
        })
    }

    fn view(id: u64, x: i32) -> ResidentView {
        ResidentView {
            id: ResidentId(id),
            name: format!("r{id}"),
            pos: Pos::new(x, 0),
            activity: None,
            model: None,
        }
    }

    fn delta(entered: Vec<ResidentView>, left: Vec<u64>) -> ZoneDelta {
        ZoneDelta {
            zone: ZoneId(1),
            tick: Tick(1),
            entered,
            left: left.into_iter().map(ResidentId).collect(),
            ..ZoneDelta::default()
        }
    }

    struct World {
        cache: ZoneCache,
        index: HashMap<ResidentId, ZoneId>,
        gone: Vec<Gone>,
    }

    impl World {
        fn new() -> World {
            World { cache: zone(1), index: HashMap::new(), gone: Vec::new() }
        }

        fn apply(&mut self, delta: &ZoneDelta) -> Option<Resolved> {
            self.cache.apply(delta, &mut self.index, &mut self.gone)
        }
    }

    #[test]
    fn a_delta_without_a_departure_is_applied_as_it_is() {
        let mut world = World::new();
        let change = ZoneDelta {
            moved: vec![(ResidentId(1), Pos::new(6, 2)), (ResidentId(99), Pos::new(1, 1))],
            activity: vec![(ResidentId(1), Some("work".into())), (ResidentId(99), None)],
            tick: Tick(9),
            ..delta(vec![view(1, 0)], vec![])
        };
        assert_eq!(world.apply(&change), None);
        // The move and the activity are newer than the `entered` view.
        // Entries of residents that the zone does not have are ignored.
        let mut expected = view(1, 0);
        expected.pos = Pos::new(6, 2);
        expected.activity = Some("work".into());
        assert_eq!(world.cache.sorted_residents(), vec![expected]);
        assert_eq!(world.cache.tick, Tick(9));
        assert_eq!(world.index.get(&ResidentId(1)), Some(&ZoneId(1)));
    }

    #[test]
    fn a_resident_that_enters_and_leaves_in_one_delta_is_not_there() {
        // A new resident that a hook moved to another zone at once. The
        // kernel also lists the step out of the zone as a move.
        let mut world = World::new();
        let change = ZoneDelta {
            moved: vec![(ResidentId(7), Pos::new(12, 0))],
            ..delta(vec![view(7, 3)], vec![7])
        };
        assert_eq!(world.apply(&change), Some(Resolved::default()));
        assert_eq!(world.cache.population(), 0);
        assert!(world.index.is_empty());
        assert!(world.gone.is_empty());
    }

    #[test]
    fn a_resident_that_leaves_and_comes_back_in_one_delta_is_there() {
        let mut world = World::new();
        assert_eq!(world.apply(&delta(vec![view(7, 3)], vec![])), None);
        // It left to column 12, and it came back at column 5. The move out
        // of the zone must not win over the view of the return.
        let change = ZoneDelta {
            moved: vec![(ResidentId(7), Pos::new(12, 0))],
            ..delta(vec![view(7, 5)], vec![7])
        };
        assert_eq!(
            world.apply(&change),
            Some(Resolved { entered: vec![view(7, 5)], ..Resolved::default() })
        );
        assert_eq!(world.cache.sorted_residents(), vec![view(7, 5)]);
        assert_eq!(world.index.get(&ResidentId(7)), Some(&ZoneId(1)));
        assert!(world.gone.is_empty());
    }

    #[test]
    fn the_last_move_inside_the_zone_wins_over_older_entries() {
        let mut world = World::new();
        world.apply(&delta(vec![view(1, 0), view(2, 0)], vec![]));
        // Resident 1 moved to column 3; then resident 2 left, which wrote
        // the delta once; then resident 1 moved to column 4.
        let change = ZoneDelta {
            moved: vec![
                (ResidentId(1), Pos::new(3, 0)),
                (ResidentId(2), Pos::new(9, 0)),
                (ResidentId(1), Pos::new(4, 0)),
            ],
            activity: vec![
                (ResidentId(2), Some("walk".into())),
                (ResidentId(1), Some("rest".into())),
            ],
            ..delta(vec![], vec![2])
        };
        assert_eq!(
            world.apply(&change),
            Some(Resolved {
                entered: vec![],
                left: vec![ResidentId(2)],
                moved: vec![(ResidentId(1), Pos::new(4, 0))],
                activity: vec![(ResidentId(1), Some("rest".into()))],
            })
        );
        let mut expected = view(1, 4);
        expected.activity = Some("rest".into());
        assert_eq!(world.cache.sorted_residents(), vec![expected]);
        // Resident 2 is in no zone now: it is gone until another zone takes it.
        assert_eq!(world.gone.len(), 1);
        assert_eq!((world.gone[0].view.id, world.gone[0].zone), (ResidentId(2), ZoneId(1)));
        assert_eq!(world.index.get(&ResidentId(2)), None);
    }

    #[test]
    fn a_resident_that_another_zone_took_already_is_not_gone() {
        // The delta of the new zone can come before the delta of the old zone.
        let mut world = World::new();
        world.apply(&delta(vec![view(1, 0)], vec![]));
        world.index.insert(ResidentId(1), ZoneId(0));
        assert_eq!(
            world.apply(&delta(vec![], vec![1])),
            Some(Resolved { left: vec![ResidentId(1)], ..Resolved::default() })
        );
        assert!(world.gone.is_empty());
        assert_eq!(world.index.get(&ResidentId(1)), Some(&ZoneId(0)));
        assert_eq!(world.cache.population(), 0);
    }

    #[test]
    fn the_snapshot_memo_ends_with_the_next_change() {
        let mut world = World::new();
        assert_eq!(world.cache.snapshot_memo(), None);
        world.cache.set_snapshot_memo(Utf8Bytes::from("one"));
        assert_eq!(world.cache.snapshot_memo().as_deref(), Some("one"));
        world.apply(&delta(vec![view(1, 0)], vec![]));
        assert_eq!(world.cache.snapshot_memo(), None);
    }
}
