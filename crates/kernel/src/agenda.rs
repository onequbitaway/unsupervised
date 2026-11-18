//! Per-resident action state: the running action, the queue, the routines,
//! and the inbox of notices and reports.

use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{Error, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use unsup_protocol::{
    ActionAck, ActionError, ActionReport, ItemId, Notice, ResidentId, Routine, Tick,
};

use crate::codec;
use crate::ids::{AttrId, VerbId};

/// Number of request ids that the kernel remembers per resident.
pub(crate) const RECENT_MAX: usize = 32;

/// Request ids that the first buffer of the list has room for.
pub(crate) const RECENT_FIRST: usize = 4;

/// Greatest length of a request id in bytes.
pub(crate) const REQUEST_ID_MAX: usize = 64;

/// Number of waiting actions that ended without their answer which the
/// kernel remembers per resident, so that it can report a late answer.
pub(crate) const LAPSED_MAX: usize = 8;

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
    pub(crate) recent: Vec<Remembered>,
    /// The tick in which `free_used` was counted.
    #[serde(rename = "ft", default, skip_serializing_if = "is_zero_u64")]
    pub(crate) free_tick: u64,
    /// Free actions taken in tick `free_tick`.
    #[serde(rename = "fu", default, skip_serializing_if = "is_zero_u32")]
    pub(crate) free_used: u32,
    /// The last tick in which a routine fired.
    #[serde(rename = "rt", default, skip_serializing_if = "is_zero_u64")]
    pub(crate) routine_tick: u64,
    /// Waiting actions that ended before their answer came, oldest first.
    /// An answer that comes late finds the verb and the request id here.
    #[serde(rename = "l", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) lapsed: Vec<Lapsed>,
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
        self.current.as_ref().map_or(0, |r| {
            r.request_id.as_ref().map_or(0, String::capacity) + r.args.memory_estimate()
        }) + self.queue.capacity() * size_of::<Queued>()
            + self.queue.iter().map(|q| q.args.memory_estimate()).sum::<usize>()
            + self.routines.capacity() * size_of::<RoutineSlot>()
            + self
                .routines
                .iter()
                .map(|slot| {
                    slot.routine.name.capacity()
                        + slot.routine.then.verb.capacity()
                        + slot.args.memory_estimate()
                })
                .sum::<usize>()
            + self.recent.capacity() * size_of::<Remembered>()
            + self.recent.iter().map(Remembered::memory_estimate).sum::<usize>()
            + self.lapsed.capacity() * size_of::<Lapsed>()
            + self
                .lapsed
                .iter()
                .map(|l| l.request_id.as_ref().map_or(0, String::capacity))
                .sum::<usize>()
    }

    /// Remembers a waiting action that ended without its answer. The oldest
    /// entry leaves when there are more than [`LAPSED_MAX`].
    pub(crate) fn remember_lapsed(&mut self, lapsed: Lapsed) {
        if self.lapsed.len() >= LAPSED_MAX {
            self.lapsed.remove(0);
        }
        self.lapsed.push(lapsed);
    }

    /// Takes the entry of a waiting action that ended without its answer.
    pub(crate) fn take_lapsed(&mut self, action_id: u64) -> Option<Lapsed> {
        let at = self.lapsed.iter().position(|l| l.action_id == action_id)?;
        Some(self.lapsed.remove(at))
    }
}

/// A waiting action that ended (interrupted, cancelled, timed out) before
/// `Realm::complete_action` gave its answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Lapsed {
    #[serde(rename = "a")]
    pub(crate) action_id: u64,
    #[serde(rename = "v")]
    pub(crate) verb: VerbId,
    #[serde(rename = "r", default, skip_serializing_if = "Option::is_none")]
    pub(crate) request_id: Option<String>,
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
    #[serde(rename = "g", default, skip_serializing_if = "PackedArgs::is_empty")]
    pub(crate) args: PackedArgs,
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
    #[serde(rename = "g", default, skip_serializing_if = "PackedArgs::is_empty")]
    pub(crate) args: PackedArgs,
}

/// One request that the kernel remembers: its idempotency key and the first
/// answer, in one block of bytes.
///
/// A client library sends a key with every request, so a resident soon has
/// [`RECENT_MAX`] of these. As a key string and an [`ActionAck`] with its
/// own strings and its tree of JSON values, one of them cost about 1,100
/// bytes. The block has one byte with the length of the key, the key, and
/// then the MessagePack bytes of the action id, the verb and the status.
/// The kernel compares the key in place and opens the rest only for a
/// repeat.
///
/// Every `Remembered` holds bytes that decode: it is made from an answer,
/// or it is checked when it is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Remembered(Box<[u8]>);

/// What a [`Remembered`] is, for error messages.
const REMEMBERED: &str = "a remembered request";

// The length of the key is one byte of the block.
const _: () = assert!(REQUEST_ID_MAX <= u8::MAX as usize);

impl Remembered {
    /// Packs the answer to a request. Returns `None` when the request had
    /// no key, or a key of more than 255 bytes; the kernel refuses a key of
    /// more than [`REQUEST_ID_MAX`] bytes before it comes here.
    pub(crate) fn new(ack: &ActionAck) -> Option<Remembered> {
        let key = ack.request_id.as_ref()?;
        let mut bytes = vec![u8::try_from(key.len()).ok()?];
        bytes.extend_from_slice(key.as_bytes());
        codec::encode_into(REMEMBERED, &(ack.action_id, &ack.verb, &ack.status), &mut bytes);
        Some(Remembered(bytes.into_boxed_slice()))
    }

    /// Splits a block into the key and the bytes of the answer, when the
    /// block is whole.
    fn split(bytes: &[u8]) -> Option<(&[u8], &[u8])> {
        let (&length, rest) = bytes.split_first()?;
        rest.split_at_checked(usize::from(length))
    }

    /// Opens a block, when it is whole and its parts decode.
    fn open(bytes: &[u8]) -> Result<ActionAck, codec::DecodeError> {
        let fail = |message: &str| codec::DecodeError {
            what: REMEMBERED.to_string(),
            message: message.to_string(),
        };
        let (key, answer) = Remembered::split(bytes).ok_or_else(|| fail("the block is cut"))?;
        let key = std::str::from_utf8(key).map_err(|_| fail("the key is not text"))?;
        let (action_id, verb, status) = codec::decode(REMEMBERED, answer)?;
        Ok(ActionAck { request_id: Some(key.to_string()), action_id, verb, status })
    }

    /// Returns true when the request had this key.
    pub(crate) fn has_key(&self, key: &str) -> bool {
        Remembered::split(&self.0).is_some_and(|(mine, _)| mine == key.as_bytes())
    }

    /// Returns the answer.
    ///
    /// # Panics
    ///
    /// Panics when the bytes do not decode. They were packed by this kernel
    /// or checked when they were read, so that is a bug in the kernel.
    pub(crate) fn ack(&self) -> ActionAck {
        match Remembered::open(&self.0) {
            Ok(ack) => ack,
            Err(error) => panic!("{error}"),
        }
    }

    /// Returns an estimate of the heap bytes: the bytes, in blocks of 16.
    fn memory_estimate(&self) -> usize {
        self.0.len().next_multiple_of(16)
    }
}

impl Serialize for Remembered {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Remembered {
    /// Reads the block. It also reads the form of a snapshot from before
    /// the kernel packed it: the pair of the key and the answer.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Remembered, D::Error> {
        struct RememberedVisitor;

        impl<'de> Visitor<'de> for RememberedVisitor {
            type Value = Remembered;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the bytes of a remembered request, or a key and its answer")
            }

            fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<Remembered, E> {
                Remembered::open(bytes).map_err(E::custom)?;
                Ok(Remembered(bytes.into()))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Remembered, A::Error> {
                let (key, mut ack): (String, ActionAck) =
                    Deserialize::deserialize(SeqAccessDeserializer::new(seq))?;
                ack.request_id = Some(key);
                Remembered::new(&ack).ok_or_else(|| A::Error::custom("the key is too long"))
            }
        }

        deserializer.deserialize_any(RememberedVisitor)
    }
}

/// The arguments of an action in compact form: the MessagePack bytes of the
/// JSON value, and no bytes for `null`.
///
/// A resident keeps the arguments of its running action, of its planned
/// actions and of its routines for as long as they last. A tree of JSON
/// values costs several hundred bytes for each object (see
/// [`PackedBody`]); the bytes of `{"ticks": 5}` are eight. The kernel opens
/// them when the action starts or ends.
///
/// Every `PackedArgs` holds bytes that decode: it is made from a value, or
/// it is checked when it is read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PackedArgs(Box<[u8]>);

/// What a [`PackedArgs`] is, for error messages.
const ARGS: &str = "the arguments of an action";

impl PackedArgs {
    /// Packs the arguments of an action.
    pub(crate) fn new(args: &Value) -> PackedArgs {
        if args.is_null() {
            PackedArgs::default()
        } else {
            PackedArgs(codec::encode(ARGS, args).into_boxed_slice())
        }
    }

    /// Returns true for `null`: an action without arguments.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Returns the arguments.
    ///
    /// # Panics
    ///
    /// Panics when the bytes do not decode. They were packed by this kernel
    /// or checked when they were read, so that is a bug in the kernel.
    pub(crate) fn open(&self) -> Value {
        if self.0.is_empty() {
            return Value::Null;
        }
        match codec::decode(ARGS, &self.0) {
            Ok(args) => args,
            Err(error) => panic!("{error}"),
        }
    }

    /// Returns an estimate of the heap bytes: the bytes, in blocks of 16.
    fn memory_estimate(&self) -> usize {
        self.0.len().next_multiple_of(16)
    }
}

impl Serialize for PackedArgs {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for PackedArgs {
    /// Reads the packed bytes. It also reads the form of a snapshot from
    /// before the kernel packed the arguments: the JSON value itself. A
    /// JSON value is never written as bytes, so the two forms cannot be
    /// taken for each other.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PackedArgs, D::Error> {
        struct ArgsVisitor;

        impl<'de> Visitor<'de> for ArgsVisitor {
            type Value = PackedArgs;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the bytes of the arguments of an action, or the arguments as a value")
            }

            fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<PackedArgs, E> {
                if !bytes.is_empty() {
                    codec::decode::<Value>(ARGS, bytes).map_err(E::custom)?;
                }
                Ok(PackedArgs(bytes.into()))
            }

            fn visit_unit<E: Error>(self) -> Result<PackedArgs, E> {
                Ok(PackedArgs::default())
            }

            fn visit_none<E: Error>(self) -> Result<PackedArgs, E> {
                Ok(PackedArgs::default())
            }

            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<PackedArgs, D::Error> {
                PackedArgs::deserialize(d)
            }

            fn visit_bool<E: Error>(self, v: bool) -> Result<PackedArgs, E> {
                Ok(PackedArgs::new(&Value::from(v)))
            }

            fn visit_i64<E: Error>(self, v: i64) -> Result<PackedArgs, E> {
                Ok(PackedArgs::new(&Value::from(v)))
            }

            fn visit_u64<E: Error>(self, v: u64) -> Result<PackedArgs, E> {
                Ok(PackedArgs::new(&Value::from(v)))
            }

            fn visit_f64<E: Error>(self, v: f64) -> Result<PackedArgs, E> {
                Ok(PackedArgs::new(&Value::from(v)))
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<PackedArgs, E> {
                Ok(PackedArgs::new(&Value::from(v)))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<PackedArgs, A::Error> {
                let value = Value::deserialize(SeqAccessDeserializer::new(seq))?;
                Ok(PackedArgs::new(&value))
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<PackedArgs, A::Error> {
                let value = Value::deserialize(MapAccessDeserializer::new(map))?;
                Ok(PackedArgs::new(&value))
            }
        }

        deserializer.deserialize_any(ArgsVisitor)
    }
}

/// One routine with its compiled condition and its retry state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "SavedRoutineSlot")]
pub(crate) struct RoutineSlot {
    /// The routine as the owner wrote it, without the arguments of its
    /// action: they are in `args`.
    pub(crate) routine: Routine,
    /// The arguments of `routine.then`, packed.
    #[serde(default, skip_serializing_if = "PackedArgs::is_empty")]
    pub(crate) args: PackedArgs,
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

/// A [`RoutineSlot`] as it is read. A snapshot from before the kernel
/// packed the arguments has them inside `routine` and has no `args`.
#[derive(Deserialize)]
struct SavedRoutineSlot {
    routine: Routine,
    #[serde(default)]
    args: PackedArgs,
    cond: Cond,
    verb: VerbId,
    #[serde(default)]
    retry_at: u64,
    #[serde(default)]
    failures: u8,
    #[serde(default)]
    runs: u64,
}

impl From<SavedRoutineSlot> for RoutineSlot {
    fn from(saved: SavedRoutineSlot) -> RoutineSlot {
        let SavedRoutineSlot { routine, args, cond, verb, retry_at, failures, runs } = saved;
        let mut slot = RoutineSlot::new(routine, cond, verb);
        if slot.args.is_empty() {
            slot.args = args;
        }
        RoutineSlot { retry_at, failures, runs, ..slot }
    }
}

impl RoutineSlot {
    /// Makes the slot of a routine that never ran. The arguments of its
    /// action move out of the routine and are packed.
    pub(crate) fn new(mut routine: Routine, cond: Cond, verb: VerbId) -> RoutineSlot {
        let args = PackedArgs::new(&std::mem::take(&mut routine.then.args));
        RoutineSlot { routine, args, cond, verb, retry_at: 0, failures: 0, runs: 0 }
    }

    /// Returns the action of the routine as the owner wrote it, with its
    /// arguments.
    pub(crate) fn then(&self) -> unsup_protocol::ActionRequest {
        let mut then = self.routine.then.clone();
        then.args = self.args.open();
        then
    }

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

/// The content of one inbox entry, as the kernel makes it and shows it. An
/// inbox keeps it in compact form: see [`PackedBody`].
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

/// The content of one inbox entry in compact form: the MessagePack bytes of
/// a [`NoticeBody`]. Residents that receive the same notice share one body.
///
/// A notice carries a small JSON document. As a tree of JSON values it
/// costs several hundred bytes for each object, however small: every
/// object is a map node with room for eleven entries. A full inbox of such
/// trees was nine tenths of the memory of a resident. The bytes of the same
/// document are about as long as its text. The kernel opens them only when
/// an observation shows the entry.
///
/// Every `PackedBody` holds bytes that decode: it is made from a
/// [`NoticeBody`], or it is checked when a snapshot or a migrating resident
/// is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PackedBody(Arc<[u8]>);

/// What a [`PackedBody`] is, for error messages.
const BODY: &str = "an inbox entry";

impl PackedBody {
    /// Packs the content of an inbox entry.
    pub(crate) fn new(body: &NoticeBody) -> PackedBody {
        PackedBody(Arc::from(codec::encode(BODY, body)))
    }

    /// Takes bytes that another kernel packed, when they decode.
    fn checked(bytes: &[u8]) -> Result<PackedBody, codec::DecodeError> {
        codec::decode::<NoticeBody>(BODY, bytes)?;
        Ok(PackedBody(Arc::from(bytes)))
    }

    /// Returns the content.
    ///
    /// # Panics
    ///
    /// Panics when the bytes do not decode. Every body was packed by this
    /// kernel or checked when it was read, so that is a bug in the kernel.
    pub(crate) fn open(&self) -> NoticeBody {
        match codec::decode(BODY, &self.0) {
            Ok(body) => body,
            Err(error) => panic!("{error}"),
        }
    }

    /// Returns an estimate of the heap bytes of the body: the two counters
    /// of the shared pointer and the bytes, in blocks of 16.
    fn memory_estimate(&self) -> usize {
        (16 + self.0.len()).next_multiple_of(16)
    }
}

impl Serialize for PackedBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for PackedBody {
    /// Reads the packed bytes. It also reads the form of a snapshot from
    /// before the kernel packed the entries: the [`NoticeBody`] itself.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<PackedBody, D::Error> {
        struct BodyVisitor;

        impl<'de> Visitor<'de> for BodyVisitor {
            type Value = PackedBody;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("the bytes of an inbox entry, or an inbox entry of an older snapshot")
            }

            fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<PackedBody, E> {
                PackedBody::checked(bytes).map_err(E::custom)
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<PackedBody, A::Error> {
                let body = NoticeBody::deserialize(MapAccessDeserializer::new(map))?;
                Ok(PackedBody::new(&body))
            }
        }

        deserializer.deserialize_any(BodyVisitor)
    }
}

/// One entry of an inbox.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct InboxEntry {
    #[serde(rename = "s")]
    pub(crate) seq: u64,
    #[serde(rename = "b")]
    pub(crate) body: PackedBody,
}

/// Entries that the first buffer of an inbox has room for. An inbox that
/// gets more has the buffer of a full inbox from then on.
const FIRST_ROOM: usize = 4;

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
    /// Adds an entry. The inbox then holds at most `max` entries, and at
    /// least the new one: the oldest entries leave first.
    ///
    /// The buffer of the entries has two sizes: room for a few entries, and
    /// then room for `max`. The inbox of a resident that acts is full soon.
    /// A buffer that grows to that size step by step leaves each smaller
    /// buffer behind as free memory of a size that little else can use.
    pub(crate) fn push(&mut self, body: PackedBody, max: usize) {
        let max = max.max(1);
        self.count += 1;
        // The oldest entries leave before the new one comes, so that the
        // inbox never holds more than `max`.
        while self.items.len() >= max {
            self.items.pop_front();
        }
        let capacity = self.items.capacity();
        if self.items.len() == capacity && capacity < max {
            let room = if capacity == 0 { max.min(FIRST_ROOM) } else { max };
            self.items.reserve_exact(room - self.items.len());
        }
        self.items.push_back(InboxEntry { seq: self.count, body });
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
        let bodies: usize = self.items.iter().map(|entry| entry.body.memory_estimate()).sum();
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
        match self.body.open() {
            NoticeBody::Notice { tick, kind, from, data } => Shown::Notice(Notice {
                seq: self.seq,
                tick: Tick(tick),
                kind: kind.into_string(),
                from,
                data,
            }),
            NoticeBody::Report { tick, action_id, request_id, verb, result } => {
                let (ok, value, error) = match result {
                    Ok(value) => (true, value, None),
                    Err(error) => (false, Value::Null, Some(error)),
                };
                Shown::Report(ActionReport {
                    seq: self.seq,
                    action_id,
                    request_id,
                    verb: verb_name(verb).to_string(),
                    tick: Tick(tick),
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

    fn note(tick: u64) -> PackedBody {
        PackedBody::new(&NoticeBody::Notice {
            tick,
            kind: "test.note".into(),
            from: None,
            data: Value::Null,
        })
    }

    /// A speech notice and a failed report: what an inbox holds.
    fn speech_and_report() -> [NoticeBody; 2] {
        [
            NoticeBody::Notice {
                tick: 5,
                kind: "society.speech".into(),
                from: Some(ResidentId(3)),
                data: serde_json::json!({ "text": { "untrusted": true, "from": 3, "text": "hi" } }),
            },
            NoticeBody::Report {
                tick: 6,
                action_id: 44,
                request_id: Some("a".into()),
                verb: VerbId(2),
                result: Err(ActionError::not_found("No such shop.")
                    .with_details(serde_json::json!({ "shop": 9 }))),
            },
        ]
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
        for body in speech_and_report() {
            inbox.push(PackedBody::new(&body), 8);
        }
        let shown: Vec<Shown> = inbox.since(0).map(|e| e.show(|_| "buy")).collect();
        match &shown[0] {
            Shown::Notice(n) => {
                assert_eq!(
                    (n.seq, n.tick, n.kind.as_str(), n.from),
                    (1, Tick(5), "society.speech", Some(ResidentId(3)))
                );
                assert_eq!(
                    n.data,
                    serde_json::json!({ "text": { "untrusted": true, "from": 3, "text": "hi" } })
                );
            }
            Shown::Report(_) => panic!("the first entry is a notice"),
        }
        match &shown[1] {
            Shown::Report(r) => {
                assert_eq!(
                    (r.seq, r.action_id, r.verb.as_str(), r.ok, r.tick, r.request_id.as_deref()),
                    (2, 44, "buy", false, Tick(6), Some("a"))
                );
                assert_eq!(r.result, Value::Null);
                assert_eq!(
                    r.error,
                    Some(
                        ActionError::not_found("No such shop.")
                            .with_details(serde_json::json!({ "shop": 9 }))
                    )
                );
            }
            Shown::Notice(_) => panic!("the second entry is a report"),
        }
        let bytes = crate::codec::encode("inbox", &inbox);
        let back: Inbox = crate::codec::decode("inbox", &bytes).unwrap();
        assert_eq!(back, inbox);
    }

    #[test]
    fn a_packed_body_opens_to_what_was_packed_and_is_small() {
        for body in speech_and_report() {
            let packed = PackedBody::new(&body);
            assert_eq!(packed.open(), body);
            // The bytes are near the length of the text of the entry. The
            // same entry as a tree of JSON values takes 1,000 bytes and
            // more.
            assert!(packed.0.len() < 100, "{} bytes", packed.0.len());
            assert_eq!(packed.memory_estimate() % 16, 0);
            assert!(packed.memory_estimate() >= packed.0.len() + 16);
            // A copy shares the bytes.
            let copy = packed.clone();
            assert!(Arc::ptr_eq(&packed.0, &copy.0));
        }
        // A report with a result.
        let done = NoticeBody::Report {
            tick: 9,
            action_id: 1 << 40,
            request_id: None,
            verb: VerbId(7),
            result: Ok(serde_json::json!({ "pay": 50 })),
        };
        let packed = PackedBody::new(&done);
        assert_eq!(packed.open(), done);
        assert!(packed.0.len() < 40, "{} bytes", packed.0.len());
    }

    /// An inbox as a snapshot held it before the kernel packed the
    /// entries: each entry has its content as a value, not as bytes.
    #[derive(Serialize)]
    struct OldInbox {
        n: u64,
        i: Vec<OldEntry>,
    }

    #[derive(Serialize)]
    struct OldEntry {
        s: u64,
        b: NoticeBody,
    }

    #[test]
    fn an_inbox_of_an_older_snapshot_is_read_and_packed() {
        let [speech, report] = speech_and_report();
        let old = OldInbox {
            n: 9,
            i: vec![OldEntry { s: 8, b: speech.clone() }, OldEntry { s: 9, b: report.clone() }],
        };
        let old_bytes = crate::codec::encode("inbox", &old);
        let read: Inbox = crate::codec::decode("inbox", &old_bytes).unwrap();
        // The same inbox, made by this kernel.
        let mut made = Inbox { count: 7, items: VecDeque::new() };
        made.push(PackedBody::new(&speech), 8);
        made.push(PackedBody::new(&report), 8);
        assert_eq!(read, made);
        assert_eq!(read.items.iter().map(|e| e.body.open()).collect::<Vec<_>>(), [speech, report]);
        // It is written in the new form, which reads back to the same.
        let new_bytes = crate::codec::encode("inbox", &read);
        assert_ne!(new_bytes, old_bytes);
        assert_eq!(new_bytes, crate::codec::encode("inbox", &made));
        assert_eq!(crate::codec::decode::<Inbox>("inbox", &new_bytes).unwrap(), made);
    }

    #[test]
    fn bytes_that_are_not_an_inbox_entry_are_refused_when_they_are_read() {
        /// An inbox entry with any bytes as its content.
        #[derive(Serialize)]
        struct RawEntry<'a> {
            s: u64,
            #[serde(with = "crate::codec::bytes")]
            b: &'a [u8],
        }
        let good = PackedBody::new(&speech_and_report()[0]);
        let entry = crate::codec::encode("entry", &RawEntry { s: 1, b: &good.0 });
        let read: InboxEntry = crate::codec::decode("entry", &entry).unwrap();
        assert_eq!((read.seq, &read.body), (1, &good));
        // Cut bytes, bytes of another type, and no bytes at all.
        let cut = &good.0[..good.0.len() - 3];
        let other = crate::codec::encode("number", &17u32);
        for bad in [cut, other.as_slice(), &[]] {
            let entry = crate::codec::encode("entry", &RawEntry { s: 1, b: bad });
            let error = crate::codec::decode::<InboxEntry>("entry", &entry).unwrap_err();
            assert!(error.message.contains("cannot decode an inbox entry"), "{error}");
        }
        // A content that is neither bytes nor an entry.
        let number = crate::codec::encode("entry", &serde_json::json!({ "s": 1, "b": 5 }));
        assert!(crate::codec::decode::<InboxEntry>("entry", &number).is_err());
    }

    #[test]
    fn the_buffer_of_an_inbox_has_two_sizes() {
        // A few entries have a small buffer.
        let mut inbox = Inbox::default();
        for tick in 0..4 {
            inbox.push(note(tick), 32);
            assert!((4..8).contains(&inbox.items.capacity()), "{} slots", inbox.items.capacity());
        }
        // One more: the buffer has room for the limit, and it is the same
        // buffer when the inbox is full and after every entry was replaced.
        inbox.push(note(4), 32);
        let capacity = inbox.items.capacity();
        assert!((32..40).contains(&capacity), "{capacity} slots for a limit of 32");
        for tick in 5..1_000 {
            inbox.push(note(tick), 32);
            assert_eq!(inbox.items.capacity(), capacity, "at entry {tick}");
        }
        assert_eq!(inbox.items.len(), 32);
        assert_eq!(
            inbox.since(0).map(|e| e.seq).collect::<Vec<_>>(),
            (969..=1_000).collect::<Vec<u64>>()
        );
        assert!(inbox.memory_estimate() >= 32 * size_of::<InboxEntry>() + 32 * 32);
        // A higher limit makes a larger buffer, one time.
        inbox.push(note(1_000), 64);
        let larger = inbox.items.capacity();
        assert!((64..80).contains(&larger), "{larger} slots for a limit of 64");
        for tick in 0..100 {
            inbox.push(note(tick), 64);
        }
        assert_eq!((inbox.items.len(), inbox.items.capacity()), (64, larger));
        // A limit below the first size makes a buffer for the limit.
        let mut small = Inbox::default();
        for tick in 0..10 {
            small.push(note(tick), 2);
        }
        assert_eq!(small.items.len(), 2);
        assert!(small.items.capacity() < 4, "{} slots", small.items.capacity());
        // An inbox that was read from a snapshot has a buffer for what it
        // holds. It gets the room of the limit at its next entry.
        let bytes = crate::codec::encode("inbox", &inbox);
        let mut read: Inbox = crate::codec::decode("inbox", &bytes).unwrap();
        read.push(note(7), 64);
        assert_eq!(read.items.len(), 64);
        assert!((64..80).contains(&read.items.capacity()));
    }

    #[test]
    fn packed_arguments_open_to_the_value_and_null_takes_no_bytes() {
        let values = [
            Value::Null,
            serde_json::json!({ "ticks": 5 }),
            serde_json::json!({ "to": [3, 17], "text": "meet me at noon", "deep": { "a": [1, { "b": null }] } }),
            serde_json::json!([1, "two", false]),
            serde_json::json!("text"),
            serde_json::json!(-7),
            serde_json::json!(u64::MAX),
            serde_json::json!(true),
        ];
        for value in &values {
            let packed = PackedArgs::new(value);
            assert_eq!(&packed.open(), value);
            assert_eq!(packed.is_empty(), value.is_null());
            // The packed form is written as bytes and read back.
            let bytes = crate::codec::encode("args", &packed);
            assert_eq!(crate::codec::decode::<PackedArgs>("args", &bytes).unwrap(), packed);
            // The form of an older snapshot, the value itself, reads as the
            // same arguments.
            let old = crate::codec::encode("args", value);
            assert_eq!(
                crate::codec::decode::<PackedArgs>("args", &old).unwrap(),
                packed,
                "{value}"
            );
        }
        assert_eq!(PackedArgs::new(&Value::Null), PackedArgs::default());
        assert_eq!(PackedArgs::new(&values[1]).0.len(), 8, "`{{\"ticks\": 5}}` is 8 bytes");
        assert_eq!(PackedArgs::new(&values[1]).memory_estimate(), 16);
        assert_eq!(PackedArgs::default().memory_estimate(), 0);
        // Bytes that are not a value are refused when they are read.
        let cut = &PackedArgs::new(&values[2]).0[..10];
        let bad = crate::codec::encode("args", &RawBytes(cut));
        let error = crate::codec::decode::<PackedArgs>("args", &bad).unwrap_err();
        assert!(error.message.contains("cannot decode the arguments of an action"), "{error}");
    }

    /// Bytes that are written as a MessagePack binary.
    #[derive(Serialize)]
    struct RawBytes<'a>(#[serde(with = "crate::codec::bytes")] &'a [u8]);

    #[test]
    fn a_remembered_request_gives_back_its_answer_and_reads_the_older_form() {
        use unsup_protocol::ActionStatus;
        let key = "0b0e6a2c-7f1d-4a55-9c3e-5d2f8a1b4c6d";
        let acks = [
            ActionAck {
                request_id: Some(key.to_string()),
                action_id: 1 << 41,
                verb: "greet".into(),
                status: ActionStatus::Done { result: serde_json::json!({ "greeted": 7 }) },
            },
            ActionAck {
                request_id: Some("k".into()),
                action_id: 9,
                verb: "work".into(),
                status: ActionStatus::Started { ends_at: Tick(40), ticks: 5 },
            },
            ActionAck {
                request_id: Some(String::new()),
                action_id: 10,
                verb: "ping".into(),
                status: ActionStatus::Pending,
            },
        ];
        for ack in &acks {
            let remembered = Remembered::new(ack).expect("the request has a key");
            assert_eq!(&remembered.ack(), ack);
            let own = ack.request_id.as_deref().unwrap_or_default();
            assert!(remembered.has_key(own));
            assert!(!remembered.has_key("another"));
            assert!(!remembered.has_key(&format!("{own}x")));
            // It is written as bytes and reads back.
            let bytes = crate::codec::encode("remembered", &remembered);
            assert_eq!(
                crate::codec::decode::<Remembered>("remembered", &bytes).unwrap(),
                remembered
            );
            // An older snapshot has the key and the answer as a pair.
            let old = crate::codec::encode("remembered", &(own, ack));
            assert_eq!(crate::codec::decode::<Remembered>("remembered", &old).unwrap(), remembered);
        }
        // The block of a request with a key of 36 bytes and a small result
        // is far below the 1,100 bytes of the answer as values.
        let block = Remembered::new(&acks[0]).expect("the request has a key");
        assert!(block.0.len() < 100, "{} bytes", block.0.len());
        assert_eq!(block.memory_estimate() % 16, 0);
        // No key, no memory.
        assert_eq!(Remembered::new(&ActionAck { request_id: None, ..acks[0].clone() }), None);
        // A block that is cut, in the key or in the answer, is refused.
        for cut in [0, 1, 20, block.0.len() - 1] {
            let bad = crate::codec::encode("remembered", &RawBytes(&block.0[..cut]));
            let error = crate::codec::decode::<Remembered>("remembered", &bad).unwrap_err();
            assert!(error.message.contains("a remembered request"), "{error}");
        }
        // An agenda with remembered requests survives the wire.
        let agenda = Agenda {
            recent: acks.iter().filter_map(Remembered::new).collect(),
            ..Agenda::default()
        };
        assert_eq!(agenda.recent.len(), 3);
        let bytes = crate::codec::encode("agenda", &agenda);
        assert_eq!(crate::codec::decode::<Agenda>("agenda", &bytes).unwrap(), agenda);
    }

    #[test]
    fn an_agenda_of_an_older_snapshot_is_read_and_packed() {
        /// The running action, a planned action and a routine as a snapshot
        /// held them before the kernel packed the arguments.
        #[derive(Serialize)]
        struct OldAgenda {
            c: OldRunning,
            q: Vec<OldQueued>,
            r: Vec<OldSlot>,
        }
        #[derive(Serialize)]
        struct OldRunning {
            a: u64,
            v: VerbId,
            g: Value,
            s: u64,
            e: u64,
        }
        #[derive(Serialize)]
        struct OldQueued {
            a: u64,
            v: VerbId,
            r: String,
            #[serde(skip_serializing_if = "Value::is_null")]
            g: Value,
        }
        #[derive(Serialize)]
        struct OldSlot {
            routine: Routine,
            cond: Cond,
            verb: VerbId,
            runs: u64,
        }
        let work = serde_json::json!({ "ticks": 5 });
        let nap = serde_json::json!({ "ticks": 4, "where": "home" });
        let routine = Routine {
            name: "rest".into(),
            when: unsup_protocol::Condition::Always,
            then: unsup_protocol::ActionRequest::new("nap", nap.clone()),
        };
        let old = OldAgenda {
            c: OldRunning { a: 11, v: VerbId(3), g: work.clone(), s: 20, e: 25 },
            q: vec![
                OldQueued { a: 12, v: VerbId(4), r: "key".into(), g: nap.clone() },
                OldQueued { a: 13, v: VerbId(5), r: "none".into(), g: Value::Null },
            ],
            r: vec![OldSlot {
                routine: routine.clone(),
                cond: Cond::Always,
                verb: VerbId(4),
                runs: 6,
            }],
        };
        let read: Agenda =
            crate::codec::decode("agenda", &crate::codec::encode("agenda", &old)).unwrap();

        let running = read.current.as_ref().unwrap();
        assert_eq!((running.action_id, running.verb, running.ends), (11, VerbId(3), 25));
        assert_eq!(running.args.open(), work);
        assert_eq!(read.queue.len(), 2);
        assert_eq!((read.queue[0].action_id, read.queue[0].args.open()), (12, nap.clone()));
        assert_eq!(read.queue[0].request_id.as_deref(), Some("key"));
        assert!(read.queue[1].args.is_empty());
        // The arguments of the routine moved out of the routine. The owner
        // still sees the action as it wrote it.
        let slot = &read.routines[0];
        assert_eq!((slot.verb, slot.runs, slot.failures), (VerbId(4), 6, 0));
        assert_eq!(slot.routine.then.args, Value::Null);
        assert_eq!(slot.args.open(), nap);
        assert_eq!(slot.then(), routine.then);
        // The same slot, made by this kernel.
        let mut made = RoutineSlot::new(routine, Cond::Always, VerbId(4));
        made.runs = 6;
        assert_eq!(*slot, made);
        // The agenda is written in the new form and reads back to the same.
        let bytes = crate::codec::encode("agenda", &read);
        assert_eq!(crate::codec::decode::<Agenda>("agenda", &bytes).unwrap(), read);
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
                args: PackedArgs::default(),
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
        let routine = Routine {
            name: "r".into(),
            when: unsup_protocol::Condition::Always,
            then: unsup_protocol::ActionRequest::new("wait", Value::Null),
        };
        let mut slot = RoutineSlot::new(routine, Cond::Always, VerbId(0));
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
