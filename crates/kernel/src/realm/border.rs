//! Sight across a zone border.
//!
//! Zones share nothing while they tick. So that a resident near a border
//! can still see and hear a resident on the other side, each zone tells the
//! zones around it about its residents that are near them, and keeps a copy
//! of what they tell it:
//!
//! - A zone has *watchers*: every zone that has a tile at most one zone
//!   size away. Normally these are the eight zones around it.
//! - At the end of each tick a zone sends each watcher what changed among
//!   its residents that are at most `kernel.sight_radius` tiles from the
//!   watcher: who came into that band, who moved or changed its activity
//!   there, and who left it. These are ordinary realm messages, so the
//!   watcher takes them at the start of its next tick. A zone in which
//!   nothing changed near a border sends nothing.
//! - The watcher keeps the copy. Only these messages change it. It shows
//!   the other zones as they were at the end of their last tick: it is one
//!   frame old.
//! - A resident that walks into another zone stays in the copies: the zone
//!   that it leaves tells its watchers the new home, keeps a copy itself,
//!   and tells the new home who holds a copy.
//!
//! The copy, and what the sender knows about the copies of its watchers,
//! are simulation state. They are in the snapshot and in the state hash.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{Pos, Rect, ResidentId, ZoneId};

use super::{F_BORDER, FateKind, Realm};
use crate::ids::{Idx, VerbId};
use crate::layout::Layout;
use crate::message::{Dest, KernelMsg};

/// Greatest number of watchers of one zone: each is one bit of a `u32`.
/// Zones are squares of one size that do not overlap, so at most 24 of them
/// have a tile within one zone size of a zone.
const WATCHERS_MAX: usize = 32;

/// A resident of another zone that is near the border of this zone, as
/// this zone last heard of it. See [`Realm::nearby_outside`].
///
/// The zone cannot change such a resident and cannot read its attributes
/// or components. It can send it a message, a notice or a wire by id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outsider {
    /// The resident.
    pub id: ResidentId,
    /// The zone in which the resident is.
    pub zone: ZoneId,
    /// Its position at the end of the last tick of that zone.
    pub pos: Pos,
    /// Its display name.
    pub name: String,
    /// The verb of its running action at the end of the last tick of that
    /// zone, if it had one.
    pub activity: Option<&'static str>,
}

/// The copy of one resident of another zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Ghost {
    #[serde(rename = "p")]
    pos: Pos,
    #[serde(rename = "n")]
    name: String,
    #[serde(rename = "a", default, skip_serializing_if = "Option::is_none")]
    activity: Option<VerbId>,
}

/// One change that a zone tells a watcher about a resident near it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum BorderNews {
    /// The resident came into the band of the watcher, or it moves to the
    /// zone `home` and stays in the band. The watcher forgets the copy that
    /// it has from the sender and keeps one under `home`.
    #[serde(rename = "e")]
    Enter {
        #[serde(rename = "i")]
        id: ResidentId,
        #[serde(rename = "h")]
        home: ZoneId,
        #[serde(rename = "g")]
        ghost: Ghost,
    },
    /// The resident moved inside the band, or its activity changed.
    #[serde(rename = "u")]
    Update {
        #[serde(rename = "i")]
        id: ResidentId,
        #[serde(rename = "p")]
        pos: Pos,
        #[serde(rename = "a", default, skip_serializing_if = "Option::is_none")]
        activity: Option<VerbId>,
    },
    /// The resident left the band of the watcher, or the world.
    #[serde(rename = "l")]
    Leave {
        #[serde(rename = "i")]
        id: ResidentId,
    },
}

/// What happens to a resident at the end of the step.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    /// It stays in this zone.
    Stays,
    /// It leaves the world.
    Leaves,
    /// It moves to another zone.
    MovesTo(ZoneId),
}

/// What a zone knows about sight across its borders.
pub(super) struct Border {
    /// The zones that this zone tells about its residents, with their
    /// bounds, ordered by id. The list never changes.
    watchers: Vec<(ZoneId, Rect)>,
    /// One entry per resident, in index order. Bit `k` is set when watcher
    /// `k` holds a copy of the resident.
    pub(super) shown: RefCell<Vec<u32>>,
    /// The residents whose copies may be out of date, by dense index, in
    /// the order in which they changed. Each has the flag `F_BORDER`.
    pub(super) dirty: RefCell<Vec<u32>>,
    /// The copy: residents of other zones near this zone, by the zone in
    /// which they are and their id.
    pub(super) ghosts: RefCell<BTreeMap<(ZoneId, ResidentId), Ghost>>,
    /// For each resident that moves to another zone in this step: the zones
    /// that hold a copy of it under its new home. The resident takes the
    /// list with it.
    handoff: RefCell<BTreeMap<ResidentId, Vec<ZoneId>>>,
    /// Residents with a copy whose action ended and that did not start a
    /// new one yet, by dense index, with the verb that ended. A resident
    /// that starts the same verb again at once (a routine does that) has
    /// the activity that its copies show, and nothing is told. The list is
    /// empty whenever the realm has settled.
    pub(super) ended: RefCell<BTreeMap<u32, VerbId>>,
}

impl Border {
    /// Makes the border state of a realm. The global realm has no zone and
    /// no watchers.
    pub(super) fn new(layout: &Layout, zone: Option<ZoneId>) -> Border {
        let mut watchers: Vec<(ZoneId, Rect)> = zone
            .map(|zone| layout.zones_within(zone, layout.zone_size()))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|other| layout.zone(other).map(|info| (other, info.bounds)))
            .collect();
        debug_assert!(watchers.len() <= WATCHERS_MAX, "a zone has {} watchers", watchers.len());
        watchers.truncate(WATCHERS_MAX);
        Border {
            watchers,
            shown: RefCell::new(Vec::new()),
            dirty: RefCell::new(Vec::new()),
            ghosts: RefCell::new(BTreeMap::new()),
            handoff: RefCell::new(BTreeMap::new()),
            ended: RefCell::new(BTreeMap::new()),
        }
    }

    /// Returns true when the realm has zones to tell about its residents.
    pub(super) fn has_watchers(&self) -> bool {
        !self.watchers.is_empty()
    }

    /// Takes the zones that hold a copy of a resident under its new home.
    pub(super) fn take_handoff(&self, id: ResidentId) -> Vec<ZoneId> {
        self.handoff.borrow_mut().remove(&id).unwrap_or_default()
    }

    /// Returns an estimate of the heap bytes of the border state.
    pub(super) fn memory_estimate(&self) -> usize {
        let ghosts: usize = self
            .ghosts
            .borrow()
            .values()
            .map(|ghost| size_of::<(ZoneId, ResidentId)>() + size_of::<Ghost>() + ghost.name.len())
            .sum();
        (self.shown.borrow().capacity() + self.dirty.borrow().capacity()) * size_of::<u32>()
            + ghosts
    }
}

impl Realm {
    /// Returns the residents of other zones within `radius` tiles of `pos`
    /// that this zone knows of, sorted by `ResidentId`.
    ///
    /// A zone knows the residents of the zones around it that are at most
    /// `kernel.sight_radius` tiles from its own tiles (and at most one zone
    /// size). What it knows is one frame old: the position and the activity
    /// are those at the end of the last tick of the other zone. A resident
    /// that an input moved or created between frames shows after the next
    /// tick of its zone.
    ///
    /// Use it together with [`Realm::nearby`], which finds the residents of
    /// this zone. The distance is [`Pos::dist`].
    #[must_use]
    pub fn nearby_outside(&self, pos: Pos, radius: u32) -> Vec<Outsider> {
        let mut out = Vec::new();
        self.for_each_outsider(pos, radius, |zone, id, ghost| {
            out.push(Outsider {
                id,
                zone,
                pos: ghost.pos,
                name: ghost.name.clone(),
                activity: ghost.activity.map(|verb| self.bp.verb_name(verb)),
            });
        });
        out.sort_unstable_by_key(|outsider| outsider.id);
        out
    }

    /// Calls `f` for every copy of a resident of another zone within
    /// `radius` tiles of `pos`, ordered by zone and id.
    pub(super) fn for_each_outsider(
        &self,
        pos: Pos,
        radius: u32,
        mut f: impl FnMut(ZoneId, ResidentId, &Ghost),
    ) {
        let ghosts = self.border.ghosts.borrow();
        if ghosts.is_empty() {
            return;
        }
        // Every copy is of a resident outside this zone. A circle that
        // stays inside the zone finds none.
        if let Some(zone) = &self.zone {
            let reach = i64::from(radius);
            let (x, y) = (i64::from(pos.x), i64::from(pos.y));
            let b = zone.bounds;
            if x - reach >= i64::from(b.x0)
                && x + reach < i64::from(b.x1)
                && y - reach >= i64::from(b.y0)
                && y + reach < i64::from(b.y1)
            {
                return;
            }
        }
        let index = self.index.borrow();
        for (&(zone, id), ghost) in ghosts.iter() {
            // A resident that came into this zone is here itself; its copy
            // leaves with the next message of its old zone.
            if ghost.pos.dist(pos) <= radius && !index.contains_key(&id) {
                f(zone, id, ghost);
            }
        }
    }

    /// Returns how far into this zone its watchers see: the sight radius,
    /// at most one zone size. It is 0 when there is no watcher.
    pub(super) fn border_band(&self) -> u32 {
        if !self.border.has_watchers() {
            return 0;
        }
        let sight = self.param(self.bp.kernel_ids().sight_radius);
        u32::try_from(sight).unwrap_or(0).min(self.layout.zone_size())
    }

    /// Returns true when a position can be in the band of a watcher: it is
    /// outside this zone or at most the band from its edge.
    fn near_border(&self, pos: Pos) -> bool {
        let band = self.border_band() as i32;
        let Some(zone) = &self.zone else {
            return false;
        };
        let b = zone.bounds;
        band > 0
            && !(pos.x >= b.x0 + band
                && pos.x < b.x1 - band
                && pos.y >= b.y0 + band
                && pos.y < b.y1 - band)
    }

    /// Marks a resident: its copies in other zones may be out of date.
    pub(super) fn touch_border(&self, idx: Idx) {
        let mut flags = self.flags.borrow_mut();
        let flag = &mut flags[idx.index()];
        if *flag & F_BORDER == 0 {
            *flag |= F_BORDER;
            self.border.dirty.borrow_mut().push(idx.0);
        }
    }

    /// Marks every resident. A change of the sight radius needs it.
    pub(super) fn touch_border_all(&self) {
        if self.border.has_watchers() {
            for idx in self.indices() {
                self.touch_border(idx);
            }
        }
    }

    /// Records that a resident is at a new position, or is a new row.
    pub(super) fn border_moved(&self, idx: Idx, pos: Pos) {
        if !self.border.has_watchers() {
            return;
        }
        let shown = self.border.shown.borrow()[idx.index()];
        if shown != 0 || self.near_border(pos) {
            self.touch_border(idx);
        }
    }

    /// Records that the action of a resident ended. Its copies are out of
    /// date unless it starts the same verb again before the realm settles.
    pub(super) fn border_ended(&self, idx: Idx, verb: VerbId) {
        if self.border.has_watchers() && self.border.shown.borrow()[idx.index()] != 0 {
            self.border.ended.borrow_mut().insert(idx.0, verb);
        }
    }

    /// Records that a resident started an action.
    pub(super) fn border_started(&self, idx: Idx, verb: VerbId) {
        if !self.border.has_watchers() || self.border.shown.borrow()[idx.index()] == 0 {
            return;
        }
        // The same verb as the action that just ended: the copies are right.
        let ended = self.border.ended.borrow_mut().remove(&idx.0);
        if ended != Some(verb) {
            self.touch_border(idx);
        }
    }

    /// Marks the residents whose action ended and that started no new one.
    /// The kernel calls it when the realm has settled.
    pub(super) fn border_settled(&self) {
        if self.border.ended.borrow().is_empty() {
            return;
        }
        let ended = std::mem::take(&mut *self.border.ended.borrow_mut());
        for &i in ended.keys() {
            self.touch_border(Idx(i));
        }
    }

    /// Records, for a resident that arrived from another zone, the zones
    /// that already hold a copy of it under this zone.
    pub(super) fn border_arrived(&self, idx: Idx, holders: &[ZoneId]) {
        let mut mask = 0u32;
        for holder in holders {
            if let Some(k) = self.border.watchers.iter().position(|(zone, _)| zone == holder) {
                mask |= 1 << k;
            }
        }
        if mask != 0 {
            self.border.shown.borrow_mut()[idx.index()] = mask;
            self.touch_border(idx);
        }
    }

    /// Tells the watchers what changed among the marked residents.
    ///
    /// It reads dense indices, so it must run before a row is removed. The
    /// kernel calls it when residents are about to leave and at the end of
    /// every tick. A resident whose removal is not decided yet (its
    /// lifecycle hooks did not run) stays marked.
    pub(super) fn build_border(&self) {
        let border = &self.border;
        if !border.has_watchers() {
            return;
        }
        // A resident can leave without a move. Its copies must go.
        {
            let fates = self.fates.borrow();
            for (&id, fate) in fates.iter() {
                if fate.sealed
                    && let Some(idx) = self.idx_of(id)
                    && border.shown.borrow()[idx.index()] != 0
                {
                    self.touch_border(idx);
                }
            }
        }
        if border.dirty.borrow().is_empty() {
            return;
        }
        let marked = std::mem::take(&mut *border.dirty.borrow_mut());
        let band = self.border_band();
        let here = self.zone_ref();
        let mut news: Vec<Vec<BorderNews>> = border.watchers.iter().map(|_| Vec::new()).collect();
        let mut waiting = Vec::new();
        {
            let mut flags = self.flags.borrow_mut();
            let ids = self.ids.borrow();
            let positions = self.pos.borrow();
            let identity = self.identity.borrow();
            let agenda = self.agenda.borrow();
            let fates = self.fates.borrow();
            let mut shown = border.shown.borrow_mut();
            for &i in &marked {
                let row = i as usize;
                let id = ids[row];
                let next = match fates.get(&id) {
                    Some(fate) if !fate.sealed => {
                        waiting.push(i);
                        continue;
                    }
                    Some(fate) => match fate.kind {
                        FateKind::Migrate(zone) => Next::MovesTo(zone),
                        FateKind::Despawn { .. } => Next::Leaves,
                    },
                    None => Next::Stays,
                };
                flags[row] &= !F_BORDER;
                let pos = positions[row];
                // The watchers that hold a copy after this step. The new
                // home of a resident that moves holds the resident itself.
                let mut new = 0u32;
                if band > 0 && next != Next::Leaves {
                    for (k, (zone, bounds)) in border.watchers.iter().enumerate() {
                        if next != Next::MovesTo(*zone) && bounds.dist_to(pos) <= band {
                            new |= 1 << k;
                        }
                    }
                }
                let old = std::mem::replace(&mut shown[row], 0);
                let activity = agenda[row].current.as_ref().map(|running| running.verb);
                let ghost = || Ghost { pos, name: identity[row].name.clone(), activity };
                let home = match next {
                    Next::MovesTo(zone) => zone,
                    _ => here.id,
                };
                if old | new != 0 {
                    for (k, list) in news.iter_mut().enumerate() {
                        let bit = 1u32 << k;
                        match (old & bit != 0, new & bit != 0) {
                            (false, false) => {}
                            (true, false) => list.push(BorderNews::Leave { id }),
                            (true, true) if next == Next::Stays => {
                                list.push(BorderNews::Update { id, pos, activity });
                            }
                            (_, true) => list.push(BorderNews::Enter { id, home, ghost: ghost() }),
                        }
                    }
                }
                match next {
                    Next::Stays => shown[row] = new,
                    Next::Leaves => {}
                    Next::MovesTo(zone) => {
                        let mut holders: Vec<ZoneId> = border
                            .watchers
                            .iter()
                            .enumerate()
                            .filter(|(k, _)| new & (1 << k) != 0)
                            .map(|(_, (watcher, _))| *watcher)
                            .collect();
                        // This zone keeps a copy too while the resident is
                        // in sight of it.
                        let watched = border.watchers.iter().any(|(watcher, _)| *watcher == zone);
                        if band > 0 && watched && here.bounds.dist_to(pos) <= band {
                            border.ghosts.borrow_mut().insert((zone, id), ghost());
                            holders.push(here.id);
                        }
                        if !holders.is_empty() {
                            border.handoff.borrow_mut().insert(id, holders);
                        }
                    }
                }
            }
        }
        *border.dirty.borrow_mut() = waiting;
        for ((zone, _), list) in border.watchers.iter().zip(news) {
            if !list.is_empty() {
                self.push_kernel(Dest::Zone(*zone), KernelMsg::Border(list));
            }
        }
    }

    /// Takes what another zone tells about its residents near this zone.
    pub(super) fn apply_border(&self, from: ZoneId, news: Vec<BorderNews>) {
        let Some(here) = self.zone.as_ref().map(|zone| zone.id) else {
            return;
        };
        let mut ghosts = self.border.ghosts.borrow_mut();
        for item in news {
            match item {
                BorderNews::Enter { id, home, ghost } => {
                    if home != from {
                        ghosts.remove(&(from, id));
                    }
                    if home != here {
                        ghosts.insert((home, id), ghost);
                    }
                }
                BorderNews::Update { id, pos, activity } => {
                    if let Some(ghost) = ghosts.get_mut(&(from, id)) {
                        ghost.pos = pos;
                        ghost.activity = activity;
                    }
                }
                BorderNews::Leave { id } => {
                    ghosts.remove(&(from, id));
                }
            }
        }
    }
}
