//! Reads: the observation of a resident and the view of a zone.
//!
//! Reads never change state. While an observation is built the realm is
//! marked read-only, and a write panics in debug builds.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};
use unsup_protocol::{
    ActionReport, Notice, Observation, ObserveOpts, ResidentId, ResidentView, WorldView, ZoneView,
};

use super::{FOREVER, ReadGuard, Realm};
use crate::agenda::Shown;
use crate::defs::AttrShow;
use crate::ids::Idx;
use crate::ledger::Holder;

impl Realm {
    /// Builds the observation of a resident of this zone: the kernel
    /// sections, the sections of the zone observers, and the notices.
    /// Returns `None` when the resident is not in this zone.
    pub(crate) fn observe(&self, resident: ResidentId, opts: &ObserveOpts) -> Option<Observation> {
        let idx = self.idx_of(resident)?;
        let _guard = ReadGuard::new(self);
        let mut sections = BTreeMap::new();
        if opts.wants("self") {
            sections.insert("self".to_string(), self.section_self(idx, opts));
        }
        if opts.wants("nearby") {
            sections.insert("nearby".to_string(), self.section_nearby(idx));
        }
        let (notices, reports, next_notice) = self.read_inbox(idx, opts.since_notice);
        if opts.wants("reports") {
            sections.insert("reports".to_string(), json!(reports));
        }
        for observer in &self.def().observers {
            if opts.wants(observer.section)
                && let Some(value) = (observer.run)(self, idx, opts)
            {
                sections.insert(observer.section.to_string(), value);
            }
        }
        Some(Observation {
            frame: self.frame.get(),
            tick: self.tick(),
            clock: self.clock(),
            resident,
            sections,
            notices,
            next_notice,
        })
    }

    /// Adds the sections of the global observers to an observation. Call it
    /// on the global realm.
    pub(crate) fn observe_global(
        &self,
        resident: ResidentId,
        opts: &ObserveOpts,
        sections: &mut BTreeMap<String, Value>,
    ) {
        let _guard = ReadGuard::new(self);
        for observer in &self.def().global_observers {
            if opts.wants(observer.section)
                && let Some(value) = (observer.run)(self, resident, opts)
            {
                sections.insert(observer.section.to_string(), value);
            }
        }
    }

    /// Returns the sequence number that the next notice of a resident of
    /// this zone will get.
    pub(crate) fn next_notice(&self, resident: ResidentId) -> Option<u64> {
        let idx = self.idx_of(resident)?;
        Some(self.inbox.borrow()[idx.index()].next_seq())
    }

    /// Reads the inbox from sequence number `since`: the notices, the
    /// action reports, and the next sequence number.
    fn read_inbox(&self, idx: Idx, since: u64) -> (Vec<Notice>, Vec<ActionReport>, u64) {
        let inboxes = self.inbox.borrow();
        let inbox = &inboxes[idx.index()];
        let mut notices = Vec::new();
        let mut reports = Vec::new();
        for entry in inbox.since(since) {
            match entry.show(|verb| self.bp.verb_name(verb)) {
                Shown::Notice(notice) => notices.push(notice),
                Shown::Report(report) => reports.push(report),
            }
        }
        (notices, reports, inbox.next_seq())
    }

    /// The section `self`: id, name, position, zone, site, activity, queue
    /// length, conditions, attributes, cash and items. Full detail adds the
    /// queue, the routines, the memberships and the identity.
    fn section_self(&self, idx: Idx, opts: &ObserveOpts) -> Value {
        let id = self.id_of(idx);
        let pos = self.pos(idx);
        let tick = self.tick.get();
        let zone = self.zone_ref();
        let identity = self.identity.borrow();
        let who = &identity[idx.index()];
        let agendas = self.agenda.borrow();
        let agenda = &agendas[idx.index()];
        let ledger = self.ledger.borrow();
        let full = opts.is_full();

        let mut out = Map::new();
        out.insert("id".into(), json!(id));
        out.insert("name".into(), json!(who.name));
        out.insert("pos".into(), json!(pos));
        out.insert("zone".into(), json!(zone.id));
        out.insert("city".into(), json!(zone.city));
        if let Some(site) = self.sites.borrow().at(pos) {
            out.insert(
                "site".into(),
                json!({ "id": site.id, "name": site.name, "kind": site.kind }),
            );
        }
        let activity = agenda.current.as_ref().map(|running| {
            json!({
                "verb": self.bp.verb_name(running.verb),
                "action_id": running.action_id,
                "started": running.started,
                "ends": running.ends,
                "waiting": running.wait,
            })
        });
        out.insert("activity".into(), activity.unwrap_or(Value::Null));
        out.insert("queue".into(), json!(agenda.queue.len()));

        let mut conditions = Map::new();
        for &(cond, until) in &self.conds.borrow()[idx.index()] {
            if until > tick {
                let until = if until == FOREVER { Value::Null } else { json!(until) };
                conditions.insert(self.bp.condition(cond).name.clone(), until);
            }
        }
        out.insert("conditions".into(), Value::Object(conditions));

        let mut attrs = Map::new();
        for (attr, def) in self.bp.attrs() {
            let shown = match def.show {
                AttrShow::Brief => true,
                AttrShow::Full => full,
                AttrShow::Hidden => false,
            };
            if shown {
                attrs.insert(def.name.clone(), json!(self.attr(idx, attr)));
            }
        }
        out.insert("attrs".into(), Value::Object(attrs));

        let holder = Holder::Resident(id);
        out.insert("cash".into(), json!(ledger.money(holder)));
        let mut items = Map::new();
        for (item, qty) in ledger.items_of(holder) {
            items.insert(self.bp.item_name(item).to_string(), json!(qty));
        }
        out.insert("items".into(), Value::Object(items));

        if full {
            out.insert("zone_name".into(), json!(zone.name));
            out.insert("owner".into(), json!(who.owner));
            out.insert("model".into(), json!(who.model));
            out.insert("born".into(), json!(who.born));
            let queued: Vec<Value> = agenda
                .queue
                .iter()
                .map(|q| json!({ "verb": self.bp.verb_name(q.verb), "action_id": q.action_id }))
                .collect();
            out.insert("queued".into(), json!(queued));
            // Each routine as the owner wrote it, with how often it ran and
            // how often it failed in a row.
            let routines: Vec<Value> = agenda
                .routines
                .iter()
                .map(|slot| {
                    json!({
                        "name": slot.routine.name,
                        "when": slot.routine.when,
                        "then": slot.then(),
                        "runs": slot.runs,
                        "failures": slot.failures,
                    })
                })
                .collect();
            out.insert("routines".into(), json!(routines));
            let members: Vec<Value> = self.members.borrow()[idx.index()]
                .iter()
                .map(|m| json!({ "org": m.org, "role": m.role, "perms": m.perms }))
                .collect();
            out.insert("memberships".into(), json!(members));
        }
        Value::Object(out)
    }

    /// The section `nearby`: the nearest residents and sites within sight,
    /// each list bounded by `kernel.nearby_max`. The residents on the other
    /// side of a zone border are in the list too, as this zone last heard
    /// of them; each has the key `zone`.
    ///
    /// The residents are in the order of distance, then of id. The section
    /// does not sort everybody in sight to find them: it asks for the
    /// nearest of this zone and the nearest across the border, and merges
    /// the two short lists.
    fn section_nearby(&self, idx: Idx) -> Value {
        let kernel = self.bp.kernel_ids();
        let radius = u32::try_from(self.param(kernel.sight_radius)).unwrap_or(0);
        let max = usize::try_from(self.param(kernel.nearby_max)).unwrap_or(0);
        let pos = self.pos(idx);

        let (here, total_here) = {
            let positions = self.pos.borrow();
            let ids = self.ids.borrow();
            let grid = self.grid.borrow();
            // One more than the list holds: the observer is its own
            // nearest resident and is not in the list.
            let mut here = Vec::new();
            grid.nearest(pos, radius, max.saturating_add(1), &positions, &ids, &mut here);
            here.retain(|near| near.idx != idx.0);
            here.truncate(max);
            (here, grid.count(pos, radius, &positions).saturating_sub(1))
        };
        let (outside, total_outside) = self.nearest_outsiders(pos, radius, max);
        let total_residents = total_here + total_outside;

        let identity = self.identity.borrow();
        let mut residents: Vec<Value> = Vec::with_capacity(max.min(total_residents));
        let mut here = here.iter().peekable();
        let mut outside = outside.iter().peekable();
        while residents.len() < max {
            let next_is_here = match (here.peek(), outside.peek()) {
                (Some(near), Some((dist, outsider))) => (near.dist, near.id) < (*dist, outsider.id),
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let mut entry = Map::new();
            if next_is_here && let Some(near) = here.next() {
                let other = Idx(near.idx);
                entry.insert("id".into(), json!(near.id));
                entry.insert("name".into(), json!(identity[other.index()].name));
                entry.insert("pos".into(), json!(self.pos(other)));
                if let Some(activity) = self.activity(other) {
                    entry.insert("activity".into(), json!(activity.verb));
                }
            } else if let Some((_, outsider)) = outside.next() {
                entry.insert("id".into(), json!(outsider.id));
                entry.insert("name".into(), json!(outsider.name));
                entry.insert("pos".into(), json!(outsider.pos));
                if let Some(activity) = outsider.activity {
                    entry.insert("activity".into(), json!(activity));
                }
                entry.insert("zone".into(), json!(outsider.zone));
            }
            residents.push(Value::Object(entry));
        }

        let sites = self.sites.borrow();
        let within = sites.within(pos, radius);
        let total_sites = within.len();
        let site_list: Vec<Value> = within
            .iter()
            .take(max)
            .map(|site| json!({ "id": site.id, "name": site.name, "kind": site.kind, "pos": site.rect.nearest(pos) }))
            .collect();

        let mut out = Map::new();
        out.insert("residents".into(), json!(residents));
        if total_residents > max {
            out.insert("more_residents".into(), json!(total_residents - max));
        }
        out.insert("sites".into(), json!(site_list));
        if total_sites > max {
            out.insert("more_sites".into(), json!(total_sites - max));
        }
        Value::Object(out)
    }

    /// Returns the public view of this zone, for viewers.
    pub(crate) fn zone_view(&self) -> ZoneView {
        let zone = self.zone_ref();
        ZoneView {
            id: zone.id,
            city: zone.city,
            name: zone.name.clone(),
            bounds: zone.bounds,
            residents: self.len() as u32,
        }
    }

    /// Returns everything that viewers can see in this zone: tiles, sites
    /// and residents in id order.
    pub(crate) fn world_view(&self) -> WorldView {
        let tiles = self.tiles.borrow();
        let mut residents: Vec<ResidentView> =
            self.indices().map(|idx| self.resident_view(idx)).collect();
        residents.sort_unstable_by_key(|r| r.id);
        WorldView::Zone {
            zone: self.zone_view(),
            width: tiles.width(),
            height: tiles.height(),
            tiles: tiles.as_slice().to_vec(),
            sites: self.sites.borrow().iter().map(|site| site.view()).collect(),
            residents,
        }
    }
}
