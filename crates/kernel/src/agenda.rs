//! Per-resident action state: the running action, the queue, the routines,
//! and the inbox of notices and reports.

use std::collections::VecDeque;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{
    ActionAck, ActionError, ActionReport, ItemId, Notice, ResidentId, Routine, Tick,
};

use crate::ids::{AttrId, VerbId};

/// Number of request ids that the kernel remembers per resident.
pub(crate) const RECENT_MAX: usize = 32;

/// Greatest length of a request id in bytes.
pub(crate) const REQUEST_ID_MAX: usize = 64;

/// Greatest number of conditions in one routine, counting nested ones.
pub(crate) const CONDITION_NODES_MAX: usize = 32;

/// Greatest doubling of the delay of a routine that keeps failing.
const BACKOFF_SHIFT_MAX: u8 = 4;

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

fn is_zero_u8(v: &u8) -> bool {
    *v == 0
}

fn is_false(v: &bool) -> bool {
    !*v
}

/// The action state of one resident.
///
/// Field names are short on the wire because one agenda is written per
/// resident in every snapshot. An idle resident without routines writes an
/// empty map.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Agenda {
    /// The action that occupies the resident.
    #[serde(rename = "c", default, skip_serializing_if = "Option::is_none")]
    pub(crate) current: Option<Running>,
    /// Planned actions, first in first out.
    #[serde(rename = "q", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) queue: Vec<Queued>,
    /// Standing orders, in priority order.
    #[serde(rename = "r", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) routines: Vec<RoutineSlot>,
    /// The last accepted request ids with their first answer.
    #[serde(rename = "i", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) recent: Vec<(String, ActionAck)>,
    /// The tick in which `free_used` was counted.
    #[serde(rename = "ft", default, skip_serializing_if = "is_zero_u64")]
    pub(crate) free_tick: u64,
    /// Free actions taken in tick `free_tick`.
    #[serde(rename = "fu", default, skip_serializing_if = "is_zero_u32")]
    pub(crate) free_used: u32,
    /// The last tick in which a routine fired.
    #[serde(rename = "rt", default, skip_serializing_if = "is_zero_u64")]
    pub(crate) routine_tick: u64,
}

impl Agenda {
    /// Moves every tick in the agenda from the time base of one zone to the
    /// time base of another. Per-tick counters start again.
    pub(crate) fn rebase(&mut self, from: u64, to: u64) {
        if let Some(running) = &mut self.current {
            running.started = rebase_tick(running.started, from, to);
            running.ends = rebase_tick(running.ends, from, to);
        }
        for slot in &mut self.routines {
            slot.retry_at = rebase_tick(slot.retry_at, from, to);
        }
        self.free_tick = 0;
        self.free_used = 0;
        self.routine_tick = 0;
    }

    /// Returns an estimate of the heap bytes that the agenda uses.
    pub(crate) fn memory_estimate(&self) -> usize {
        self.current
            .as_ref()
            .map_or(0, |r| r.request_id.as_ref().map_or(0, String::capacity) + value_bytes(&r.args))
            + self.queue.capacity() * size_of::<Queued>()
            + self.queue.iter().map(|q| value_bytes(&q.args)).sum::<usize>()
            + self.routines.capacity() * size_of::<RoutineSlot>()
            + self
                .routines
                .iter()
                .map(|slot| {
                    slot.routine.name.capacity()
                        + slot.routine.then.verb.capacity()
                        + value_bytes(&slot.routine.then.args)
                })
                .sum::<usize>()
            + self.recent.capacity() * (size_of::<(String, ActionAck)>() + 32)
    }
}

/// Returns an estimate of the heap bytes behind a JSON value: strings, and
/// about 48 bytes of tree node per array element or object entry.
pub(crate) fn value_bytes(value: &Value) -> usize {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
        Value::String(text) => text.capacity(),
        Value::Array(list) => {
            list.capacity() * size_of::<Value>() + list.iter().map(value_bytes).sum::<usize>()
        }
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| {
                key.capacity() + size_of::<String>() + size_of::<Value>() + 16 + value_bytes(value)
            })
            .sum(),
    }
}

/// Moves a tick from the time base of one zone to the time base of another.
/// `u64::MAX` means "never" and stays.
pub(crate) fn rebase_tick(tick: u64, from: u64, to: u64) -> u64 {
    if tick == u64::MAX {
        return tick;
    }
    let moved = i128::from(tick) - i128::from(from) + i128::from(to);
    moved.clamp(0, i128::from(u64::MAX - 1)) as u64
}

/// The action that occupies a resident.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Running {
    #[serde(rename = "a")]
    pub(crate) action_id: u64,
    #[serde(rename = "v")]
    pub(crate) verb: VerbId,
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
    #[serde(rename = "g", default, skip_serializing_if = "Value::is_null")]
    pub(crate) args: Value,
    /// Tick of the zone at which the action started.
    #[serde(rename = "s")]
    pub(crate) started: u64,
    /// Tick of the zone at which the action ends (busy) or times out (wait).
    #[serde(rename = "e")]
    pub(crate) ends: u64,
    /// True when the action waits for `complete_action` or for the global
    /// realm.
    #[serde(rename = "w", default, skip_serializing_if = "is_false")]
    pub(crate) wait: bool,
    /// Index of the routine that started the action, if one did.
    #[serde(rename = "o", default, skip_serializing_if = "Option::is_none")]
    pub(crate) routine: Option<u8>,
}

/// A planned action in the queue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Queued {
    #[serde(rename = "a")]
    pub(crate) action_id: u64,
    #[serde(rename = "v")]
    pub(crate) verb: VerbId,
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
    #[serde(rename = "g", default, skip_serializing_if = "Value::is_null")]
    pub(crate) args: Value,
}

/// One routine with its compiled condition and its retry state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct RoutineSlot {
    /// The routine as the owner wrote it.
    pub(crate) routine: Routine,
    /// The condition with names resolved to ids.
    pub(crate) cond: Cond,
    /// The verb of `routine.then`.
    pub(crate) verb: VerbId,
    /// The routine does not fire before this tick.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub(crate) retry_at: u64,
    /// Failures in a row.
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub(crate) failures: u8,
    /// How many actions of this routine succeeded. The owner sees it in an
    /// observation with full detail, also after the reports have left the
    /// bounded inbox or when `kernel.routine_reports` is 0.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub(crate) runs: u64,
}

impl RoutineSlot {
    /// Records that the action of the routine failed. The routine waits
    /// `backoff` ticks, doubled for each failure in a row, up to 16 times.
    pub(crate) fn failed(&mut self, tick: u64, backoff: u64) {
        let shift = self.failures.min(BACKOFF_SHIFT_MAX);
        self.failures = self.failures.saturating_add(1);
        self.retry_at = tick.saturating_add(backoff.max(1) << shift);
    }

    /// Records that the action of the routine succeeded. `instant` says that
    /// it finished at once; the routine then waits `backoff` ticks so that
    /// it cannot fire every tick.
    pub(crate) fn succeeded(&mut self, tick: u64, backoff: u64, instant: bool) {
        self.failures = 0;
        self.runs += 1;
        if instant {
            self.retry_at = tick.saturating_add(backoff.max(1));
        }
    }
}

/// A routine condition with names resolved to ids.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Cond {
    Always,
    AttrBelow(AttrId, i64),
    AttrAbove(AttrId, i64),
    ClockBetween(u8, u8),
    HasItem(ItemId, i64),
    All(Vec<Cond>),
    Any(Vec<Cond>),
}

/// Returns true when `hour` is in the range from `from` (inclusive) to `to`
/// (exclusive). The range wraps past midnight when `from` is greater than
/// `to`. It is empty when the two are equal.
pub(crate) fn hour_between(hour: u8, from: u8, to: u8) -> bool {
    if from < to { from <= hour && hour < to } else { from > to && (hour >= from || hour < to) }
}

/// What a resident is doing now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActivityView {
    /// Name of the verb.
    pub verb: &'static str,
    /// Id of the action.
    pub action_id: u64,
    /// Tick of the zone at which the action started.
    pub started: Tick,
    /// Tick of the zone at which the action ends, or times out when
    /// `waiting` is true.
    pub ends: Tick,
    /// True when the action waits for an answer instead of a fixed time.
    pub waiting: bool,
}

/// The content of one inbox entry. Residents that receive the same notice
/// share one body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum NoticeBody {
    /// A notice: speech heard, a message, an event nearby.
    #[serde(rename = "n")]
    Notice {
        #[serde(rename = "t")]
        tick: u64,
        #[serde(rename = "k")]
        kind: Box<str>,
        #[serde(rename = "f", default, skip_serializing_if = "Option::is_none")]
        from: Option<ResidentId>,
        #[serde(rename = "d", default, skip_serializing_if = "Value::is_null")]
        data: Value,
    },
    /// The report of an action that ended.
    #[serde(rename = "r")]
    Report {
        #[serde(rename = "t")]
        tick: u64,
        #[serde(rename = "a")]
        action_id: u64,
        #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        #[serde(rename = "v")]
        verb: VerbId,
        #[serde(rename = "o")]
        result: Result<Value, ActionError>,
    },
}

/// One entry of an inbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct InboxEntry {
    #[serde(rename = "s")]
    pub(crate) seq: u64,
    #[serde(rename = "b")]
    pub(crate) body: Arc<NoticeBody>,
}

/// The bounded inbox of one resident.
///
/// Every entry gets the next sequence number of the resident, from 1. When
/// the inbox is full the oldest entry is dropped; sequence numbers are never
/// reused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Inbox {
    /// Number of entries ever written. The next entry gets `count + 1`.
    #[serde(rename = "n", default, skip_serializing_if = "is_zero_u64")]
    pub(crate) count: u64,
    #[serde(rename = "i", default, skip_serializing_if = "VecDeque::is_empty")]
    pub(crate) items: VecDeque<InboxEntry>,
}

impl Inbox {
    /// Adds an entry and drops the oldest entries above `max`.
    pub(crate) fn push(&mut self, body: Arc<NoticeBody>, max: usize) {
        self.count += 1;
        self.items.push_back(InboxEntry { seq: self.count, body });
        while self.items.len() > max.max(1) {
            self.items.pop_front();
        }
    }

    /// Returns the sequence number that the next entry will get.
    pub(crate) fn next_seq(&self) -> u64 {
        self.count + 1
    }

    /// Returns the entries with a sequence number at or above `since`.
    pub(crate) fn since(&self, since: u64) -> impl Iterator<Item = &InboxEntry> + '_ {
        // Entries are in sequence order, so the wanted ones are a suffix.
        let skip = self.items.partition_point(|e| e.seq < since);
        self.items.range(skip..)
    }

    /// Returns an estimate of the heap bytes that the inbox uses. A shared
    /// body is counted in every inbox that holds it.
    pub(crate) fn memory_estimate(&self) -> usize {
        // One allocation per body: the reference counts, the body, and
        // what the body points to.
        let bodies: usize = self
            .items
            .iter()
            .map(|entry| {
                16 + size_of::<NoticeBody>()
                    + match &*entry.body {
                        NoticeBody::Notice { kind, data, .. } => kind.len() + value_bytes(data),
                        NoticeBody::Report { request_id, result, .. } => {
                            request_id.as_ref().map_or(0, String::capacity)
                                + match result {
                                    Ok(value) => value_bytes(value),
                                    Err(error) => {
                                        error.message.capacity() + value_bytes(&error.details)
                                    }
                                }
                        }
                    }
            })
            .sum();
        self.items.capacity() * size_of::<InboxEntry>() + bodies
    }
}

/// An inbox entry as the protocol shows it.
pub(crate) enum Shown {
    Notice(Notice),
    Report(ActionReport),
}

impl InboxEntry {
    /// Converts the entry to its protocol form. `verb_name` gives the name
    /// of a verb id.
    pub(crate) fn show(&self, verb_name: impl Fn(VerbId) -> &'static str) -> Shown {
        match &*self.body {
            NoticeBody::Notice { tick, kind, from, data } => Shown::Notice(Notice {
                seq: self.seq,
                tick: Tick(*tick),
                kind: kind.to_string(),
                from: *from,
                data: data.clone(),
            }),
            NoticeBody::Report { tick, action_id, request_id, verb, result } => {
                let (ok, value, error) = match result {
                    Ok(value) => (true, value.clone(), None),
                    Err(error) => (false, Value::Null, Some(error.clone())),
                };
                Shown::Report(ActionReport {
                    action_id: *action_id,
                    request_id: request_id.clone(),
                    verb: verb_name(*verb).to_string(),
                    tick: Tick(*tick),
                    ok,
                    result: value,
                    error,
                })
            }
        }
    }
}

/// The kind of the notice that carries an action report, as `WaitFilter`
/// and the gateway see it.
pub const REPORT_KIND: &str = "report";

#[cfg(test)]
mod tests {
    use super::*;

    fn note(tick: u64) -> Arc<NoticeBody> {
        Arc::new(NoticeBody::Notice {
            tick,
            kind: "test.note".into(),
            from: None,
            data: Value::Null,
        })
    }

    #[test]
    fn inbox_is_bounded_and_numbers_never_repeat() {
        let mut inbox = Inbox::default();
        assert_eq!(inbox.next_seq(), 1);
        for tick in 0..10 {
            inbox.push(note(tick), 4);
        }
        assert_eq!(inbox.items.len(), 4);
        assert_eq!(inbox.next_seq(), 11);
        assert_eq!(inbox.since(0).map(|e| e.seq).collect::<Vec<_>>(), [7, 8, 9, 10]);
        assert_eq!(inbox.since(9).map(|e| e.seq).collect::<Vec<_>>(), [9, 10]);
        assert_eq!(inbox.since(11).count(), 0);
        // A smaller limit applies at the next push.
        inbox.push(note(10), 2);
        assert_eq!(inbox.since(0).map(|e| e.seq).collect::<Vec<_>>(), [10, 11]);
        // A limit of zero still keeps the newest entry.
        inbox.push(note(11), 0);
        assert_eq!(inbox.since(0).map(|e| e.seq).collect::<Vec<_>>(), [12]);
    }

    #[test]
    fn inbox_entries_show_as_protocol_types() {
        let mut inbox = Inbox::default();
        inbox.push(
            Arc::new(NoticeBody::Notice {
                tick: 5,
                kind: "society.speech".into(),
                from: Some(ResidentId(3)),
                data: serde_json::json!({ "text": "hi" }),
            }),
            8,
        );
        inbox.push(
            Arc::new(NoticeBody::Report {
                tick: 6,
                action_id: 44,
                request_id: Some("a".into()),
                verb: VerbId(2),
                result: Err(ActionError::not_found("No such shop.")),
            }),
            8,
        );
        let shown: Vec<Shown> = inbox.since(0).map(|e| e.show(|_| "buy")).collect();
        match &shown[0] {
            Shown::Notice(n) => {
                assert_eq!(
                    (n.seq, n.tick, n.kind.as_str(), n.from),
                    (1, Tick(5), "society.speech", Some(ResidentId(3)))
                );
            }
            Shown::Report(_) => panic!("the first entry is a notice"),
        }
        match &shown[1] {
            Shown::Report(r) => {
                assert_eq!(
                    (r.action_id, r.verb.as_str(), r.ok, r.tick),
                    (44, "buy", false, Tick(6))
                );
                assert_eq!(
                    r.error.as_ref().map(|e| e.code),
                    Some(unsup_protocol::ErrorCode::NotFound)
                );
            }
            Shown::Notice(_) => panic!("the second entry is a report"),
        }
        let bytes = crate::codec::encode("inbox", &inbox);
        let back: Inbox = crate::codec::decode("inbox", &bytes).unwrap();
        assert_eq!(back, inbox);
    }

    #[test]
    fn an_idle_agenda_is_one_byte_on_the_wire() {
        let agenda = Agenda::default();
        let bytes = crate::codec::encode("agenda", &agenda);
        assert_eq!(bytes, [0x80]);
        assert_eq!(crate::codec::decode::<Agenda>("agenda", &bytes).unwrap(), agenda);
    }

    #[test]
    fn rebase_moves_ticks_between_time_bases() {
        assert_eq!(rebase_tick(110, 100, 40), 50);
        assert_eq!(rebase_tick(90, 100, 40), 30);
        assert_eq!(rebase_tick(10, 100, 40), 0);
        assert_eq!(rebase_tick(u64::MAX, 100, 40), u64::MAX);

        let mut agenda = Agenda {
            current: Some(Running {
                action_id: 1,
                verb: VerbId(0),
                request_id: None,
                args: Value::Null,
                started: 95,
                ends: 108,
                wait: false,
                routine: None,
            }),
            free_tick: 100,
            free_used: 3,
            routine_tick: 99,
            ..Agenda::default()
        };
        agenda.rebase(100, 7);
        let running = agenda.current.as_ref().unwrap();
        assert_eq!((running.started, running.ends), (2, 15));
        assert_eq!((agenda.free_tick, agenda.free_used, agenda.routine_tick), (0, 0, 0));
        let bytes = crate::codec::encode("agenda", &agenda);
        assert_eq!(crate::codec::decode::<Agenda>("agenda", &bytes).unwrap(), agenda);
    }

    #[test]
    fn routine_backoff_doubles_and_resets() {
        let mut slot = RoutineSlot {
            routine: Routine {
                name: "r".into(),
                when: unsup_protocol::Condition::Always,
                then: unsup_protocol::ActionRequest::new("wait", Value::Null),
            },
            cond: Cond::Always,
            verb: VerbId(0),
            retry_at: 0,
            failures: 0,
            runs: 0,
        };
        let mut delays = Vec::new();
        for _ in 0..7 {
            slot.failed(1_000, 30);
            delays.push(slot.retry_at - 1_000);
        }
        assert_eq!(delays, [30, 60, 120, 240, 480, 480, 480]);
        slot.succeeded(2_000, 30, false);
        assert_eq!((slot.failures, slot.retry_at, slot.runs), (0, 1_480, 1));
        slot.succeeded(2_000, 30, true);
        assert_eq!((slot.retry_at, slot.runs), (2_030, 2));
    }

    #[test]
    fn hour_ranges_wrap_past_midnight() {
        assert!(hour_between(9, 9, 17) && hour_between(16, 9, 17));
        assert!(!hour_between(17, 9, 17) && !hour_between(8, 9, 17));
        assert!(hour_between(23, 22, 6) && hour_between(0, 22, 6) && hour_between(5, 22, 6));
        assert!(!hour_between(6, 22, 6) && !hour_between(12, 22, 6));
        assert!(!hour_between(3, 3, 3));
        assert!(hour_between(23, 0, 24));
    }
}
