//! Snapshot files: the whole state of the world at one moment.
//!
//! A snapshot is one blob in the directory `snapshots`, named
//! `<sequence number>-<frame>.snap`:
//!
//! ```text
//! file   = header | payload
//! header = "UNSUPSNP" | version u32 | frame u64 | seq u64 | segment u64
//!        | state_hash [32] | payload_len u64 | payload_hash [32] | check u32
//! ```
//!
//! All numbers are little-endian. `payload` is what `World::snapshot`
//! returned. `seq` is the number of inputs that the world had applied:
//! a recovery restores the payload and replays the log from that sequence
//! number. `segment` is the index of the newest log segment at that moment;
//! it tells whether a later rollback dropped inputs that the snapshot holds.
//! `state_hash` is `World::state_hash` at that moment. `payload_hash` is the
//! blake3 hash of the payload and `check` covers the header.

mod format;
mod store;
mod writer;

pub use format::SnapshotMeta;
pub(crate) use store::{SnapshotEntry, head, list, load, write};
pub(crate) use writer::{SnapshotJob, SnapshotWriter};
