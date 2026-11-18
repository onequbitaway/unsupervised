//! A realm: one single-threaded actor with its own state. See [`Realm`].

mod actions;
mod border;
mod observe;
mod state;
mod tick;

use std::any::{TypeId, type_name};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{
    CityId, Clock, Event, Money, OrgId, OwnerId, Pos, RealmId, Rect, ResidentId, ResidentView,
    Tick, ZoneDelta, ZoneId,
};

use self::border::Border;
pub(crate) use self::border::BorderNews;
pub use self::border::Outsider;
pub(crate) use self::state::{RealmState, Transfer};
use crate::agenda::{Agenda, Inbox, NoticeBody, PackedBody};
use crate::codec;
use crate::column::{AnyColumn, AnyResource, Column, Component, Resource};
use crate::directory::Directory;
use crate::event::{Audience, EventMeta};
use crate::grid::Grid;
use crate::ids::{AttrId, CondId, Idx, ParamId};
use crate::layout::{Layout, ZoneInfo};
use crate::ledger::{Holder, Ledger, LedgerError};
use crate::message::{Body, Dest, Envelope, KernelMsg};
use crate::plugin::{Blueprint, Message, Scope, ScopeDef};
use crate::rng::Rng;
use crate::sites::Sites;
use crate::tiles::TileMap;
use crate::timers::{Timer, TimerKind, TimerWheel};

/// The resident has a running action.
const F_BUSY: u8 = 1;
/// The resident has at least one routine.
const F_ROUTINES: u8 = 2;
/// The position of the resident changed since the last delta.
const F_MOVED: u8 = 4;
/// The activity of the resident changed since the last delta.
const F_ACTIVITY: u8 = 8;
/// The copies of the resident in other zones may be out of date.
const F_BORDER: u8 = 16;

/// The end tick of a condition that has none.
const FOREVER: u64 = u64::MAX;

/// Who a resident is. It never changes after the spawn.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    /// Display name.
    pub name: String,
    /// The owner that controls the resident.
    pub owner: OwnerId,
    /// Name of the model that controls the resident, if the owner gave one.
    pub model: Option<String>,
    /// The frame in which the resident was spawned. The calendar follows
    /// the frame number, so `frame - born` is the age in game minutes.
    pub born: u64,
}

/// One membership of a resident: an organization, a role and the
/// permissions that the role gives.
///
/// The global realm (the module that owns organizations) is the source of
/// truth. The kernel mirrors memberships to the zone of the resident so that
/// zone code can check a permission without a message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Membership {
    /// The organization.
    pub org: OrgId,
    /// Name of the role, for example `owner` or `clerk`.
    pub role: String,
    /// Names of the permissions.
    pub perms: Vec<String>,
}

/// Ids below this number are reserved for well-known things (the nation, the
/// central bank, city governments). [`Realm::alloc_id`] never returns one.
pub const RESERVED_IDS: u64 = 1 << 20;

/// A position change was refused. Nothing changed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PosError {
    /// The position is outside every zone.
    OutOfWorld,
    /// No zone has this id.
    UnknownZone,
    /// The position is not inside the zone that `teleport` names.
    OutsideZone,
    /// The resident is being removed from the zone right now (a lifecycle
    /// hook is running for it).
    Leaving,
}

impl fmt::Display for PosError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PosError::OutOfWorld => "the position is outside every zone",
            PosError::UnknownZone => "no zone has this id",
            PosError::OutsideZone => "the position is not inside the zone",
            PosError::Leaving => "the resident is leaving the zone",
        })
    }
}

impl std::error::Error for PosError {}

/// Why a resident leaves the table at the end of the tick.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FateKind {
    /// The resident moves to another zone.
    Migrate(ZoneId),
    /// The resident leaves the world. Its holdings go to `heir`.
    Despawn { heir: Holder, cause: String },
}

/// The pending removal of a resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fate {
    pub(crate) kind: FateKind,
    /// True after the lifecycle hooks ran. The fate can no longer change.
    #[serde(default)]
    pub(crate) sealed: bool,
}

/// National parameter values, city overrides, and the values that apply in
/// this realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ParamTable {
    national: Vec<i64>,
    #[serde(with = "crate::codec::pairs")]
    overrides: BTreeMap<(CityId, ParamId), i64>,
    /// The value that `param` returns: the override of the city of this
    /// zone, else the national value. Derived, so not saved.
    #[serde(skip)]
    effective: Vec<i64>,
}

impl ParamTable {
    fn new(bp: &Blueprint, city: Option<CityId>) -> ParamTable {
        let mut table = ParamTable {
            national: bp.params().map(|(_, def)| def.value).collect(),
            overrides: BTreeMap::new(),
            effective: Vec::new(),
        };
        table.refresh(bp, city);
        table
    }

    /// Rebuilds the derived values. It also gives parameters that are new
    /// since the snapshot their default.
    fn refresh(&mut self, bp: &Blueprint, city: Option<CityId>) {
        for (id, def) in bp.params() {
            if id.index() >= self.national.len() {
                self.national.push(def.value);
            }
        }
        self.effective = self.national.clone();
        if let Some(city) = city {
            for (&(c, param), &value) in &self.overrides {
                if c == city
                    && let Some(slot) = self.effective.get_mut(param.index())
                {
                    *slot = value;
                }
            }
        }
    }

    fn set(
        &mut self,
        bp: &Blueprint,
        own_city: Option<CityId>,
        param: ParamId,
        city: Option<CityId>,
        value: Option<i64>,
    ) {
        match (city, value) {
            (None, Some(v)) => {
                if let Some(slot) = self.national.get_mut(param.index()) {
                    *slot = v;
                }
            }
            (Some(c), Some(v)) => {
                self.overrides.insert((c, param), v);
            }
            (Some(c), None) => {
                self.overrides.remove(&(c, param));
            }
            (None, None) => {}
        }
        self.refresh(bp, own_city);
    }
}

/// Buffers that the tick reuses so that it does not allocate.
#[derive(Default)]
struct Scratch {
    timers: Vec<Timer>,
    mail: Vec<Envelope>,
    restart: Vec<ResidentId>,
    candidates: Vec<u32>,
}

/// One single-threaded actor with its own state.
///
/// There are two kinds of realm. A *zone* realm holds one district: its
/// tiles, its sites and the residents that are there now. The *global* realm
/// holds state that has no location. Module code reaches all state through
/// a `&Realm`.
///
/// # Interior mutability
///
/// Every method takes `&self`. The realm keeps each column and each
/// singleton in its own cell, so a system can hold several of them at once:
///
/// ```ignore
/// let mut food = realm.attrs_mut(food_attr);
/// let mut needs = realm.col_mut::<Needs>();
/// ```
///
/// Two borrows of the same cell that are not both shared are a bug and
/// panic. Keep borrows short, and drop them before a call that needs the
/// same cell: `attr_set` needs the attribute column, `wire_money` needs the
/// ledger, `set_pos` needs the position column.
///
/// # `Idx` and `ResidentId`
///
/// A resident has a stable [`ResidentId`] and, in its zone, a dense index
/// [`Idx`]. An `Idx` is valid only during the current tick: residents leave
/// the table (migration, despawn) at the end of the tick, and the last row
/// then takes the place of the removed one. New rows are added at the end,
/// so they never move an existing index. State must store `ResidentId`,
/// never `Idx`.
///
/// # One tick
///
/// 1. The tick counter grows by one.
/// 2. The messages in the inbox of the realm are delivered, in order.
/// 3. Timers fire: conditions that end, actions that finish or time out,
///    in `ResidentId` order.
/// 4. Module systems run: stage `Pre`, then `Update`, then `Post`.
/// 5. Routines of idle residents are checked.
/// 6. Residents that leave (migration, despawn) are removed.
/// 7. The zones around this one are told what changed near their border.
/// 8. The ledger is audited.
///
/// After every message, after the timers and after every system the realm
/// *settles*: residents whose action ended start their next action, and
/// event handlers get the events that were emitted.
pub struct Realm {
    id: RealmId,
    scope: Scope,
    /// 0 for the global realm, zone number + 1 for a zone.
    ordinal: u64,
    bp: Arc<Blueprint>,
    layout: Arc<Layout>,
    zone: Option<ZoneInfo>,
    audit_every: u32,

    tick: Cell<u64>,
    frame: Cell<u64>,
    /// True while an observation is built. Writes are a bug then.
    readonly: Cell<bool>,
    rng: RefCell<Rng>,
    next_id: Cell<u64>,
    event_seq: Cell<u64>,
    out_seq: Cell<u64>,

    // The resident table. Every column has one entry per resident.
    ids: RefCell<Vec<ResidentId>>,
    index: RefCell<BTreeMap<ResidentId, u32>>,
    identity: RefCell<Vec<Identity>>,
    pos: RefCell<Vec<Pos>>,
    attrs: Vec<RefCell<Vec<i64>>>,
    conds: RefCell<Vec<Vec<(CondId, u64)>>>,
    comps: Vec<RefCell<Box<dyn AnyColumn>>>,
    agenda: RefCell<Vec<Agenda>>,
    flags: RefCell<Vec<u8>>,
    inbox: RefCell<Vec<Inbox>>,
    members: RefCell<Vec<Vec<Membership>>>,
    grid: RefCell<Grid>,
    fates: RefCell<BTreeMap<ResidentId, Fate>>,
    /// Number of residents with at least one routine.
    routine_holders: Cell<u32>,
    /// Sight across the borders of the zone.
    border: Border,

    // Singletons.
    resources: Vec<RefCell<Box<dyn AnyResource>>>,
    params: RefCell<ParamTable>,
    sites: RefCell<Sites>,
    /// Grows each time the sites are borrowed for a change. The world reads
    /// it to know when its list of spawn sites is out of date. It is not
    /// simulation state.
    sites_epoch: Cell<u64>,
    tiles: RefCell<TileMap>,
    ledger: RefCell<Ledger>,
    directory: RefCell<Directory>,

    // Schedule.
    timers: RefCell<TimerWheel>,
    restart: RefCell<Vec<ResidentId>>,

    // Messages and events.
    mail: RefCell<Vec<Envelope>>,
    outbox: RefCell<Vec<Envelope>>,
    departures: RefCell<Vec<(ZoneId, Transfer)>>,
    /// Residents that left the world since the world last asked.
    despawned: RefCell<Vec<ResidentId>>,
    pending_events: RefCell<VecDeque<Event>>,
    forward: RefCell<Vec<Event>>,

    // Output of the frame. It is not simulation state.
    frame_events: RefCell<Vec<Event>>,
    woke: RefCell<Vec<ResidentId>>,
    delta: RefCell<ZoneDelta>,
    dirty: RefCell<Vec<u32>>,
    /// Messages from outside this process that no handler could take.
    dropped: Cell<u64>,
    scratch: RefCell<Scratch>,
}

impl fmt::Debug for Realm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Realm")
            .field("id", &self.id)
            .field("tick", &self.tick.get())
            .field("residents", &self.len())
            .finish_non_exhaustive()
    }
}

/// Marks a realm read-only while an observation is built, and restores the
/// old mark when it is dropped.
struct ReadGuard<'a> {
    realm: &'a Realm,
    previous: bool,
}

impl<'a> ReadGuard<'a> {
    fn new(realm: &'a Realm) -> ReadGuard<'a> {
        ReadGuard { realm, previous: realm.readonly.replace(true) }
    }
}

impl Drop for ReadGuard<'_> {
    fn drop(&mut self) {
        self.realm.readonly.set(self.previous);
    }
}

impl Realm {
    /// Makes an empty realm.
    pub(crate) fn new(
        id: RealmId,
        bp: Arc<Blueprint>,
        layout: Arc<Layout>,
        seed: u64,
        audit_every: u32,
    ) -> Realm {
        let (scope, ordinal, zone) = match id {
            RealmId::Global => (Scope::Global, 0, None),
            RealmId::Zone(z) => {
                let info = layout.zone(z).cloned().expect("the zone is in the layout");
                (Scope::Zone, u64::from(z.0) + 1, Some(info))
            }
        };
        let def = bp.scope(scope);
        let attr_count = if scope == Scope::Zone { bp.attrs().count() } else { 0 };
        let bounds = zone.as_ref().map(|z| z.bounds).unwrap_or_default();
        let city = zone.as_ref().map(|z| z.city);
        let border = Border::new(&layout, zone.as_ref().map(|z| z.id));
        Realm {
            id,
            scope,
            ordinal,
            zone,
            audit_every,
            tick: Cell::new(0),
            frame: Cell::new(0),
            readonly: Cell::new(false),
            rng: RefCell::new(Rng::stream(seed, ordinal)),
            next_id: Cell::new(RESERVED_IDS),
            event_seq: Cell::new(0),
            out_seq: Cell::new(0),
            ids: RefCell::new(Vec::new()),
            index: RefCell::new(BTreeMap::new()),
            identity: RefCell::new(Vec::new()),
            pos: RefCell::new(Vec::new()),
            attrs: (0..attr_count).map(|_| RefCell::new(Vec::new())).collect(),
            conds: RefCell::new(Vec::new()),
            comps: def.components.iter().map(|c| RefCell::new((c.make)())).collect(),
            agenda: RefCell::new(Vec::new()),
            flags: RefCell::new(Vec::new()),
            inbox: RefCell::new(Vec::new()),
            members: RefCell::new(Vec::new()),
            grid: RefCell::new(Grid::new(bounds)),
            fates: RefCell::new(BTreeMap::new()),
            routine_holders: Cell::new(0),
            border,
            resources: def.resources.iter().map(|r| RefCell::new((r.make)())).collect(),
            params: RefCell::new(ParamTable::new(&bp, city)),
            sites: RefCell::new(Sites::default()),
            sites_epoch: Cell::new(0),
            tiles: RefCell::new(TileMap::new(bounds)),
            ledger: RefCell::new(Ledger::new()),
            directory: RefCell::new(Directory::default()),
            timers: RefCell::new(TimerWheel::default()),
            restart: RefCell::new(Vec::new()),
            mail: RefCell::new(Vec::new()),
            outbox: RefCell::new(Vec::new()),
            departures: RefCell::new(Vec::new()),
            despawned: RefCell::new(Vec::new()),
            pending_events: RefCell::new(VecDeque::new()),
            forward: RefCell::new(Vec::new()),
            frame_events: RefCell::new(Vec::new()),
            woke: RefCell::new(Vec::new()),
            delta: RefCell::new(ZoneDelta::default()),
            dirty: RefCell::new(Vec::new()),
            dropped: Cell::new(0),
            scratch: RefCell::new(Scratch::default()),
            bp,
            layout,
        }
    }

    /// Panics, in debug builds, when state is written while an observation
    /// is built. Reads (`observe`, `view`) must not change state.
    #[inline]
    fn guard_write(&self) {
        debug_assert!(!self.readonly.get(), "an observer must not change state");
    }

    fn def(&self) -> &ScopeDef {
        self.bp.scope(self.scope)
    }

    /// Returns the zone description.
    ///
    /// # Panics
    ///
    /// Panics in the global realm, which has no residents and no map.
    fn zone_ref(&self) -> &ZoneInfo {
        match &self.zone {
            Some(zone) => zone,
            None => panic!("this call needs a zone realm; the global realm has no residents"),
        }
    }

    // ---- Identity, time, randomness -------------------------------------

    /// Returns the address of the realm.
    #[must_use]
    pub fn id(&self) -> RealmId {
        self.id
    }

    /// Returns the kind of the realm.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    /// Returns the bounds, the city and the name of the zone, or `None` in
    /// the global realm.
    #[must_use]
    pub fn zone_info(&self) -> Option<&ZoneInfo> {
        self.zone.as_ref()
    }

    /// Returns the cities and zones of the world: bounds, `zone_at(pos)` and
    /// neighbours.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Returns the local tick. One tick is one game minute.
    ///
    /// During a tick it is the number of the tick that runs. Between frames
    /// it is the number of the last tick that ran. Action durations and
    /// condition end ticks use it. A zone that skips a frame (time dilation)
    /// falls behind the frame number, so do not use the tick as a calendar.
    #[must_use]
    pub fn tick(&self) -> Tick {
        Tick(self.tick.get())
    }

    /// Returns the number of the frame that runs, or of the last frame
    /// between frames. It is the same in every realm.
    #[must_use]
    pub fn frame(&self) -> u64 {
        self.frame.get()
    }

    /// Returns the calendar time. It follows the frame number.
    #[must_use]
    pub fn clock(&self) -> Clock {
        self.layout.clock_at(self.frame.get())
    }

    /// Returns the random number generator of the realm. All randomness of
    /// the simulation comes from here.
    pub fn rng(&self) -> RefMut<'_, Rng> {
        self.guard_write();
        self.rng.borrow_mut()
    }

    /// Returns a new id that no realm of the world has returned before.
    ///
    /// The id is never below [`RESERVED_IDS`] and is below 2^53, so it
    /// survives a JSON number in every client and never collides with a
    /// well-known id such as `OrgId::NATION`. Use it for sites,
    /// organizations, accounts, escrows and anything else that needs an id.
    pub fn alloc_id(&self) -> u64 {
        self.guard_write();
        let counter = self.next_id.get();
        assert!(counter < 1 << 40, "{} ran out of ids", self.id);
        self.next_id.set(counter + 1);
        (self.ordinal << 40) | counter
    }

    // ---- Residents (zone realms) ------------------------------------------

    /// Returns the number of residents in the zone. It is 0 in the global
    /// realm.
    #[must_use]
    pub fn len(&self) -> usize {
        self.ids.borrow().len()
    }

    /// Returns true when the zone has no resident.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.ids.borrow().is_empty()
    }

    /// Returns every dense index of the zone, in order.
    pub fn indices(&self) -> impl Iterator<Item = Idx> + use<> {
        (0..self.len() as u32).map(Idx)
    }

    /// Returns the dense index of a resident, or `None` when the resident is
    /// not in this zone.
    #[must_use]
    pub fn idx_of(&self, id: ResidentId) -> Option<Idx> {
        self.index.borrow().get(&id).map(|&i| Idx(i))
    }

    /// Returns the id of the resident at a dense index.
    ///
    /// # Panics
    ///
    /// Panics when the index is past the end of the table.
    #[must_use]
    pub fn id_of(&self, idx: Idx) -> ResidentId {
        self.ids.borrow()[idx.index()]
    }

    /// Returns the ids of all residents in index order.
    #[must_use]
    pub fn ids(&self) -> Ref<'_, [ResidentId]> {
        Ref::map(self.ids.borrow(), Vec::as_slice)
    }

    /// Returns the name, the owner, the model and the birth frame of a
    /// resident.
    #[must_use]
    pub fn identity(&self, idx: Idx) -> Ref<'_, Identity> {
        Ref::map(self.identity.borrow(), |v| &v[idx.index()])
    }

    fn comp_cell<T: Component>(&self) -> &RefCell<Box<dyn AnyColumn>> {
        match self.bp.zone.component_slot(TypeId::of::<T>()) {
            Some(slot) if self.scope == Scope::Zone => &self.comps[slot],
            Some(_) => panic!("the global realm has no component columns"),
            None => panic!("component `{}` ({}) is not registered", T::NAME, type_name::<T>()),
        }
    }

    /// Borrows a component column.
    ///
    /// # Panics
    ///
    /// Panics when the component is not registered, in the global realm, or
    /// when the column is mutably borrowed.
    #[must_use]
    pub fn col<T: Component>(&self) -> Ref<'_, Column<T>> {
        Ref::map(self.comp_cell::<T>().borrow(), |column| {
            column
                .as_any()
                .downcast_ref::<Column<T>>()
                .expect("the slot holds a column of this type")
        })
    }

    /// Borrows a component column for a change.
    ///
    /// # Panics
    ///
    /// Panics when the component is not registered, in the global realm, or
    /// when the column is borrowed.
    pub fn col_mut<T: Component>(&self) -> RefMut<'_, Column<T>> {
        self.guard_write();
        RefMut::map(self.comp_cell::<T>().borrow_mut(), |column| {
            column
                .as_any_mut()
                .downcast_mut::<Column<T>>()
                .expect("the slot holds a column of this type")
        })
    }

    /// Returns the position of a resident.
    #[must_use]
    pub fn pos(&self, idx: Idx) -> Pos {
        self.pos.borrow()[idx.index()]
    }

    /// Returns the positions of all residents in index order.
    #[must_use]
    pub fn positions(&self) -> Ref<'_, [Pos]> {
        Ref::map(self.pos.borrow(), Vec::as_slice)
    }

    /// Moves a resident.
    ///
    /// A position inside this zone moves the resident at once. A position
    /// in another zone also changes the position at once and marks the
    /// resident for migration: it leaves this table at the end of the tick
    /// and is in the other zone before the next frame, with all its
    /// components, attributes, conditions, agenda, inbox and holdings. A
    /// position outside every zone is refused.
    pub fn set_pos(&self, idx: Idx, pos: Pos) -> Result<(), PosError> {
        self.guard_write();
        let here = self.zone_ref();
        let target = if here.bounds.contains(pos) {
            here.id
        } else {
            self.layout.zone_at(pos).ok_or(PosError::OutOfWorld)?
        };
        self.move_to(idx, target, pos)
    }

    /// Moves a resident to a position in a named zone, for example at the
    /// end of a journey between cities. It works like [`Realm::set_pos`];
    /// the position must be inside the zone.
    pub fn teleport(&self, idx: Idx, zone: ZoneId, pos: Pos) -> Result<(), PosError> {
        self.guard_write();
        let _ = self.zone_ref();
        let info = self.layout.zone(zone).ok_or(PosError::UnknownZone)?;
        if !info.bounds.contains(pos) {
            return Err(PosError::OutsideZone);
        }
        self.move_to(idx, zone, pos)
    }

    fn move_to(&self, idx: Idx, target: ZoneId, pos: Pos) -> Result<(), PosError> {
        let home = self.zone_ref().id;
        let id = self.id_of(idx);
        {
            let mut fates = self.fates.borrow_mut();
            match fates.get(&id) {
                Some(fate) if fate.sealed => return Err(PosError::Leaving),
                Some(Fate { kind: FateKind::Despawn { .. }, .. }) => {}
                _ if target == home => {
                    fates.remove(&id);
                }
                _ => {
                    fates.insert(id, Fate { kind: FateKind::Migrate(target), sealed: false });
                }
            }
        }
        let old = std::mem::replace(&mut self.pos.borrow_mut()[idx.index()], pos);
        if old != pos {
            self.grid.borrow_mut().relocate(idx.0, old, pos);
            self.mark_dirty(idx, F_MOVED);
            self.border_moved(idx, pos);
        }
        Ok(())
    }

    /// Returns the residents of this zone within `radius` tiles of `pos`,
    /// sorted by `ResidentId`.
    ///
    /// The distance is [`Pos::dist`]: a diagonal step counts as one. Only
    /// residents of this zone are found, also near a zone border: an `Idx`
    /// names a row of this zone. [`Realm::nearby_outside`] gives the
    /// residents on the other side of a border.
    #[must_use]
    pub fn nearby(&self, pos: Pos, radius: u32) -> Vec<Idx> {
        let mut out = Vec::new();
        self.nearby_into(pos, radius, &mut out);
        out
    }

    /// Like [`Realm::nearby`], but writes the result to a buffer that the
    /// caller reuses. The buffer is cleared first.
    pub fn nearby_into(&self, pos: Pos, radius: u32, out: &mut Vec<Idx>) {
        out.clear();
        let positions = self.pos.borrow();
        self.grid.borrow().query(pos, radius, &positions, out);
        let ids = self.ids.borrow();
        out.sort_unstable_by_key(|idx| ids[idx.index()]);
    }

    /// Returns the `k` residents of this zone that are nearest to `pos`,
    /// among those within `radius` tiles: nearest first, and residents at
    /// the same distance in `ResidentId` order.
    ///
    /// The distance is [`Pos::dist`]. A resident that stands on `pos` is in
    /// the list, at distance zero. Like [`Realm::nearby`], it finds only
    /// residents of this zone.
    ///
    /// Use it when a few neighbours are enough. It does not collect or sort
    /// the residents in sight: it searches outward from `pos` and stops
    /// when it has the `k` nearest, so a crowd around `pos` costs little.
    /// Residents that share one tile are the exception: the kernel must
    /// read each of them to find the lowest ids.
    #[must_use]
    pub fn nearest(&self, pos: Pos, radius: u32, k: usize) -> Vec<Idx> {
        let mut found = Vec::new();
        self.grid.borrow().nearest(
            pos,
            radius,
            k,
            &self.pos.borrow(),
            &self.ids.borrow(),
            &mut found,
        );
        found.into_iter().map(|near| Idx(near.idx)).collect()
    }

    fn attr_cell(&self, a: AttrId) -> &RefCell<Vec<i64>> {
        match self.attrs.get(a.index()) {
            Some(cell) => cell,
            None if self.scope == Scope::Global => {
                panic!("the global realm has no attribute columns")
            }
            None => panic!("attribute id {} is not from this world", a.0),
        }
    }

    /// Returns an attribute of a resident.
    #[must_use]
    pub fn attr(&self, idx: Idx, a: AttrId) -> i64 {
        self.attr_cell(a).borrow()[idx.index()]
    }

    /// Sets an attribute of a resident. The value is clamped to the bounds
    /// of the attribute. Returns the new value.
    pub fn attr_set(&self, idx: Idx, a: AttrId, v: i64) -> i64 {
        self.guard_write();
        let (min, max) = self.bp.attr_bounds(a);
        let v = v.clamp(min, max);
        self.attr_cell(a).borrow_mut()[idx.index()] = v;
        v
    }

    /// Adds to an attribute of a resident. The result is clamped to the
    /// bounds of the attribute. Returns the new value.
    pub fn attr_add(&self, idx: Idx, a: AttrId, d: i64) -> i64 {
        self.guard_write();
        let (min, max) = self.bp.attr_bounds(a);
        let mut column = self.attr_cell(a).borrow_mut();
        let slot = &mut column[idx.index()];
        *slot = slot.saturating_add(d).clamp(min, max);
        *slot
    }

    /// Borrows the whole column of an attribute, in index order.
    #[must_use]
    pub fn attrs(&self, a: AttrId) -> Ref<'_, [i64]> {
        Ref::map(self.attr_cell(a).borrow(), Vec::as_slice)
    }

    /// Borrows the whole column of an attribute for a change. The kernel
    /// does not clamp what a system writes here; keep the values inside the
    /// bounds of the attribute.
    pub fn attrs_mut(&self, a: AttrId) -> RefMut<'_, [i64]> {
        self.guard_write();
        RefMut::map(self.attr_cell(a).borrow_mut(), Vec::as_mut_slice)
    }

    /// Returns true when a resident has a condition now.
    #[must_use]
    pub fn has_condition(&self, idx: Idx, c: CondId) -> bool {
        let tick = self.tick.get();
        self.conds.borrow()[idx.index()].iter().any(|&(id, until)| id == c && until > tick)
    }

    /// Returns the end tick of a condition of a resident: `None` when the
    /// resident does not have it, `Some(None)` when it has no end.
    #[must_use]
    pub fn condition_until(&self, idx: Idx, c: CondId) -> Option<Option<Tick>> {
        let tick = self.tick.get();
        self.conds.borrow()[idx.index()]
            .iter()
            .find(|&&(id, until)| id == c && until > tick)
            .map(|&(_, until)| (until != FOREVER).then_some(Tick(until)))
    }

    /// Returns the conditions that a resident has now, with their end ticks.
    #[must_use]
    pub fn conditions(&self, idx: Idx) -> Vec<(CondId, Option<Tick>)> {
        let tick = self.tick.get();
        self.conds.borrow()[idx.index()]
            .iter()
            .filter(|&&(_, until)| until > tick)
            .map(|&(id, until)| (id, (until != FOREVER).then_some(Tick(until))))
            .collect()
    }

    /// Gives a resident a condition. With `until`, the condition ends by
    /// itself when the zone reaches that tick; an end tick that is not in
    /// the future clears the condition. Setting a condition again replaces
    /// its end tick.
    ///
    /// When a condition ends, by time or by [`Realm::clear_condition`], the
    /// kernel emits the silent event `kernel.condition_ended` with
    /// `{ resident, condition, expired }`.
    pub fn set_condition(&self, idx: Idx, c: CondId, until: Option<Tick>) {
        self.guard_write();
        let tick = self.tick.get();
        let until = until.map_or(FOREVER, |t| t.0);
        if until <= tick {
            self.clear_condition(idx, c);
            return;
        }
        {
            let mut conds = self.conds.borrow_mut();
            let list = &mut conds[idx.index()];
            match list.iter_mut().find(|(id, _)| *id == c) {
                Some(entry) => entry.1 = until,
                None => list.push((c, until)),
            }
        }
        if until != FOREVER {
            let timer = Timer {
                resident: self.id_of(idx),
                kind: TimerKind::Condition,
                key: u64::from(c.0),
            };
            self.timers.borrow_mut().insert(tick, until, timer);
        }
    }

    /// Takes a condition from a resident. It does nothing when the resident
    /// does not have the condition.
    pub fn clear_condition(&self, idx: Idx, c: CondId) {
        self.guard_write();
        self.end_condition(idx, c, false);
    }

    fn end_condition(&self, idx: Idx, c: CondId, expired: bool) {
        let removed = {
            let mut conds = self.conds.borrow_mut();
            let list = &mut conds[idx.index()];
            let before = list.len();
            list.retain(|(id, _)| *id != c);
            list.len() != before
        };
        if removed {
            let resident = self.id_of(idx);
            self.emit(
                EventMeta::silent("kernel.condition_ended").actor(resident),
                &serde_json::json!({
                    "resident": resident,
                    "condition": self.bp.condition(c).name,
                    "expired": expired,
                }),
            );
        }
    }

    /// Returns true when a resident is a member of `org` with a role that
    /// gives the permission.
    #[must_use]
    pub fn can(&self, idx: Idx, org: OrgId, perm: &str) -> bool {
        self.members.borrow()[idx.index()]
            .iter()
            .any(|m| m.org == org && m.perms.iter().any(|p| p == perm))
    }

    /// Returns an organization in which a resident has the permission: the
    /// one with the lowest id.
    #[must_use]
    pub fn has_perm(&self, idx: Idx, perm: &str) -> Option<OrgId> {
        self.members.borrow()[idx.index()]
            .iter()
            .filter(|m| m.perms.iter().any(|p| p == perm))
            .map(|m| m.org)
            .min()
    }

    /// Returns the memberships of a resident, ordered by organization.
    #[must_use]
    pub fn memberships(&self, idx: Idx) -> Ref<'_, [Membership]> {
        Ref::map(self.members.borrow(), |v| v[idx.index()].as_slice())
    }

    /// Makes a resident a member of an organization, or replaces its role
    /// and permissions there. It works from every realm: the kernel routes
    /// the change to the zone of the resident, where it applies at the next
    /// tick (at once when the resident is in this zone).
    pub fn grant(&self, resident: ResidentId, org: OrgId, role: &str, perms: &[&str]) {
        self.guard_write();
        let membership = Membership {
            org,
            role: role.to_string(),
            perms: perms.iter().map(|p| (*p).to_string()).collect(),
        };
        self.set_membership(resident, org, Some(membership));
    }

    /// Ends the membership of a resident in an organization. It works from
    /// every realm, like [`Realm::grant`].
    pub fn revoke(&self, resident: ResidentId, org: OrgId) {
        self.guard_write();
        self.set_membership(resident, org, None);
    }

    fn set_membership(&self, resident: ResidentId, org: OrgId, grant: Option<Membership>) {
        match self.idx_of(resident) {
            Some(idx) => {
                let mut members = self.members.borrow_mut();
                let list = &mut members[idx.index()];
                list.retain(|m| m.org != org);
                if let Some(membership) = grant {
                    let at = list.partition_point(|m| m.org < org);
                    list.insert(at, membership);
                }
            }
            None => {
                self.push_kernel(
                    Dest::Resident(resident),
                    KernelMsg::Membership { resident, org, grant },
                );
            }
        }
    }

    /// Removes a resident from the world at the end of the tick.
    ///
    /// Before the removal the `on_despawn` hooks run. The running action
    /// stops. Everything that the resident holds goes to `heir`: to the
    /// ledger of this zone when the heir is a site or a resident of this
    /// zone, to the zone of the heir when it is a resident elsewhere, and to
    /// the ledger of the global realm for an organization, an account or an
    /// escrow. The kernel emits the public event `kernel.despawned` with
    /// `{ resident, name, cause }`.
    pub fn despawn(&self, idx: Idx, heir: Holder, cause: &str) {
        self.guard_write();
        let id = self.id_of(idx);
        let mut fates = self.fates.borrow_mut();
        if fates.get(&id).is_some_and(|f| f.sealed) {
            return;
        }
        fates.insert(
            id,
            Fate { kind: FateKind::Despawn { heir, cause: cause.to_string() }, sealed: false },
        );
    }

    // ---- Singletons and registries ----------------------------------------

    fn res_cell<T: Resource>(&self) -> &RefCell<Box<dyn AnyResource>> {
        match self.def().resource_slot(TypeId::of::<T>()) {
            Some(slot) => &self.resources[slot],
            None => panic!(
                "resource `{}` ({}) is not registered for the {:?} scope",
                T::NAME,
                type_name::<T>(),
                self.scope
            ),
        }
    }

    /// Borrows a resource.
    ///
    /// # Panics
    ///
    /// Panics when the resource is not registered for the scope of this
    /// realm, or when it is mutably borrowed.
    #[must_use]
    pub fn res<T: Resource>(&self) -> Ref<'_, T> {
        Ref::map(self.res_cell::<T>().borrow(), |r| {
            r.as_any().downcast_ref::<T>().expect("the slot holds a resource of this type")
        })
    }

    /// Borrows a resource for a change.
    ///
    /// # Panics
    ///
    /// Panics when the resource is not registered for the scope of this
    /// realm, or when it is borrowed.
    pub fn res_mut<T: Resource>(&self) -> RefMut<'_, T> {
        self.guard_write();
        RefMut::map(self.res_cell::<T>().borrow_mut(), |r| {
            r.as_any_mut().downcast_mut::<T>().expect("the slot holds a resource of this type")
        })
    }

    /// Returns the value of a parameter in this realm: in a zone, the
    /// override of its city when there is one, else the national value; in
    /// the global realm, the national value.
    #[must_use]
    pub fn param(&self, p: ParamId) -> i64 {
        self.params.borrow().effective[p.index()]
    }

    /// Returns the value of a parameter in a city: its override when there
    /// is one, else the national value.
    #[must_use]
    pub fn param_in(&self, p: ParamId, city: CityId) -> i64 {
        let params = self.params.borrow();
        params.overrides.get(&(city, p)).copied().unwrap_or(params.national[p.index()])
    }

    /// Changes a parameter: the national value, or the override of one
    /// city. Only the global realm can do it. The value is clamped to the
    /// range of the parameter. The global realm sees the change at once;
    /// zones see it in the next frame. Returns the value that was set.
    ///
    /// # Panics
    ///
    /// Panics in a zone realm.
    pub fn set_param(&self, p: ParamId, v: i64, city: Option<CityId>) -> i64 {
        self.guard_write();
        assert!(self.scope == Scope::Global, "only the global realm can change a parameter");
        let def = self.bp.param(p);
        let v = v.clamp(def.min, def.max);
        self.params.borrow_mut().set(&self.bp, None, p, city, Some(v));
        self.push_kernel(Dest::AllZones, KernelMsg::ParamSet { param: p, city, value: Some(v) });
        v
    }

    /// Removes the override of a parameter for one city, so that the city
    /// uses the national value again. Only the global realm can do it.
    ///
    /// # Panics
    ///
    /// Panics in a zone realm.
    pub fn clear_param(&self, p: ParamId, city: CityId) {
        self.guard_write();
        assert!(self.scope == Scope::Global, "only the global realm can change a parameter");
        self.params.borrow_mut().set(&self.bp, None, p, Some(city), None);
        self.push_kernel(
            Dest::AllZones,
            KernelMsg::ParamSet { param: p, city: Some(city), value: None },
        );
    }

    /// Returns the definitions of the world: parameters, items, attributes,
    /// conditions, faucets, sinks and verbs.
    #[must_use]
    pub fn defs(&self) -> &Blueprint {
        &self.bp
    }

    /// Borrows the sites of the zone. The global realm has none.
    #[must_use]
    pub fn sites(&self) -> Ref<'_, Sites> {
        self.sites.borrow()
    }

    /// Borrows the sites of the zone for a change.
    pub fn sites_mut(&self) -> RefMut<'_, Sites> {
        self.guard_write();
        self.sites_epoch.set(self.sites_epoch.get().wrapping_add(1));
        self.sites.borrow_mut()
    }

    /// Returns a number that grows each time the sites of the zone are
    /// borrowed for a change or replaced.
    pub(crate) fn sites_epoch(&self) -> u64 {
        self.sites_epoch.get()
    }

    /// Returns the tile of a spawn site on which its newcomer number
    /// `turn`, from 0, appears.
    ///
    /// The tiles of the site with a code of the parameter
    /// `kernel.spawn_tiles` (the walkable codes) take turns, row by row.
    /// The first turn has the centre of the site, or the next such tile
    /// after it; after the last tile comes the first. A site without such a
    /// tile gives its centre.
    pub(crate) fn spawn_tile(&self, site: Rect, turn: u64) -> Pos {
        let kinds = self.param(self.bp.kernel_ids().spawn_tiles).cast_unsigned();
        let tiles = self.tiles.borrow();
        let centre = site.centre();
        let free = tiles.count_in(site, kinds) as u64;
        if free == 0 {
            return centre;
        }
        // The tiles that come before the centre: the rows above it, and
        // the tiles to its left in its own row.
        let above = Rect::new(site.x0, site.y0, site.x1, centre.y);
        let left = Rect::new(site.x0, centre.y, centre.x, centre.y.saturating_add(1));
        let before = (tiles.count_in(above, kinds) + tiles.count_in(left, kinds)) as u64;
        let number = (before + turn % free) % free;
        tiles.nth_in(site, kinds, number as usize).unwrap_or(centre)
    }

    /// Borrows the tile map of the zone. The global realm has an empty map.
    #[must_use]
    pub fn tiles(&self) -> Ref<'_, TileMap> {
        self.tiles.borrow()
    }

    /// Borrows the tile map of the zone for a change.
    pub fn tiles_mut(&self) -> RefMut<'_, TileMap> {
        self.guard_write();
        self.tiles.borrow_mut()
    }

    /// Borrows the ledger of the realm.
    #[must_use]
    pub fn ledger(&self) -> Ref<'_, Ledger> {
        self.ledger.borrow()
    }

    /// Borrows the ledger of the realm for a change.
    pub fn ledger_mut(&self) -> RefMut<'_, Ledger> {
        self.guard_write();
        self.ledger.borrow_mut()
    }

    /// Borrows the directory of residents: zone, owner, name and whether the
    /// resident is alive. Only the global realm has entries; in a zone the
    /// directory is empty.
    #[must_use]
    pub fn directory(&self) -> Ref<'_, Directory> {
        self.directory.borrow()
    }

    // ---- Output -----------------------------------------------------------

    /// Emits an event.
    ///
    /// Viewers see events with audience `Public` or `Nearby`. Residents get
    /// a notice as the audience says; the notice has the kind, the actor and
    /// the payload of the event. A `Nearby` audience also reaches the
    /// residents on the other side of a zone border that this zone knows of
    /// ([`Realm::nearby_outside`]); their notice arrives in the next frame.
    /// Event handlers of this realm run later in the same tick. Handlers of
    /// the global realm get zone events in the same frame.
    ///
    /// An event that nobody would receive costs almost nothing: the payload
    /// is not serialized.
    ///
    /// # Panics
    ///
    /// Panics when the payload cannot be written as JSON.
    pub fn emit<P: Serialize>(&self, meta: EventMeta, payload: &P) {
        self.guard_write();
        let public = matches!(meta.audience, Audience::Public | Audience::Nearby(_));
        let notices = matches!(meta.audience, Audience::Nearby(_) | Audience::Private(_));
        let local = self.def().wants_event(meta.kind);
        let forward = self.scope == Scope::Zone && self.bp.global.wants_event(meta.kind);
        if !(public || notices || local || forward) {
            return;
        }
        let data = match serde_json::to_value(payload) {
            Ok(data) => data,
            Err(e) => panic!("the payload of event `{}` is not JSON: {e}", meta.kind),
        };
        let seq = self.event_seq.get() + 1;
        self.event_seq.set(seq);
        let tick = self.tick.get();

        match &meta.audience {
            Audience::Nearby(radius) if self.scope == Scope::Zone => {
                let actor_idx = meta.actor.and_then(|a| self.idx_of(a));
                let place = meta.pos.or_else(|| actor_idx.map(|idx| self.pos(idx)));
                if let Some(place) = place {
                    // One body for everybody that hears it.
                    let body = PackedBody::new(&NoticeBody::Notice {
                        tick,
                        kind: meta.kind.into(),
                        from: meta.actor,
                        data: data.clone(),
                    });
                    for idx in self.nearby(place, *radius) {
                        if Some(idx) != actor_idx {
                            self.push_inbox(idx, body.clone());
                        }
                    }
                    // The residents on the other side of a border that this
                    // zone knows of. The notice follows each one to
                    // wherever it is in the next frame.
                    let mut outside = Vec::new();
                    self.for_each_outsider(place, *radius, |_, id, _| {
                        if Some(id) != meta.actor {
                            outside.push(id);
                        }
                    });
                    for id in outside {
                        self.push_kernel(
                            Dest::Resident(id),
                            KernelMsg::Notice {
                                to: id,
                                kind: meta.kind.to_string(),
                                from: meta.actor,
                                data: data.clone(),
                            },
                        );
                    }
                }
            }
            Audience::Private(list) => {
                for &resident in list {
                    self.notify(resident, meta.kind, meta.actor, data.clone());
                }
            }
            _ => {}
        }

        if !(public || local || forward) {
            return;
        }
        let event = Event {
            seq,
            frame: self.frame.get(),
            tick: Tick(tick),
            realm: self.id,
            kind: meta.kind.to_string(),
            actor: meta.actor,
            pos: meta.pos,
            public,
            drama: meta.drama.min(100),
            data,
        };
        if public {
            self.frame_events.borrow_mut().push(event.clone());
        }
        if forward {
            self.forward.borrow_mut().push(event.clone());
        }
        if local {
            self.pending_events.borrow_mut().push_back(event);
        }
    }

    /// Writes a notice to the inbox of a resident. It works for a resident
    /// in any zone and from the global realm: the kernel routes the notice,
    /// and it arrives in the next frame when the resident is elsewhere. A
    /// notice for a resident that no longer exists is dropped.
    ///
    /// Text that a resident wrote must not be a plain string in `data`. Wrap
    /// it in `unsup_protocol::Untrusted`.
    pub fn notify(&self, to: ResidentId, kind: &str, from: Option<ResidentId>, data: Value) {
        self.guard_write();
        match self.idx_of(to) {
            Some(idx) => {
                let body = PackedBody::new(&NoticeBody::Notice {
                    tick: self.tick.get(),
                    kind: kind.into(),
                    from,
                    data,
                });
                self.push_inbox(idx, body);
            }
            None => {
                self.push_kernel(
                    Dest::Resident(to),
                    KernelMsg::Notice { to, kind: kind.to_string(), from, data },
                );
            }
        }
    }

    /// Sends a typed message to another realm. The router delivers it at
    /// the frame boundary: a message from a zone to the global realm arrives
    /// in the same frame, every other message arrives in the next frame.
    /// Handlers registered with `Builder::on_message` (or
    /// `on_resident_message` for `Dest::Resident`) receive it.
    pub fn send<M: Message>(&self, to: Dest, msg: &M) {
        self.guard_write();
        self.bp.check_message_type::<M>();
        let payload = codec::encode(M::KIND, msg);
        self.push_out(to, Body::Module { kind: std::borrow::Cow::Borrowed(M::KIND), payload });
    }

    /// Checks that a wire can reach a ledger.
    fn check_wire_dest(&self, to: Dest) -> Result<(), LedgerError> {
        match to {
            Dest::AllZones => Err(LedgerError::BadDestination),
            Dest::Zone(zone) if self.layout.zone(zone).is_none() => {
                Err(LedgerError::BadDestination)
            }
            _ => Ok(()),
        }
    }

    /// Sends money from a holder of this realm to a holder of another
    /// realm. The money leaves `from` at once and reaches `to_holder` when
    /// the message is delivered; `World::audit` counts it in between.
    ///
    /// A wire to a resident follows the resident when it moved. A wire to a
    /// resident that no longer exists goes to [`UNCLAIMED`](crate::UNCLAIMED)
    /// in the global realm. A failed wire changes nothing.
    pub fn wire_money(
        &self,
        from: Holder,
        to: Dest,
        to_holder: Holder,
        amount: Money,
        reason: &str,
    ) -> Result<(), LedgerError> {
        self.guard_write();
        self.check_wire_dest(to)?;
        self.ledger.borrow_mut().wire_out_money(from, amount.0, reason)?;
        if amount.0 > 0 {
            self.push_kernel(to, KernelMsg::WireMoney { to: to_holder, amount: amount.0 });
        }
        Ok(())
    }

    /// Sends goods from a holder of this realm to a holder of another
    /// realm. The rules are those of [`Realm::wire_money`].
    pub fn wire_item(
        &self,
        from: Holder,
        to: Dest,
        to_holder: Holder,
        item: unsup_protocol::ItemId,
        qty: i64,
        reason: &str,
    ) -> Result<(), LedgerError> {
        self.guard_write();
        self.check_wire_dest(to)?;
        self.ledger.borrow_mut().wire_out_item(from, item, qty, reason)?;
        if qty > 0 {
            self.push_kernel(to, KernelMsg::WireItem { to: to_holder, item, qty });
        }
        Ok(())
    }

    // ---- Kernel internals ---------------------------------------------------

    fn push_out(&self, to: Dest, body: Body) {
        let seq = self.out_seq.get() + 1;
        self.out_seq.set(seq);
        self.outbox.borrow_mut().push(Envelope::new(self.id, seq, to, body));
    }

    fn push_kernel(&self, to: Dest, msg: KernelMsg) {
        self.push_out(to, Body::Kernel(Box::new(msg)));
    }

    /// Adds an entry to the inbox of a resident of this zone and records
    /// that the resident has news.
    fn push_inbox(&self, idx: Idx, body: PackedBody) {
        let max = usize::try_from(self.param(self.bp.kernel_ids().inbox_max)).unwrap_or(1);
        self.inbox.borrow_mut()[idx.index()].push(body, max);
        self.woke.borrow_mut().push(self.id_of(idx));
    }

    /// Records that the position or the activity of a resident changed, for
    /// the delta that viewers get.
    fn mark_dirty(&self, idx: Idx, flag: u8) {
        let mut flags = self.flags.borrow_mut();
        let slot = &mut flags[idx.index()];
        if *slot & (F_MOVED | F_ACTIVITY) == 0 {
            self.dirty.borrow_mut().push(idx.0);
        }
        *slot |= flag;
    }

    /// Returns the public view of a resident.
    pub(crate) fn resident_view(&self, idx: Idx) -> ResidentView {
        let identity = self.identity.borrow();
        let who = &identity[idx.index()];
        ResidentView {
            id: self.id_of(idx),
            name: who.name.clone(),
            pos: self.pos(idx),
            activity: self.activity(idx).map(|a| a.verb.to_string()),
            model: who.model.clone(),
        }
    }

    /// Returns an estimate of the bytes that the resident table of this
    /// realm uses: the columns, and what the kernel knows that they point
    /// to. Heap data inside module components is not counted.
    pub(crate) fn memory_estimate(&self) -> usize {
        let n = self.ids.borrow().capacity();
        let fixed = n
            * (size_of::<ResidentId>()
                + size_of::<Identity>()
                + size_of::<Pos>()
                + size_of::<Vec<(CondId, u64)>>()
                + size_of::<Agenda>()
                + 1
                + size_of::<Inbox>()
                + size_of::<Vec<Membership>>());
        let index = self.index.borrow().len() * 24;
        let attrs = self.attrs.len() * n * 8;
        let comps: usize = self.comps.iter().map(|c| c.borrow().memory_estimate()).sum();
        let names: usize = self
            .identity
            .borrow()
            .iter()
            .map(|i| i.name.capacity() + i.model.as_ref().map_or(0, String::capacity))
            .sum();
        let conds: usize = self.conds.borrow().iter().map(|c| c.capacity() * 16).sum();
        let agendas: usize = self.agenda.borrow().iter().map(Agenda::memory_estimate).sum();
        let inboxes: usize = self.inbox.borrow().iter().map(Inbox::memory_estimate).sum();
        let members: usize =
            self.members.borrow().iter().map(|m| m.capacity() * size_of::<Membership>()).sum();
        fixed
            + index
            + attrs
            + comps
            + names
            + conds
            + agendas
            + inboxes
            + members
            + self.grid.borrow().memory_estimate()
            + self.border.memory_estimate()
            + self.timers.borrow().memory_estimate()
            + self.ledger.borrow().memory_estimate()
    }
}
