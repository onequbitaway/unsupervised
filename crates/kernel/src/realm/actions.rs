//! The action engine: submit, begin, finish, queue, routines.
//!
//! Rules (see `docs/design/architecture.md`, section 5.2):
//!
//! - A resident runs at most one non-free action at a time.
//! - A request for an idle resident begins at once. A request for a busy
//!   resident goes to the queue.
//! - A free verb runs at once, even when the resident is busy, and is
//!   limited per resident per tick.
//! - A blocking condition rejects every verb that does not allow it.
//! - Every accepted action whose result was not in the acknowledgement ends
//!   with exactly one report in the inbox of the resident. Actions that a
//!   routine started report too; the reports are how an owner that was away
//!   learns what its resident did. The parameter `kernel.routine_reports`
//!   can limit this to failed routine actions, for a world where the
//!   reports of routines push other notices out of the bounded inbox. The
//!   routine counts its successes (`runs`) either way.
//! - When an action ends, the next queued request starts. When the queue is
//!   empty, the routines are checked.
//!
//! The engine never calls module code while it holds a borrow of a kernel
//! cell. Module code that ends an action (`interrupt`, `complete_action`)
//! only records the end; the next action starts at the next settle point,
//! when no module borrow is alive.

use serde_json::{Value, json};
use unsup_protocol::{
    ActionAck, ActionError, ActionRequest, ActionStatus, Condition, ErrorCode, ResidentId, Routine,
    Tick,
};

use super::{F_ACTIVITY, F_BUSY, F_ROUTINES, Realm};
use crate::agenda::{
    ActivityView, CONDITION_NODES_MAX, Cond, Lapsed, NoticeBody, PackedArgs, PackedBody, Queued,
    RECENT_FIRST, RECENT_MAX, REQUEST_ID_MAX, Remembered, RoutineSlot, Running, hour_between,
};
use crate::defs::AttrShow;
use crate::event::EventMeta;
use crate::ids::{Idx, VerbId};
use crate::ledger::Holder;
use crate::message::{ActionDone, Dest, GlobalAct, KernelMsg};
use crate::plugin::{ActionCtx, Begin, GlobalCtx, VerbEntry, VerbKind};
use crate::timers::{Timer, TimerKind};

/// Deepest nesting of `all` and `any` in a routine condition.
const CONDITION_DEPTH_MAX: usize = 8;

/// Greatest length of a routine name in bytes.
const ROUTINE_NAME_MAX: usize = 64;

fn rejected(error: ActionError) -> ActionStatus {
    ActionStatus::Rejected { error }
}

impl Realm {
    // ---- Agenda -------------------------------------------------------------

    /// Returns what a resident is doing: the verb of its running action, when
    /// it started and when it ends. `None` when the resident is idle.
    #[must_use]
    pub fn activity(&self, idx: Idx) -> Option<ActivityView> {
        self.agenda.borrow()[idx.index()].current.as_ref().map(|running| ActivityView {
            verb: self.bp.verb_name(running.verb),
            action_id: running.action_id,
            started: Tick(running.started),
            ends: Tick(running.ends),
            waiting: running.wait,
        })
    }

    /// Returns the number of planned actions in the queue of a resident.
    #[must_use]
    pub fn queue_len(&self, idx: Idx) -> usize {
        self.agenda.borrow()[idx.index()].queue.len()
    }

    /// Fails the running action of a resident with the error `interrupted`.
    /// It does nothing when the resident is idle.
    ///
    /// `finish` of the verb does not run. The kernel emits the silent event
    /// `kernel.action_interrupted` with `{ resident, verb, action_id,
    /// reason }`; a module that reserved something in `begin` listens to it
    /// to give the reservation back.
    ///
    /// The queue is kept: the next planned action starts later in the same
    /// tick. Call [`Realm::clear_queue`] first, or set a blocking condition
    /// first, when the plan must not go on.
    pub fn interrupt(&self, idx: Idx, reason: &str) {
        self.guard_write();
        if let Some(running) = self.take_running(idx) {
            self.emit_interrupted(self.id_of(idx), &running, "interrupted");
            self.note_lapsed(idx, &running);
            self.end_action(idx, running, Err(ActionError::new(ErrorCode::Interrupted, reason)));
        }
    }

    /// Removes every planned action from the queue of a resident. Each gets
    /// a report with the error `interrupted`. Returns how many were removed.
    pub fn clear_queue(&self, idx: Idx) -> usize {
        self.guard_write();
        let queue = std::mem::take(&mut self.agenda.borrow_mut()[idx.index()].queue);
        let count = queue.len();
        for queued in queue {
            self.report(
                idx,
                queued.action_id,
                queued.request_id,
                queued.verb,
                Err(ActionError::new(
                    ErrorCode::Interrupted,
                    "The action was removed from the queue.",
                )),
            );
        }
        count
    }

    /// Ends an action that waits (`Begin::Wait`) with the given result. The
    /// result goes to the action report.
    ///
    /// It works from every realm: when the resident is not in this realm the
    /// kernel routes the call to its zone, where it applies in the next
    /// frame. A call that names no action of the resident (a wrong id, an
    /// action that was already completed) does nothing.
    ///
    /// The action can have ended before the answer comes: it was
    /// interrupted or cancelled, or it timed out. Its report then says so,
    /// but the work behind the answer can still have taken effect. With the
    /// parameter `kernel.late_reports` at 1 the kernel writes the late
    /// answer as one more report with the same action id, so that the
    /// resident learns the real outcome. It does so once for an action, for
    /// the last 8 such actions of a resident. With the parameter at 0 (the
    /// default) the late answer is dropped.
    pub fn complete_action(
        &self,
        resident: ResidentId,
        action_id: u64,
        r: Result<Value, ActionError>,
    ) {
        self.guard_write();
        match self.idx_of(resident) {
            Some(idx) => {
                if let Some(running) =
                    self.take_running_if(idx, |run| run.action_id == action_id && run.wait)
                {
                    self.end_action(idx, running, r);
                    return;
                }
                // The answer of an action that ended before it came: the
                // resident still learns the truth.
                let lapsed = self.agenda.borrow_mut()[idx.index()].take_lapsed(action_id);
                if let Some(lapsed) = lapsed {
                    self.report(idx, lapsed.action_id, lapsed.request_id, lapsed.verb, r);
                }
            }
            None => self.push_kernel(
                Dest::Resident(resident),
                KernelMsg::Complete { resident, action_id, result: r },
            ),
        }
    }

    // ---- Submit -------------------------------------------------------------

    /// Handles one action request of a resident and returns the
    /// acknowledgement.
    pub(crate) fn submit(&self, idx: Idx, request: ActionRequest) -> ActionAck {
        if let Some(key) = &request.id {
            if key.len() > REQUEST_ID_MAX {
                let message = format!("The request id is longer than {REQUEST_ID_MAX} bytes.");
                return ActionAck {
                    request_id: None,
                    action_id: 0,
                    verb: request.verb,
                    status: rejected(ActionError::invalid_args(message)),
                };
            }
            let agenda = self.agenda.borrow();
            if let Some(first) = agenda[idx.index()].recent.iter().find(|r| r.has_key(key)) {
                return first.ack();
            }
        }
        let action_id = self.alloc_id();
        let ActionRequest { id: request_id, verb, args } = request;
        let status = self.submit_status(idx, &verb, request_id.clone(), args, action_id);
        let ack = ActionAck { request_id, action_id, verb, status };
        // A rejected request changed nothing, so the same key may be tried
        // again. Only an accepted request is remembered.
        if ack.status.is_accepted()
            && let Some(remembered) = Remembered::new(&ack)
        {
            let mut agenda = self.agenda.borrow_mut();
            let recent = &mut agenda[idx.index()].recent;
            if recent.len() >= RECENT_MAX {
                recent.remove(0);
            }
            // Room for a few, then room for all: a client that sends keys
            // fills the list soon, and a list that grows step by step
            // leaves its smaller buffers behind.
            if recent.len() == recent.capacity() {
                let room = if recent.is_empty() { RECENT_FIRST } else { RECENT_MAX };
                recent.reserve_exact(room.saturating_sub(recent.len()));
            }
            recent.push(remembered);
        }
        ack
    }

    fn submit_status(
        &self,
        idx: Idx,
        verb: &str,
        request_id: Option<String>,
        args: Value,
        action_id: u64,
    ) -> ActionStatus {
        let unknown = || {
            rejected(ActionError::new(
                ErrorCode::UnknownVerb,
                format!("No verb is named `{verb}`."),
            ))
        };
        let Some(verb_id) = self.bp.verb_id(verb) else {
            return unknown();
        };
        let entry = self.bp.verb(verb_id);
        if matches!(entry.kind, VerbKind::Retired) {
            return unknown();
        }
        if let Some(error) = self.blocked(idx, entry) {
            return rejected(error);
        }
        if let Err(error) = (entry.validate)(&args) {
            return rejected(error);
        }
        let tick = self.tick.get();
        if entry.free {
            let limit = self.param(self.bp.kernel_ids().free_actions_per_tick);
            {
                let mut agenda = self.agenda.borrow_mut();
                let a = &mut agenda[idx.index()];
                if a.free_tick != tick {
                    a.free_tick = tick;
                    a.free_used = 0;
                }
                if i64::from(a.free_used) >= limit {
                    return rejected(ActionError::new(
                        ErrorCode::RateLimited,
                        format!("At most {limit} free actions in one tick."),
                    ));
                }
                a.free_used += 1;
            }
            return self.begin(idx, verb_id, request_id, args, action_id, None);
        }
        {
            let mut agenda = self.agenda.borrow_mut();
            let a = &mut agenda[idx.index()];
            if a.current.is_some() || !a.queue.is_empty() {
                let max = self.param(self.bp.kernel_ids().queue_max);
                if a.queue.len() as i64 >= max {
                    return rejected(ActionError::new(
                        ErrorCode::QueueFull,
                        format!("The queue holds at most {max} planned actions."),
                    ));
                }
                let args = PackedArgs::new(&args);
                a.queue.push(Queued { action_id, verb: verb_id, request_id, args });
                let position = a.queue.len() as u32;
                if a.current.is_none() {
                    // The queue is about to be served; keep the order.
                    self.restart.borrow_mut().push(self.id_of(idx));
                }
                return ActionStatus::Queued { position };
            }
        }
        self.begin(idx, verb_id, request_id, args, action_id, None)
    }

    /// Returns why the conditions of a resident forbid a verb, if they do.
    fn blocked(&self, idx: Idx, entry: &VerbEntry) -> Option<ActionError> {
        let tick = self.tick.get();
        let frozen = self.bp.kernel_ids().frozen;
        let conds = self.conds.borrow();
        for &(cond, until) in &conds[idx.index()] {
            if until <= tick {
                continue;
            }
            let forbidden = cond == frozen
                || (self.bp.is_blocking(cond)
                    && !entry.allow_all
                    && !entry.allowed.contains(&cond));
            if forbidden {
                let name = &self.bp.condition(cond).name;
                return Some(
                    ActionError::new(
                        ErrorCode::Blocked,
                        format!("You cannot use `{}` while you are {name}.", entry.name),
                    )
                    .with_details(json!({ "condition": name })),
                );
            }
        }
        None
    }

    /// Starts an action now. The caller checked that the resident is idle,
    /// or the verb is free.
    fn begin(
        &self,
        idx: Idx,
        verb: VerbId,
        request_id: Option<String>,
        args: Value,
        action_id: u64,
        routine: Option<u8>,
    ) -> ActionStatus {
        let entry = self.bp.verb(verb);
        let resident = self.id_of(idx);
        let tick = self.tick.get();
        match &entry.kind {
            VerbKind::Zone { begin, .. } => {
                let ctx = ActionCtx { resident, idx, action_id, started: Tick(tick) };
                // No kernel cell is borrowed here: `begin` is module code.
                let (ticks, wait) = match begin(self, &ctx, &args) {
                    Ok(Begin::Done(result)) => return ActionStatus::Done { result },
                    Err(error) => return rejected(error),
                    Ok(Begin::Busy { ticks }) => (ticks.max(1), false),
                    Ok(Begin::Wait { timeout }) => (timeout.max(1), true),
                };
                if entry.free {
                    return rejected(ActionError::internal(format!(
                        "The free verb `{}` did not finish at once.",
                        entry.name
                    )));
                }
                let ends = tick + u64::from(ticks);
                self.set_running(
                    idx,
                    Running {
                        action_id,
                        verb,
                        request_id,
                        args: PackedArgs::new(&args),
                        started: tick,
                        ends,
                        wait,
                        routine,
                    },
                );
                if wait {
                    ActionStatus::Pending
                } else {
                    ActionStatus::Started { ends_at: Tick(ends), ticks }
                }
            }
            VerbKind::Global { .. } => {
                let zone = self.zone_ref().id;
                self.push_kernel(
                    Dest::Global,
                    KernelMsg::GlobalAct(GlobalAct {
                        resident,
                        action_id,
                        verb,
                        request_id: request_id.clone(),
                        args,
                        zone,
                        pos: self.pos(idx),
                        free: entry.free,
                    }),
                );
                if !entry.free {
                    let timeout = self.param(self.bp.kernel_ids().global_timeout).max(1) as u64;
                    self.set_running(
                        idx,
                        Running {
                            action_id,
                            verb,
                            request_id,
                            args: PackedArgs::default(),
                            started: tick,
                            ends: tick + timeout,
                            wait: true,
                            routine,
                        },
                    );
                }
                ActionStatus::Pending
            }
            VerbKind::Retired => rejected(ActionError::new(
                ErrorCode::UnknownVerb,
                format!("The verb `{}` no longer exists.", entry.name),
            )),
        }
    }

    // ---- Running action -------------------------------------------------------

    fn set_running(&self, idx: Idx, running: Running) {
        let timer =
            Timer { resident: self.id_of(idx), kind: TimerKind::Action, key: running.action_id };
        self.timers.borrow_mut().insert(self.tick.get(), running.ends, timer);
        let verb = running.verb;
        self.agenda.borrow_mut()[idx.index()].current = Some(running);
        self.flags.borrow_mut()[idx.index()] |= F_BUSY;
        self.mark_dirty(idx, F_ACTIVITY);
        self.border_started(idx, verb);
    }

    fn take_running(&self, idx: Idx) -> Option<Running> {
        self.take_running_if(idx, |_| true)
    }

    fn take_running_if(&self, idx: Idx, test: impl Fn(&Running) -> bool) -> Option<Running> {
        let mut agenda = self.agenda.borrow_mut();
        let slot = &mut agenda[idx.index()].current;
        if slot.as_ref().is_some_and(test) {
            self.flags.borrow_mut()[idx.index()] &= !F_BUSY;
            slot.take()
        } else {
            None
        }
    }

    /// Writes the report of an action to the inbox of the resident.
    fn report(
        &self,
        idx: Idx,
        action_id: u64,
        request_id: Option<String>,
        verb: VerbId,
        result: Result<Value, ActionError>,
    ) {
        let body =
            NoticeBody::Report { tick: self.tick.get(), action_id, request_id, verb, result };
        self.push_inbox(idx, PackedBody::new(&body));
    }

    /// Records the end of an action that occupied the resident: the report,
    /// the routine bookkeeping, and the request to start what comes next.
    fn end_action(&self, idx: Idx, running: Running, result: Result<Value, ActionError>) {
        self.note_routine_result(idx, running.routine, result.is_ok(), false);
        if running.routine.is_none() || result.is_err() || self.routine_reports() {
            self.report(idx, running.action_id, running.request_id, running.verb, result);
        }
        self.mark_dirty(idx, F_ACTIVITY);
        self.border_ended(idx, running.verb);
        self.restart.borrow_mut().push(self.id_of(idx));
    }

    /// Remembers a waiting action that ends now without its answer, so that
    /// a late `complete_action` can still be reported. The answer of a
    /// global verb comes back by itself (`ActionDone`) and needs no entry.
    fn note_lapsed(&self, idx: Idx, running: &Running) {
        if !running.wait
            || self.bp.verb(running.verb).is_global()
            || self.param(self.bp.kernel_ids().late_reports) == 0
        {
            return;
        }
        self.agenda.borrow_mut()[idx.index()].remember_lapsed(Lapsed {
            action_id: running.action_id,
            verb: running.verb,
            request_id: running.request_id.clone(),
        });
    }

    /// Returns true when a routine action that succeeds writes a report.
    fn routine_reports(&self) -> bool {
        self.param(self.bp.kernel_ids().routine_reports) != 0
    }

    fn note_routine_result(&self, idx: Idx, routine: Option<u8>, ok: bool, instant: bool) {
        let Some(slot) = routine else {
            return;
        };
        let backoff = self.param(self.bp.kernel_ids().routine_backoff).max(1) as u64;
        let tick = self.tick.get();
        let mut agenda = self.agenda.borrow_mut();
        if let Some(slot) = agenda[idx.index()].routines.get_mut(usize::from(slot)) {
            if ok {
                slot.succeeded(tick, backoff, instant);
            } else {
                slot.failed(tick, backoff);
            }
        }
    }

    fn emit_interrupted(&self, resident: ResidentId, running: &Running, reason: &str) {
        const KIND: &str = "kernel.action_interrupted";
        if self.def().wants_event(KIND) || self.bp.global.wants_event(KIND) {
            self.emit(
                EventMeta::silent(KIND).actor(resident),
                &json!({
                    "resident": resident,
                    "verb": self.bp.verb_name(running.verb),
                    "action_id": running.action_id,
                    "reason": reason,
                }),
            );
        }
    }

    /// Ends the action that a timer names: `finish` for a busy action, a
    /// timeout for a waiting one. A timer for an action that already ended
    /// does nothing.
    pub(super) fn finish_due(&self, idx: Idx, action_id: u64) {
        let tick = self.tick.get();
        let Some(running) =
            self.take_running_if(idx, |run| run.action_id == action_id && run.ends <= tick)
        else {
            return;
        };
        let resident = self.id_of(idx);
        if running.wait {
            self.emit_interrupted(resident, &running, "timeout");
            self.note_lapsed(idx, &running);
            let error =
                ActionError::new(ErrorCode::Timeout, "The action waited too long for an answer.");
            self.end_action(idx, running, Err(error));
            return;
        }
        let entry = self.bp.verb(running.verb);
        let result = match &entry.kind {
            VerbKind::Zone { finish, .. } => {
                let ctx = ActionCtx { resident, idx, action_id, started: Tick(running.started) };
                finish(self, &ctx, &running.args.open())
            }
            _ => Err(ActionError::internal(format!("The verb `{}` no longer exists.", entry.name))),
        };
        self.end_action(idx, running, result);
    }

    // ---- What comes next --------------------------------------------------------

    /// Starts the next planned action of an idle resident, or checks its
    /// routines when the queue is empty.
    pub(super) fn restart_resident(&self, resident: ResidentId) {
        let Some(idx) = self.idx_of(resident) else {
            return;
        };
        loop {
            let next = {
                let mut agenda = self.agenda.borrow_mut();
                let a = &mut agenda[idx.index()];
                if a.current.is_some() {
                    return;
                }
                if a.queue.is_empty() { None } else { Some(a.queue.remove(0)) }
            };
            let Some(queued) = next else {
                break;
            };
            let entry = self.bp.verb(queued.verb);
            let status = match self.blocked(idx, entry) {
                Some(error) => rejected(error),
                None => self.begin(
                    idx,
                    queued.verb,
                    queued.request_id.clone(),
                    queued.args.open(),
                    queued.action_id,
                    None,
                ),
            };
            match status {
                ActionStatus::Done { result } => {
                    self.report(idx, queued.action_id, queued.request_id, queued.verb, Ok(result));
                }
                ActionStatus::Rejected { error } => {
                    self.report(idx, queued.action_id, queued.request_id, queued.verb, Err(error));
                }
                _ => return,
            }
        }
        self.try_routines(idx);
    }

    /// Fires the first routine of an idle resident whose condition holds.
    /// At most one routine fires per resident per tick.
    pub(super) fn try_routines(&self, idx: Idx) {
        let tick = self.tick.get();
        let pick = {
            let agenda = self.agenda.borrow();
            let a = &agenda[idx.index()];
            if a.routines.is_empty() || a.current.is_some() || a.routine_tick == tick {
                return;
            }
            a.routines
                .iter()
                .position(|slot| slot.retry_at <= tick && self.eval(idx, &slot.cond))
                .map(|i| (i, a.routines[i].verb, a.routines[i].args.open()))
        };
        let Some((slot, verb, args)) = pick else {
            return;
        };
        if self.has_condition(idx, self.bp.kernel_ids().frozen) {
            return;
        }
        self.agenda.borrow_mut()[idx.index()].routine_tick = tick;
        let action_id = self.alloc_id();
        let slot = slot as u8;
        let entry = self.bp.verb(verb);
        let status = match self.blocked(idx, entry) {
            Some(error) => rejected(error),
            None => self.begin(idx, verb, None, args, action_id, Some(slot)),
        };
        match status {
            ActionStatus::Done { result } => {
                self.note_routine_result(idx, Some(slot), true, true);
                if self.routine_reports() {
                    self.report(idx, action_id, None, verb, Ok(result));
                }
            }
            ActionStatus::Rejected { error } => {
                self.note_routine_result(idx, Some(slot), false, true);
                self.report(idx, action_id, None, verb, Err(error));
            }
            // A free global verb: the answer comes later as a report. Hold
            // the routine back so that it cannot fire every tick.
            ActionStatus::Pending if entry.free => {
                self.note_routine_result(idx, Some(slot), true, true)
            }
            _ => {}
        }
    }

    /// Evaluates a routine condition for a resident.
    fn eval(&self, idx: Idx, cond: &Cond) -> bool {
        match cond {
            Cond::Always => true,
            Cond::AttrBelow(attr, value) => self.attr(idx, *attr) < *value,
            Cond::AttrAbove(attr, value) => self.attr(idx, *attr) > *value,
            Cond::ClockBetween(from, to) => hour_between(self.clock().hour, *from, *to),
            Cond::HasItem(item, qty) => {
                self.ledger.borrow().qty(Holder::Resident(self.id_of(idx)), *item) >= *qty
            }
            Cond::All(list) => list.iter().all(|c| self.eval(idx, c)),
            Cond::Any(list) => list.iter().any(|c| self.eval(idx, c)),
        }
    }

    /// Resolves the names of a routine condition.
    fn compile(
        &self,
        cond: &Condition,
        nodes: &mut usize,
        depth: usize,
    ) -> Result<Cond, ActionError> {
        *nodes += 1;
        if *nodes > CONDITION_NODES_MAX || depth > CONDITION_DEPTH_MAX {
            return Err(ActionError::invalid_args(format!(
                "A routine condition has at most {CONDITION_NODES_MAX} parts and {CONDITION_DEPTH_MAX} levels."
            )));
        }
        let attr_id = |name: &str| {
            self.bp
                .attr_id(name)
                .filter(|id| self.bp.attr(*id).show != AttrShow::Hidden)
                .ok_or_else(|| {
                    ActionError::invalid_args(format!("No attribute is named `{name}`."))
                })
        };
        let list = |of: &[Condition], nodes: &mut usize| {
            of.iter().map(|c| self.compile(c, nodes, depth + 1)).collect::<Result<Vec<_>, _>>()
        };
        Ok(match cond {
            Condition::Always => Cond::Always,
            Condition::AttrBelow { attr, value } => Cond::AttrBelow(attr_id(attr)?, *value),
            Condition::AttrAbove { attr, value } => Cond::AttrAbove(attr_id(attr)?, *value),
            Condition::ClockBetween { from_hour, to_hour } => {
                if *from_hour > 23 || *to_hour > 24 {
                    return Err(ActionError::invalid_args(
                        "`from_hour` is from 0 to 23 and `to_hour` is from 0 to 24.",
                    ));
                }
                Cond::ClockBetween(*from_hour, *to_hour)
            }
            Condition::HasItem { item, qty } => {
                let id = self.bp.item_id(item).ok_or_else(|| {
                    ActionError::invalid_args(format!("No item is named `{item}`."))
                })?;
                Cond::HasItem(id, *qty)
            }
            Condition::All { of } => Cond::All(list(of, nodes)?),
            Condition::Any { of } => Cond::Any(list(of, nodes)?),
        })
    }

    // ---- Kernel verbs -------------------------------------------------------------

    /// The verb `cancel`: stops the running action and clears the queue.
    pub(crate) fn cancel_actions(&self, idx: Idx) -> Value {
        let cleared = self.clear_queue(idx);
        let cancelled = self.take_running(idx).map(|running| {
            let action_id = running.action_id;
            self.emit_interrupted(self.id_of(idx), &running, "cancelled");
            self.note_lapsed(idx, &running);
            let error = ActionError::new(ErrorCode::Interrupted, "You cancelled the action.");
            self.end_action(idx, running, Err(error));
            action_id
        });
        json!({ "cancelled": cancelled, "cleared": cleared })
    }

    /// Replaces the routines of a resident. An empty list removes them.
    ///
    /// It does what the verb `set_routines` does, for module code: for
    /// example an `on_spawn` hook that gives a new resident its default
    /// routines, so that it lives when its owner is away.
    ///
    /// The list is checked as a whole and a refused list changes nothing.
    /// It is refused with `invalid_args` when it has more routines than the
    /// parameter `kernel.routines_max`, when a name is too long, when a
    /// condition names an unknown attribute or item or is too large, when
    /// the arguments of an action do not fit its verb, and when an action
    /// uses `set_routines` or `cancel`. It is refused with `unknown_verb`
    /// when an action names no verb.
    ///
    /// The running action goes on, but it no longer counts for a routine.
    pub fn set_routines(&self, idx: Idx, routines: &[Routine]) -> Result<(), ActionError> {
        self.guard_write();
        let max = self.param(self.bp.kernel_ids().routines_max);
        if routines.len() as i64 > max {
            return Err(ActionError::invalid_args(format!(
                "A resident has at most {max} routines."
            )));
        }
        let mut slots = Vec::with_capacity(routines.len());
        for routine in routines {
            if routine.name.len() > ROUTINE_NAME_MAX {
                return Err(ActionError::invalid_args(format!(
                    "A routine name has at most {ROUTINE_NAME_MAX} bytes."
                )));
            }
            let cond = self.compile(&routine.when, &mut 0, 0)?;
            let name = routine.then.verb.as_str();
            let verb = self
                .bp
                .verb_id(name)
                .filter(|id| !matches!(self.bp.verb(*id).kind, VerbKind::Retired))
                .ok_or_else(|| {
                    ActionError::new(ErrorCode::UnknownVerb, format!("No verb is named `{name}`."))
                })?;
            if matches!(name, "set_routines" | "cancel") {
                return Err(ActionError::invalid_args(format!(
                    "A routine cannot use the verb `{name}`."
                )));
            }
            (self.bp.verb(verb).validate)(&routine.then.args)?;
            let mut routine = routine.clone();
            routine.then.id = None;
            slots.push(RoutineSlot::new(routine, cond, verb));
        }
        let count = slots.len();
        let had = {
            let mut agenda = self.agenda.borrow_mut();
            let a = &mut agenda[idx.index()];
            let had = !a.routines.is_empty();
            a.routines = slots;
            // The running action no longer belongs to a routine of the list.
            if let Some(running) = &mut a.current {
                running.routine = None;
            }
            had
        };
        self.note_routines(idx, had, count > 0);
        Ok(())
    }

    /// Keeps the routine flag and the routine counter right after the list
    /// of routines of a resident changed.
    pub(super) fn note_routines(&self, idx: Idx, had: bool, has: bool) {
        let mut flags = self.flags.borrow_mut();
        if has {
            flags[idx.index()] |= F_ROUTINES;
        } else {
            flags[idx.index()] &= !F_ROUTINES;
        }
        match (had, has) {
            (false, true) => self.routine_holders.set(self.routine_holders.get() + 1),
            (true, false) => self.routine_holders.set(self.routine_holders.get().saturating_sub(1)),
            _ => {}
        }
    }

    /// Stops a resident for an administrator: its queue is cleared, its
    /// action fails and the condition `frozen` rejects every later request.
    pub(crate) fn freeze(&self, idx: Idx) {
        self.set_condition(idx, self.bp.kernel_ids().frozen, None);
        self.clear_queue(idx);
        self.interrupt(idx, "An administrator froze this resident.");
    }

    /// Undoes [`Realm::freeze`].
    pub(crate) fn unfreeze(&self, idx: Idx) {
        self.clear_condition(idx, self.bp.kernel_ids().frozen);
    }

    // ---- Global verbs ---------------------------------------------------------------

    /// Runs a global verb in the global realm and sends the answer to the
    /// zone of the resident.
    pub(super) fn handle_global_act(&self, act: GlobalAct) {
        let entry = self.bp.verb(act.verb);
        let result = match &entry.kind {
            VerbKind::Global { run } => {
                let ctx = GlobalCtx {
                    resident: act.resident,
                    action_id: act.action_id,
                    zone: act.zone,
                    pos: act.pos,
                };
                run(self, &ctx, &act.args)
            }
            _ => Err(ActionError::internal(format!(
                "The verb `{}` does not run in the global realm.",
                entry.name
            ))),
        };
        self.push_kernel(
            Dest::Resident(act.resident),
            KernelMsg::ActionDone(ActionDone {
                resident: act.resident,
                action_id: act.action_id,
                verb: act.verb,
                request_id: act.request_id,
                free: act.free,
                result,
            }),
        );
    }

    /// Takes the answer of the global realm for a resident of this zone.
    pub(super) fn handle_action_done(&self, idx: Idx, done: ActionDone) {
        if !done.free
            && let Some(running) = self.take_running_if(idx, |run| run.action_id == done.action_id)
        {
            self.end_action(idx, running, done.result);
            return;
        }
        // The answer of a free verb, or a late answer for an action that
        // timed out or was interrupted: the resident still learns the truth.
        self.report(idx, done.action_id, done.request_id, done.verb, done.result);
    }
}
