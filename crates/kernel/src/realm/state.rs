//! Saving, loading and moving realm state: snapshots and migration.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_protocol::{Event, ItemId, Pos, RealmId, ResidentId, ZoneId};

use super::border::Ghost;
use super::{F_BORDER, F_BUSY, F_ROUTINES, FOREVER, Fate, Identity, Membership, ParamTable, Realm};
use crate::agenda::{Agenda, Inbox, rebase_tick};
use crate::codec::{self, DecodeError};
use crate::directory::Directory;
use crate::ids::{CondId, Idx, VerbId};
use crate::ledger::{Holder, Ledger};
use crate::message::Envelope;
use crate::rng::Rng;
use crate::sites::Sites;
use crate::tiles::TileMap;
use crate::timers::{Timer, TimerKind, TimerWheel};

/// A resident in transit between two zones, with everything that moves with
/// it: identity, position, attributes, conditions, components, agenda (the
/// running action too), inbox, memberships and holdings.
///
/// Ticks inside are in the time base of the zone that the resident left;
/// `depart_tick` is the tick of that zone at the departure. The zone that
/// receives the resident moves them to its own time base.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Transfer {
    pub(crate) id: ResidentId,
    identity: Identity,
    pos: Pos,
    attrs: Vec<i64>,
    conds: Vec<(CondId, u64)>,
    /// Hash of the component names of the sender, in column order.
    comp_signature: u64,
    /// The component values, written one after another in column order.
    #[serde(with = "crate::codec::bytes")]
    comps: Vec<u8>,
    agenda: Agenda,
    inbox: Inbox,
    members: Vec<Membership>,
    pub(crate) money: i64,
    pub(crate) items: Vec<(ItemId, i64)>,
    depart_tick: u64,
    /// The zones that hold a copy of the resident under its new zone, for
    /// sight across borders. The zone that the resident left told them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    seen_by: Vec<ZoneId>,
}

/// One named blob: a component column or a resource.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Blob {
    name: String,
    #[serde(with = "crate::codec::bytes")]
    data: Vec<u8>,
}

/// The state of a realm as it is read from a snapshot.
///
/// Derived structures (the id index, the grid, the timer wheel, the flags)
/// are not saved; `Realm::load` rebuilds them.
#[derive(Debug, Deserialize)]
pub(crate) struct RealmState {
    pub(crate) id: RealmId,
    tick: u64,
    rng: Rng,
    next_id: u64,
    event_seq: u64,
    out_seq: u64,
    ids: Vec<ResidentId>,
    identity: Vec<Identity>,
    pos: Vec<Pos>,
    attrs: Vec<Vec<i64>>,
    conds: Vec<Vec<(CondId, u64)>>,
    comps: Vec<Blob>,
    agenda: Vec<Agenda>,
    inbox: Vec<Inbox>,
    members: Vec<Vec<Membership>>,
    fates: Vec<(ResidentId, Fate)>,
    resources: Vec<Blob>,
    params: ParamTable,
    sites: Sites,
    tiles: TileMap,
    ledger: Ledger,
    directory: Directory,
    mail: Vec<Envelope>,
    outbox: Vec<Envelope>,
    forward: Vec<Event>,
    /// Sight across borders: per resident, the zones that hold a copy of
    /// it. Absent in the state of the global realm, of a world with one
    /// zone, and in a snapshot from before the kernel had it.
    #[serde(default)]
    border_shown: Option<Vec<u32>>,
    /// The residents whose copies may be out of date, by dense index.
    #[serde(default)]
    border_dirty: Vec<u32>,
    /// The copies of residents of other zones.
    #[serde(default, with = "crate::codec::pairs")]
    border_ghosts: BTreeMap<(ZoneId, ResidentId), Ghost>,
    /// Residents with a copy whose action ended, with the verb, when the
    /// realm was saved before it settled. Normally empty.
    #[serde(default, with = "crate::codec::pairs")]
    border_ended: BTreeMap<u32, VerbId>,
}

/// The state of a realm as it is written to a snapshot. It borrows the
/// realm, so that a snapshot does not copy the state first. The field names
/// are those of [`RealmState`].
#[derive(Serialize)]
struct RealmStateRef<'a> {
    id: RealmId,
    tick: u64,
    rng: &'a Rng,
    next_id: u64,
    event_seq: u64,
    out_seq: u64,
    ids: &'a [ResidentId],
    identity: &'a [Identity],
    pos: &'a [Pos],
    attrs: &'a [std::cell::RefCell<Vec<i64>>],
    conds: &'a [Vec<(CondId, u64)>],
    comps: Vec<Blob>,
    agenda: &'a [Agenda],
    inbox: &'a [Inbox],
    members: &'a [Vec<Membership>],
    fates: Vec<(&'a ResidentId, &'a Fate)>,
    resources: Vec<Blob>,
    params: &'a ParamTable,
    sites: &'a Sites,
    tiles: &'a TileMap,
    ledger: &'a Ledger,
    directory: &'a Directory,
    mail: &'a [Envelope],
    outbox: &'a [Envelope],
    forward: &'a [Event],
    #[serde(skip_serializing_if = "Option::is_none")]
    border_shown: Option<&'a [u32]>,
    #[serde(skip_serializing_if = "<[u32]>::is_empty")]
    border_dirty: &'a [u32],
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "crate::codec::pairs")]
    border_ghosts: &'a BTreeMap<(ZoneId, ResidentId), Ghost>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", with = "crate::codec::pairs")]
    border_ended: &'a BTreeMap<u32, VerbId>,
}

impl Realm {
    // ---- Snapshot -------------------------------------------------------------

    /// Writes the state of the realm as MessagePack. Equal states give equal
    /// bytes, so the bytes are also what the state hash is made from.
    pub(crate) fn save(&self) -> Vec<u8> {
        let rng = self.rng.borrow();
        let ids = self.ids.borrow();
        let identity = self.identity.borrow();
        let pos = self.pos.borrow();
        let conds = self.conds.borrow();
        let agenda = self.agenda.borrow();
        let inbox = self.inbox.borrow();
        let members = self.members.borrow();
        let fates = self.fates.borrow();
        let params = self.params.borrow();
        let sites = self.sites.borrow();
        let tiles = self.tiles.borrow();
        let ledger = self.ledger.borrow();
        let directory = self.directory.borrow();
        let mail = self.mail.borrow();
        let outbox = self.outbox.borrow();
        let forward = self.forward.borrow();
        let border_shown = self.border.shown.borrow();
        let border_dirty = self.border.dirty.borrow();
        let border_ghosts = self.border.ghosts.borrow();
        let border_ended = self.border.ended.borrow();
        let def = self.def();
        let state = RealmStateRef {
            id: self.id,
            tick: self.tick.get(),
            rng: &rng,
            next_id: self.next_id.get(),
            event_seq: self.event_seq.get(),
            out_seq: self.out_seq.get(),
            ids: &ids,
            identity: &identity,
            pos: &pos,
            attrs: &self.attrs,
            conds: &conds,
            comps: def
                .components
                .iter()
                .zip(&self.comps)
                .map(|(entry, column)| Blob {
                    name: entry.name.to_string(),
                    data: column.borrow().encode_all(),
                })
                .collect(),
            agenda: &agenda,
            inbox: &inbox,
            members: &members,
            fates: fates.iter().collect(),
            resources: def
                .resources
                .iter()
                .zip(&self.resources)
                .map(|(entry, resource)| Blob {
                    name: entry.name.to_string(),
                    data: resource.borrow().encode(),
                })
                .collect(),
            params: &params,
            sites: &sites,
            tiles: &tiles,
            ledger: &ledger,
            directory: &directory,
            mail: &mail,
            outbox: &outbox,
            forward: &forward,
            border_shown: self.border.has_watchers().then_some(border_shown.as_slice()),
            border_dirty: &border_dirty,
            border_ghosts: &border_ghosts,
            border_ended: &border_ended,
        };
        codec::encode("realm state", &state)
    }

    /// Replaces the state of a new, empty realm with the state of a
    /// snapshot, and rebuilds the derived structures.
    ///
    /// A component or a resource that the snapshot does not have keeps its
    /// default value. One that the blueprint does not have is dropped.
    pub(crate) fn load(&self, state: RealmState, frame: u64) -> Result<(), DecodeError> {
        let fail =
            |message: String| DecodeError { what: format!("the state of {}", self.id), message };
        let n = state.ids.len();
        let lengths = [
            state.identity.len(),
            state.pos.len(),
            state.conds.len(),
            state.agenda.len(),
            state.inbox.len(),
            state.members.len(),
        ];
        if lengths.iter().any(|&len| len != n) || state.attrs.iter().any(|column| column.len() != n)
        {
            return Err(fail("the resident columns have different lengths".into()));
        }
        if state.attrs.len() > self.attrs.len() && n > 0 {
            return Err(fail("the snapshot has more attributes than the world".into()));
        }
        if state.border_shown.as_ref().is_some_and(|shown| shown.len() != n)
            || state.border_dirty.iter().any(|&i| i as usize >= n)
            || state.border_ended.keys().any(|&i| i as usize >= n)
        {
            return Err(fail("the border columns do not match the residents".into()));
        }
        let tick = state.tick;
        self.tick.set(tick);
        self.frame.set(frame);
        *self.rng.borrow_mut() = state.rng;
        self.next_id.set(state.next_id);
        self.event_seq.set(state.event_seq);
        self.out_seq.set(state.out_seq);

        // Attributes: the snapshot columns by id, then defaults for
        // attributes that are new since the snapshot.
        let mut saved = state.attrs.into_iter();
        for (cell, (_, def)) in self.attrs.iter().zip(self.bp.attrs()) {
            *cell.borrow_mut() = saved.next().unwrap_or_else(|| vec![def.default; n]);
        }
        // Components and resources: by name.
        let mut blobs: BTreeMap<String, Vec<u8>> =
            state.comps.into_iter().map(|blob| (blob.name, blob.data)).collect();
        for (entry, column) in self.def().components.iter().zip(&self.comps) {
            let mut column = column.borrow_mut();
            match blobs.remove(entry.name) {
                Some(data) => {
                    column.decode_all(&data)?;
                    if column.len() != n {
                        return Err(fail(format!(
                            "component `{}` has the wrong length",
                            entry.name
                        )));
                    }
                }
                None => column.resize(n),
            }
        }
        let mut blobs: BTreeMap<String, Vec<u8>> =
            state.resources.into_iter().map(|blob| (blob.name, blob.data)).collect();
        for (entry, resource) in self.def().resources.iter().zip(&self.resources) {
            if let Some(data) = blobs.remove(entry.name) {
                resource.borrow_mut().decode(&data)?;
            }
        }

        let own_city = self.zone.as_ref().map(|z| z.city);
        let mut params = state.params;
        params.refresh(&self.bp, own_city);
        *self.params.borrow_mut() = params;
        *self.sites.borrow_mut() = state.sites;
        self.sites_epoch.set(self.sites_epoch.get().wrapping_add(1));
        *self.tiles.borrow_mut() = state.tiles;
        *self.ledger.borrow_mut() = state.ledger;
        *self.directory.borrow_mut() = state.directory;
        *self.mail.borrow_mut() = state.mail;
        *self.outbox.borrow_mut() = state.outbox;
        *self.forward.borrow_mut() = state.forward;
        *self.fates.borrow_mut() = state.fates.into_iter().collect();

        // Derived structures.
        let mut index = BTreeMap::new();
        let mut grid = self.grid.borrow_mut();
        let mut timers = TimerWheel::default();
        let mut flags = Vec::with_capacity(n);
        let mut holders = 0u32;
        for (i, &id) in state.ids.iter().enumerate() {
            if index.insert(id, i as u32).is_some() {
                return Err(fail(format!("resident {id} is in the table twice")));
            }
            grid.insert(i as u32, state.pos[i]);
            let agenda = &state.agenda[i];
            let mut flag = 0u8;
            if let Some(running) = &agenda.current {
                flag |= F_BUSY;
                timers.insert(
                    tick,
                    running.ends,
                    Timer { resident: id, kind: TimerKind::Action, key: running.action_id },
                );
            }
            if !agenda.routines.is_empty() {
                flag |= F_ROUTINES;
                holders += 1;
            }
            flags.push(flag);
            for &(cond, until) in &state.conds[i] {
                if until != FOREVER {
                    timers.insert(
                        tick,
                        until,
                        Timer { resident: id, kind: TimerKind::Condition, key: u64::from(cond.0) },
                    );
                }
            }
        }
        drop(grid);
        // Sight across borders. A snapshot from before the kernel had it
        // knows of no copy: every resident is checked in the next tick.
        let border_dirty = match &state.border_shown {
            Some(_) => state.border_dirty,
            None if self.border.has_watchers() => (0..n as u32).collect(),
            None => Vec::new(),
        };
        for &i in &border_dirty {
            flags[i as usize] |= F_BORDER;
        }
        *self.border.shown.borrow_mut() = state.border_shown.unwrap_or_else(|| vec![0; n]);
        *self.border.dirty.borrow_mut() = border_dirty;
        *self.border.ghosts.borrow_mut() = state.border_ghosts;
        *self.border.ended.borrow_mut() = state.border_ended;
        *self.index.borrow_mut() = index;
        *self.timers.borrow_mut() = timers;
        *self.flags.borrow_mut() = flags;
        self.routine_holders.set(holders);
        *self.ids.borrow_mut() = state.ids;
        *self.identity.borrow_mut() = state.identity;
        *self.pos.borrow_mut() = state.pos;
        *self.conds.borrow_mut() = state.conds;
        *self.agenda.borrow_mut() = state.agenda;
        *self.inbox.borrow_mut() = state.inbox;
        *self.members.borrow_mut() = state.members;
        Ok(())
    }

    // ---- Migration ------------------------------------------------------------

    /// Takes a resident out of this zone with everything that moves with it.
    pub(super) fn extract(&self, idx: Idx) -> Transfer {
        let i = idx.index();
        let id = self.id_of(idx);
        let mut comps = Vec::new();
        for column in &self.comps {
            column.borrow().encode_row(i, &mut comps);
        }
        let (money, items) = self.ledger.borrow_mut().take_holdings(Holder::Resident(id));
        let transfer = Transfer {
            id,
            identity: self.identity.borrow()[i].clone(),
            pos: self.pos.borrow()[i],
            attrs: self.attrs.iter().map(|a| a.borrow()[i]).collect(),
            conds: std::mem::take(&mut self.conds.borrow_mut()[i]),
            comp_signature: self.bp.component_signature,
            comps,
            agenda: std::mem::take(&mut self.agenda.borrow_mut()[i]),
            inbox: std::mem::take(&mut self.inbox.borrow_mut()[i]),
            members: std::mem::take(&mut self.members.borrow_mut()[i]),
            money,
            items,
            depart_tick: self.tick.get(),
            seen_by: self.border.take_handoff(id),
        };
        // `remove_row` reads the agenda to keep the routine counter right;
        // the agenda was taken, so do it here.
        if !transfer.agenda.routines.is_empty() {
            self.routine_holders.set(self.routine_holders.get().saturating_sub(1));
        }
        self.remove_row(idx);
        transfer
    }

    /// Puts a resident that left another zone into this zone and runs the
    /// `on_arrive` hooks.
    ///
    /// # Panics
    ///
    /// Panics when the transfer comes from a process with another set of
    /// components. All processes of one world must run the same build.
    pub(crate) fn arrive(&self, transfer: Transfer) {
        let zone = self.zone_ref().id;
        assert!(
            transfer.comp_signature == self.bp.component_signature,
            "resident {} arrives in zone {zone} from a build with other components",
            transfer.id
        );
        let Transfer {
            id,
            identity,
            pos,
            attrs,
            conds,
            comps,
            agenda,
            inbox,
            members,
            money,
            items,
            depart_tick,
            seen_by,
            ..
        } = transfer;
        let tick = self.tick.get();
        let idx = self.add_row(id, identity, pos);
        self.border_arrived(idx, &seen_by);
        let i = idx.index();
        for (cell, value) in self.attrs.iter().zip(attrs) {
            cell.borrow_mut()[i] = value;
        }
        let mut reader = codec::seq_reader(&comps);
        for column in &self.comps {
            if let Err(e) = column.borrow_mut().push_decoded(&mut reader) {
                panic!("resident {id} arrives in zone {zone}: {e}");
            }
        }
        // Conditions and the agenda move to the time base of this zone.
        {
            let mut timers = self.timers.borrow_mut();
            let mut list = conds;
            list.retain_mut(|(cond, until)| {
                *until = rebase_tick(*until, depart_tick, tick);
                if *until == FOREVER {
                    return true;
                }
                if *until <= tick {
                    return false;
                }
                timers.insert(
                    tick,
                    *until,
                    Timer { resident: id, kind: TimerKind::Condition, key: u64::from(cond.0) },
                );
                true
            });
            self.conds.borrow_mut()[i] = list;
            let mut agenda = agenda;
            agenda.rebase(depart_tick, tick);
            let mut flag = 0u8;
            if let Some(running) = &mut agenda.current {
                // A running action never ends in the tick of the arrival.
                running.ends = running.ends.max(tick + 1);
                flag |= F_BUSY;
                timers.insert(
                    tick,
                    running.ends,
                    Timer { resident: id, kind: TimerKind::Action, key: running.action_id },
                );
            }
            if !agenda.routines.is_empty() {
                flag |= F_ROUTINES;
                self.routine_holders.set(self.routine_holders.get() + 1);
            }
            // `add_row` may have marked the row for the zones around this
            // one; keep that mark.
            self.flags.borrow_mut()[i] |= flag;
            self.agenda.borrow_mut()[i] = agenda;
        }
        self.inbox.borrow_mut()[i] = inbox;
        self.members.borrow_mut()[i] = members;
        self.ledger.borrow_mut().put_holdings(Holder::Resident(id), money, &items);
        self.delta.borrow_mut().entered.push(self.resident_view(idx));
        for hook in &self.def().on_arrive {
            hook(self, idx);
        }
    }

    /// Returns the zone that this realm is, for messages about arrivals.
    pub(crate) fn zone_id(&self) -> Option<ZoneId> {
        self.zone.as_ref().map(|z| z.id)
    }
}
