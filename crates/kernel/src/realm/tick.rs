//! The schedule of a realm: one tick, message delivery, settle points and
//! the removal of residents.
//!
//! One tick of a realm runs in this order:
//!
//! 1. The tick counter grows by one.
//! 2. The messages in the inbox of the realm are delivered, in order.
//! 3. Timers fire: conditions that end, actions that finish or time out. The
//!    timers of one tick fire in `ResidentId` order.
//! 4. Module systems run: stage `Pre`, then `Update`, then `Post`.
//! 5. Routines of idle residents are checked.
//! 6. Residents that leave (migration, despawn) are removed.
//! 7. The zones around this one are told what changed near their border.
//! 8. The ledger is audited.
//!
//! After every message, after the timers and after every system the realm
//! *settles*: residents whose action ended start their next action, and
//! event handlers get the events that were emitted.

use serde_json::json;
use unsup_protocol::{Pos, RealmId, ResidentId, ZoneDelta, ZoneId};

use super::{F_ACTIVITY, F_BUSY, F_MOVED, F_ROUTINES, FateKind, Identity, Realm};
use crate::agenda::{Agenda, Inbox};
use crate::event::EventMeta;
use crate::ids::{CondId, Idx};
use crate::ledger::{Holder, UNCLAIMED};
use crate::message::{Body, Dest, DirUpdate, Envelope, KernelMsg};
use crate::plugin::{Scope, WorldGen};
use crate::timers::TimerKind;

/// Greatest number of settle rounds before the kernel decides that event
/// handlers feed each other without end.
const SETTLE_ROUNDS_MAX: u32 = 10_000;

impl Realm {
    /// Tells the realm which frame runs.
    pub(crate) fn begin_frame(&self, frame: u64) {
        self.frame.set(frame);
    }

    /// Runs one tick.
    pub(crate) fn run_tick(&self) {
        self.tick.set(self.tick.get() + 1);
        self.deliver_mail();
        self.fire_timers();
        self.run_systems();
        self.scan_routines();
        self.finish_step();
        self.build_border();
        self.self_audit();
    }

    /// Brings the realm to rest after a tick or after an input between
    /// frames: settles, removes the residents that leave, and hands the
    /// events for the global realm to the outbox.
    pub(crate) fn finish_step(&self) {
        self.settle();
        self.process_removals();
        self.flush_forward();
    }

    /// Brings the realm to rest without removing residents: settles and
    /// hands the events for the global realm to the outbox.
    pub(crate) fn rest(&self) {
        self.settle();
        self.flush_forward();
    }

    /// Calls `f` for every message that waits in the inbox or the outbox of
    /// the realm.
    pub(crate) fn for_each_pending(&self, mut f: impl FnMut(&Envelope)) {
        for envelope in self.mail.borrow().iter().chain(self.outbox.borrow().iter()) {
            f(envelope);
        }
    }

    /// Runs the world generation hooks of this realm.
    pub(crate) fn worldgen(&self, context: &WorldGen<'_>) {
        for hook in &self.def().worldgen {
            (hook.run)(self, context);
        }
        self.finish_step();
    }

    // ---- Messages -----------------------------------------------------------

    /// Puts a message in the inbox of the realm. It is delivered at the
    /// start of the next tick.
    pub(crate) fn enqueue(&self, envelope: Envelope) {
        self.mail.borrow_mut().push(envelope);
    }

    fn deliver_mail(&self) {
        if self.mail.borrow().is_empty() {
            return;
        }
        // Swap the inbox with an empty buffer that is kept for this purpose,
        // so that neither is allocated again.
        let mut mail = std::mem::take(&mut self.scratch.borrow_mut().mail);
        std::mem::swap(&mut mail, &mut *self.mail.borrow_mut());
        for envelope in mail.drain(..) {
            self.deliver(envelope);
            self.settle();
        }
        self.scratch.borrow_mut().mail = mail;
    }

    /// Sends a message on to wherever its resident is now.
    fn reroute(&self, envelope: Envelope) {
        self.outbox.borrow_mut().push(envelope);
    }

    fn deliver(&self, envelope: Envelope) {
        let Envelope { from, seq, to, body, foreign } = envelope;
        match body {
            Body::Module { kind, payload } => {
                let def = self.def();
                // What the handlers of this kind did. `None`: no handler
                // takes the kind here.
                let outcome = match to {
                    Dest::Resident(resident) => match self.idx_of(resident) {
                        Some(idx) => def.resident_messages.get(kind.as_ref()).map(|handlers| {
                            handlers
                                .iter()
                                .try_for_each(|handler| handler(self, idx, from, &payload))
                        }),
                        // The resident moved after the message was routed.
                        None if self.scope == Scope::Zone => {
                            self.reroute(Envelope {
                                from,
                                seq,
                                to,
                                body: Body::Module { kind, payload },
                                foreign,
                            });
                            return;
                        }
                        None => None,
                    },
                    _ => def.messages.get(kind.as_ref()).map(|handlers| {
                        handlers.iter().try_for_each(|handler| handler(self, from, &payload))
                    }),
                };
                match outcome {
                    Some(Ok(())) => {}
                    // A message from outside this process (`Input::Deliver`)
                    // that no handler takes, or that the handler cannot
                    // read: the sender made a mistake, the world goes on.
                    Some(Err(_)) | None if foreign => self.dropped.set(self.dropped.get() + 1),
                    // A module of this process sent a payload that its own
                    // handler cannot read. That is a bug in the module.
                    Some(Err(e)) => panic!("message `{kind}` from {from}: {e}"),
                    None => {}
                }
            }
            Body::Kernel(msg) => self.deliver_kernel(from, seq, *msg),
        }
    }

    /// Returns how many messages from outside this process the realm
    /// dropped because no handler could take them. It is a statistic, not
    /// simulation state.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.get()
    }

    fn deliver_kernel(&self, from: RealmId, seq: u64, msg: KernelMsg) {
        // A message for a resident that is not here goes on to its zone.
        let forward = |msg: KernelMsg, resident: ResidentId| {
            self.reroute(Envelope::new(
                from,
                seq,
                Dest::Resident(resident),
                Body::Kernel(Box::new(msg)),
            ));
        };
        match msg {
            KernelMsg::Arrive(transfer) => self.arrive(*transfer),
            KernelMsg::Notice { to: resident, kind, from: sender, data } => match self
                .idx_of(resident)
            {
                Some(_) => self.notify(resident, &kind, sender, data),
                None if self.scope == Scope::Zone => {
                    forward(KernelMsg::Notice { to: resident, kind, from: sender, data }, resident);
                }
                None => {}
            },
            KernelMsg::GlobalAct(act) => self.handle_global_act(act),
            KernelMsg::ActionDone(done) => match self.idx_of(done.resident) {
                Some(idx) => self.handle_action_done(idx, done),
                None if self.scope == Scope::Zone => {
                    let resident = done.resident;
                    forward(KernelMsg::ActionDone(done), resident);
                }
                None => {}
            },
            KernelMsg::Complete { resident, action_id, result } => match self.idx_of(resident) {
                Some(_) => self.complete_action(resident, action_id, result),
                None if self.scope == Scope::Zone => {
                    forward(KernelMsg::Complete { resident, action_id, result }, resident);
                }
                None => {}
            },
            KernelMsg::WireMoney { to: holder, amount } => match self.absent_resident(holder) {
                Some(resident) if self.wire_can_follow(resident) => {
                    forward(KernelMsg::WireMoney { to: holder, amount }, resident);
                }
                Some(_) => self.ledger.borrow_mut().wire_in_money(UNCLAIMED, amount),
                None => self.ledger.borrow_mut().wire_in_money(holder, amount),
            },
            KernelMsg::WireItem { to: holder, item, qty } => match self.absent_resident(holder) {
                Some(resident) if self.wire_can_follow(resident) => {
                    forward(KernelMsg::WireItem { to: holder, item, qty }, resident);
                }
                Some(_) => self.ledger.borrow_mut().wire_in_item(UNCLAIMED, item, qty),
                None => self.ledger.borrow_mut().wire_in_item(holder, item, qty),
            },
            KernelMsg::ParamSet { param, city, value } => {
                let own_city = self.zone.as_ref().map(|z| z.city);
                let band = self.border_band();
                self.params.borrow_mut().set(&self.bp, own_city, param, city, value);
                // Another sight radius: the zones around this one hold
                // copies of other residents than they must.
                if self.border_band() != band {
                    self.touch_border_all();
                }
            }
            KernelMsg::Events(events) => {
                // Zone events for the handlers of the global realm.
                self.pending_events.borrow_mut().extend(events);
            }
            KernelMsg::Directory(update) => self.directory.borrow_mut().apply(update),
            KernelMsg::Membership { resident, org, grant } => match self.idx_of(resident) {
                Some(_) => self.set_membership(resident, org, grant),
                None if self.scope == Scope::Zone => {
                    forward(KernelMsg::Membership { resident, org, grant }, resident);
                }
                None => {}
            },
            KernelMsg::Border(news) => {
                if let RealmId::Zone(zone) = from {
                    self.apply_border(zone, news);
                }
            }
        }
    }

    /// Returns the resident of a holder when the holder is a resident that
    /// is not in this realm.
    fn absent_resident(&self, holder: Holder) -> Option<ResidentId> {
        match holder {
            Holder::Resident(resident) if self.idx_of(resident).is_none() => Some(resident),
            _ => None,
        }
    }

    /// Returns true when a wire for an absent resident can be sent on. A
    /// zone always sends it on: the router knows where the resident is, and
    /// gives the wire to the global realm when the resident is gone. The
    /// global realm sends it on only for a resident that is alive.
    fn wire_can_follow(&self, resident: ResidentId) -> bool {
        self.scope == Scope::Zone || self.directory.borrow().is_alive(resident)
    }

    // ---- Timers, systems, routines ------------------------------------------------

    fn fire_timers(&self) {
        let tick = self.tick.get();
        let mut due = std::mem::take(&mut self.scratch.borrow_mut().timers);
        due.clear();
        self.timers.borrow_mut().drain_due(tick, &mut due);
        for timer in &due {
            // The resident may have left the zone since the timer was set.
            let Some(idx) = self.idx_of(timer.resident) else {
                continue;
            };
            match timer.kind {
                TimerKind::Condition => self.expire_condition(idx, CondId(timer.key as u16)),
                TimerKind::Action => self.finish_due(idx, timer.key),
            }
        }
        self.scratch.borrow_mut().timers = due;
        self.settle();
    }

    /// Ends a condition whose end tick is now. A timer for a condition that
    /// was cleared or set again with another end tick does nothing.
    fn expire_condition(&self, idx: Idx, cond: CondId) {
        let tick = self.tick.get();
        let due =
            self.conds.borrow()[idx.index()].iter().any(|&(id, until)| id == cond && until <= tick);
        if due {
            self.end_condition(idx, cond, true);
        }
    }

    fn run_systems(&self) {
        let tick = self.tick.get();
        let offset = match self.id {
            RealmId::Zone(zone) => u64::from(zone.0),
            RealmId::Global => 0,
        };
        for system in &self.def().systems {
            if system.period == 1 || (tick + offset).is_multiple_of(u64::from(system.period)) {
                (system.run)(self);
                self.settle();
            }
        }
    }

    /// Checks the routines of every idle resident that has routines. The
    /// scan reads one byte per resident.
    fn scan_routines(&self) {
        if self.routine_holders.get() == 0 {
            return;
        }
        let mut candidates = std::mem::take(&mut self.scratch.borrow_mut().candidates);
        candidates.clear();
        {
            let flags = self.flags.borrow();
            for (i, &flag) in flags.iter().enumerate() {
                if flag & (F_BUSY | F_ROUTINES) == F_ROUTINES {
                    candidates.push(i as u32);
                }
            }
        }
        for &i in &candidates {
            self.try_routines(Idx(i));
        }
        self.scratch.borrow_mut().candidates = candidates;
    }

    // ---- Settle ---------------------------------------------------------------------

    /// Starts what comes next for residents whose action ended, and gives
    /// emitted events to the handlers of this realm, until nothing is left.
    /// No module borrow is alive when the kernel calls it.
    ///
    /// # Panics
    ///
    /// Panics when handlers keep emitting events without end.
    pub(crate) fn settle(&self) {
        let mut rounds = 0u32;
        loop {
            let mut worked = false;
            if !self.restart.borrow().is_empty() {
                worked = true;
                let mut list = std::mem::take(&mut self.scratch.borrow_mut().restart);
                std::mem::swap(&mut list, &mut *self.restart.borrow_mut());
                for &resident in &list {
                    self.restart_resident(resident);
                }
                list.clear();
                self.scratch.borrow_mut().restart = list;
            }
            loop {
                let event = self.pending_events.borrow_mut().pop_front();
                let Some(event) = event else {
                    break;
                };
                worked = true;
                for (prefix, handler) in &self.def().events {
                    if event.kind.starts_with(prefix) {
                        handler(self, &event);
                    }
                }
            }
            if !worked {
                self.border_settled();
                return;
            }
            rounds += 1;
            assert!(
                rounds < SETTLE_ROUNDS_MAX,
                "{}: event handlers and actions keep triggering each other without end",
                self.id
            );
        }
    }

    // ---- Residents: spawn and removal -----------------------------------------------

    /// Adds the kernel columns of a new row. The caller adds the component
    /// columns.
    pub(super) fn add_row(&self, id: ResidentId, identity: Identity, pos: Pos) -> Idx {
        let mut ids = self.ids.borrow_mut();
        let idx = Idx(ids.len() as u32);
        ids.push(id);
        self.index.borrow_mut().insert(id, idx.0);
        self.identity.borrow_mut().push(identity);
        self.pos.borrow_mut().push(pos);
        for (attr, def) in self.attrs.iter().zip(self.bp.attrs()) {
            attr.borrow_mut().push(def.1.default);
        }
        self.conds.borrow_mut().push(Vec::new());
        self.agenda.borrow_mut().push(Agenda::default());
        self.flags.borrow_mut().push(0);
        self.inbox.borrow_mut().push(Inbox::default());
        self.members.borrow_mut().push(Vec::new());
        self.grid.borrow_mut().insert(idx.0, pos);
        self.border.shown.borrow_mut().push(0);
        drop(ids);
        self.border_moved(idx, pos);
        idx
    }

    /// Adds a new resident and runs the `on_spawn` hooks.
    pub(crate) fn spawn_resident(&self, id: ResidentId, identity: Identity, pos: Pos) {
        let zone = self.zone_ref().id;
        let owner = identity.owner;
        let name = identity.name.clone();
        let idx = self.add_row(id, identity, pos);
        for column in &self.comps {
            column.borrow_mut().push_default();
        }
        self.delta.borrow_mut().entered.push(self.resident_view(idx));
        self.push_kernel(
            Dest::Global,
            KernelMsg::Directory(DirUpdate::Spawned { id, owner, name: name.clone(), zone }),
        );
        for hook in &self.def().on_spawn {
            (hook.run)(self, idx);
        }
        self.emit(
            EventMeta::public("kernel.spawned", 10).actor(id).at(pos),
            &json!({ "resident": id, "name": name }),
        );
    }

    /// Removes the row of a resident from every column. The last row takes
    /// its place.
    pub(super) fn remove_row(&self, idx: Idx) {
        let i = idx.index();
        let mut ids = self.ids.borrow_mut();
        let last = ids.len() - 1;
        let id = ids[i];
        let mut pos = self.pos.borrow_mut();
        {
            let mut grid = self.grid.borrow_mut();
            grid.remove(idx.0, pos[i]);
            if i != last {
                grid.rename(last as u32, idx.0, pos[last]);
            }
        }
        let mut index = self.index.borrow_mut();
        index.remove(&id);
        if i != last {
            index.insert(ids[last], idx.0);
        }
        ids.swap_remove(i);
        pos.swap_remove(i);
        self.identity.borrow_mut().swap_remove(i);
        for attr in &self.attrs {
            attr.borrow_mut().swap_remove(i);
        }
        self.conds.borrow_mut().swap_remove(i);
        for column in &self.comps {
            column.borrow_mut().swap_remove(i);
        }
        let agenda = self.agenda.borrow_mut().swap_remove(i);
        if !agenda.routines.is_empty() {
            self.routine_holders.set(self.routine_holders.get().saturating_sub(1));
        }
        self.flags.borrow_mut().swap_remove(i);
        self.inbox.borrow_mut().swap_remove(i);
        self.members.borrow_mut().swap_remove(i);
        self.border.shown.borrow_mut().swap_remove(i);
    }

    /// Removes the residents that leave: runs the lifecycle hooks, then
    /// takes the rows out. Migrating residents go to `departures`, where the
    /// world picks them up.
    fn process_removals(&self) {
        if self.fates.borrow().is_empty() {
            return;
        }
        // 1. Hooks, in `ResidentId` order. A hook can make more residents
        //    leave; they get their hooks in the next round.
        loop {
            let batch: Vec<ResidentId> =
                self.fates.borrow().iter().filter(|(_, f)| !f.sealed).map(|(&id, _)| id).collect();
            if batch.is_empty() {
                break;
            }
            for id in batch {
                // Seal the fate. An earlier hook of this round may have
                // cancelled it (the resident stepped back into the zone).
                let despawn = match self.fates.borrow_mut().get_mut(&id) {
                    Some(fate) if !fate.sealed => {
                        fate.sealed = true;
                        matches!(fate.kind, FateKind::Despawn { .. })
                    }
                    _ => continue,
                };
                let Some(idx) = self.idx_of(id) else {
                    continue;
                };
                let hooks = if despawn { &self.def().on_despawn } else { &self.def().on_depart };
                for hook in hooks {
                    hook(self, idx);
                }
            }
            self.settle();
        }
        // 2. The delta for viewers and the news for the zones around this
        //    one, while every index is still valid.
        self.build_delta();
        self.build_border();
        // 3. Take the rows out, highest index first, so that the row that
        //    fills a gap is never one that also leaves.
        let fates = std::mem::take(&mut *self.fates.borrow_mut());
        let mut leaving: Vec<(u32, ResidentId, FateKind)> = fates
            .into_iter()
            .filter_map(|(id, fate)| self.idx_of(id).map(|idx| (idx.0, id, fate.kind)))
            .collect();
        leaving.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
        for (idx, id, kind) in leaving {
            match kind {
                FateKind::Migrate(zone) => self.depart(Idx(idx), id, zone),
                FateKind::Despawn { heir, cause } => {
                    self.remove_despawned(Idx(idx), id, heir, &cause)
                }
            }
        }
        self.settle();
    }

    fn depart(&self, idx: Idx, id: ResidentId, zone: ZoneId) {
        let transfer = self.extract(idx);
        self.delta.borrow_mut().left.push(id);
        self.push_kernel(Dest::Global, KernelMsg::Directory(DirUpdate::Moved { id, zone }));
        self.departures.borrow_mut().push((zone, transfer));
    }

    fn remove_despawned(&self, idx: Idx, id: ResidentId, heir: Holder, cause: &str) {
        let running = self.agenda.borrow_mut()[idx.index()].current.take();
        if let Some(running) = running {
            let kind = "kernel.action_interrupted";
            if self.def().wants_event(kind) || self.bp.global.wants_event(kind) {
                self.emit(
                    EventMeta::silent(kind).actor(id),
                    &json!({
                        "resident": id,
                        "verb": self.bp.verb_name(running.verb),
                        "action_id": running.action_id,
                        "reason": "despawned",
                    }),
                );
            }
        }
        // Holdings go to the heir.
        let me = Holder::Resident(id);
        let local = match heir {
            Holder::Site(_) => true,
            Holder::Resident(other) => other != id && self.idx_of(other).is_some(),
            _ => false,
        };
        if local {
            self.ledger.borrow_mut().move_holdings(me, heir, "inherit");
        } else {
            let (to, holder) = match heir {
                Holder::Resident(other) if other != id => (Dest::Resident(other), heir),
                // A resident cannot inherit from itself.
                Holder::Resident(_) => (Dest::Global, UNCLAIMED),
                _ => (Dest::Global, heir),
            };
            let (money, items) = self.ledger.borrow_mut().take_holdings(me);
            if money != 0 {
                self.push_kernel(to, KernelMsg::WireMoney { to: holder, amount: money });
            }
            for (item, qty) in items {
                self.push_kernel(to, KernelMsg::WireItem { to: holder, item, qty });
            }
        }
        let name = self.identity.borrow()[idx.index()].name.clone();
        let pos = self.pos(idx);
        self.emit(
            EventMeta::public("kernel.despawned", 30).actor(id).at(pos),
            &json!({ "resident": id, "name": name, "cause": cause }),
        );
        self.push_kernel(Dest::Global, KernelMsg::Directory(DirUpdate::Despawned { id }));
        self.delta.borrow_mut().left.push(id);
        self.despawned.borrow_mut().push(id);
        self.remove_row(idx);
    }

    // ---- Output -------------------------------------------------------------------------

    /// Writes the position and activity changes since the last call to the
    /// delta. It must run before a row is removed, because it reads dense
    /// indices.
    fn build_delta(&self) {
        let mut dirty = self.dirty.borrow_mut();
        if dirty.is_empty() {
            return;
        }
        let mut flags = self.flags.borrow_mut();
        let ids = self.ids.borrow();
        let pos = self.pos.borrow();
        let agenda = self.agenda.borrow();
        let mut delta = self.delta.borrow_mut();
        for &i in dirty.iter() {
            let i = i as usize;
            let flag = flags[i];
            if flag & F_MOVED != 0 {
                delta.moved.push((ids[i], pos[i]));
            }
            if flag & F_ACTIVITY != 0 {
                let verb =
                    agenda[i].current.as_ref().map(|r| self.bp.verb_name(r.verb).to_string());
                delta.activity.push((ids[i], verb));
            }
            flags[i] = flag & !(F_MOVED | F_ACTIVITY);
        }
        dirty.clear();
    }

    /// Takes what changed in this zone since the last call, for viewers.
    pub(crate) fn take_delta(&self) -> Option<ZoneDelta> {
        let zone = self.zone.as_ref()?.id;
        self.build_delta();
        let mut delta = self.delta.borrow_mut();
        if delta.is_empty() {
            return None;
        }
        let mut out = std::mem::take(&mut *delta);
        out.zone = zone;
        out.tick = self.tick();
        Some(out)
    }

    /// Moves the public events since the last call to `out`.
    pub(crate) fn take_events(&self, out: &mut Vec<unsup_protocol::Event>) {
        out.append(&mut self.frame_events.borrow_mut());
    }

    /// Moves the residents that got a notice or a report since the last
    /// call to `out`.
    pub(crate) fn take_woke(&self, out: &mut Vec<ResidentId>) {
        out.append(&mut self.woke.borrow_mut());
    }

    /// Moves the messages that this realm sent to `out`, in sending order.
    pub(crate) fn take_outbox(&self, out: &mut Vec<Envelope>) {
        out.append(&mut self.outbox.borrow_mut());
    }

    /// Takes the residents that left for another zone, in leaving order.
    pub(crate) fn take_departures(&self) -> Vec<(ZoneId, super::Transfer)> {
        std::mem::take(&mut *self.departures.borrow_mut())
    }

    /// Takes the residents that left the world.
    pub(crate) fn take_despawned(&self) -> Vec<ResidentId> {
        std::mem::take(&mut *self.despawned.borrow_mut())
    }

    /// Hands the events that the handlers of the global realm want to the
    /// outbox, as one message.
    fn flush_forward(&self) {
        let mut forward = self.forward.borrow_mut();
        if !forward.is_empty() {
            let events = std::mem::take(&mut *forward);
            drop(forward);
            self.push_kernel(Dest::Global, KernelMsg::Events(events));
        }
    }

    /// Audits the ledger of the realm.
    ///
    /// # Panics
    ///
    /// Panics when the balances do not match the totals. Only kernel code
    /// writes balances, so a failure is a kernel bug or corrupted memory,
    /// and going on would spread it.
    fn self_audit(&self) {
        if self.audit_every == 0 || !self.tick.get().is_multiple_of(u64::from(self.audit_every)) {
            return;
        }
        if let Err(mut error) = self.ledger.borrow().audit() {
            error.realm = Some(self.id);
            panic!("{error}");
        }
    }
}
