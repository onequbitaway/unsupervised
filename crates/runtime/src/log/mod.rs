//! The input log: every input of the world, in the order in which it was
//! applied.
//!
//! # Files
//!
//! The log is a row of segment files in the directory `log`. A segment is
//! named `<index>-<first sequence number>.seg`. It starts with a header of
//! 32 bytes and then holds records:
//!
//! ```text
//! header  = "UNSUPLOG" | version u32 | index u64 | first_seq u64 | check u32
//! record  = length u32 | check u32 | payload
//! payload = MessagePack with field names of { seq, input }
//! ```
//!
//! All numbers are little-endian. `check` is the first four bytes of the
//! blake3 hash of the bytes that it protects. The header is written
//! atomically, so a segment file either does not exist or has a header.
//!
//! # One rule for every accident
//!
//! Every input has a sequence number, from 0. A segment declares the
//! sequence number of its first record. **A new segment makes every record
//! with the same or a higher sequence number in an older segment void.**
//!
//! The log is never changed in place. The writer starts a new segment
//!
//! - when a segment is full: the new one starts where the old one ends;
//! - after a crash: the new one starts at the first record that could not be
//!   read, so the torn end of the old segment is void;
//! - after a write that failed: the inputs of that write were not applied,
//!   and the new segment makes whatever reached the disk void;
//! - after a rollback: the new one starts at the sequence number of the
//!   snapshot, so everything after the snapshot is void.
//!
//! A reader that finds a record that it cannot read accepts that only when
//! the record is void or is the end of the last segment. Everything else is
//! damage, and the reader reports it.

mod index;
mod reader;
mod record;
mod segment;
mod syncer;
mod writer;

use std::fmt;

pub(crate) use index::LogIndex;
pub use reader::TornTail;
pub(crate) use reader::read_from;
pub(crate) use segment::SegmentId;
pub(crate) use syncer::{LogShared, Syncer};
pub(crate) use writer::LogWriter;

use crate::storage::StorageError;

/// The input log cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogError {
    /// The storage failed.
    Storage(StorageError),
    /// A segment file is not what its name says.
    Segment {
        /// Name of the segment.
        name: String,
        /// What is wrong.
        message: String,
    },
    /// Records are missing: the log has no record with a sequence number
    /// from `from` up to, but not including, `to`.
    Gap {
        /// The first missing sequence number.
        from: u64,
        /// The first sequence number after the gap.
        to: u64,
    },
    /// A record that is not at the end of the log cannot be read.
    Corrupt {
        /// Name of the segment.
        segment: String,
        /// Sequence number of the record.
        seq: u64,
        /// Position of the record in the segment file, in bytes.
        offset: u64,
        /// What is wrong.
        message: String,
    },
}

impl fmt::Display for LogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LogError::Storage(e) => write!(f, "cannot read the input log: {e}"),
            LogError::Segment { name, message } => {
                write!(f, "the log segment `{name}` is not valid: {message}")
            }
            LogError::Gap { from, to } => write!(
                f,
                "the input log has no records from sequence number {from} to {}",
                to.saturating_sub(1)
            ),
            LogError::Corrupt { segment, seq, offset, message } => write!(
                f,
                "record {seq} at byte {offset} of the log segment `{segment}` is damaged: {message}"
            ),
        }
    }
}

impl std::error::Error for LogError {}

impl From<StorageError> for LogError {
    fn from(error: StorageError) -> LogError {
        LogError::Storage(error)
    }
}
