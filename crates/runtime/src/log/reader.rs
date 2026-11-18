//! Reads the valid records of a log in order.

use unsup_kernel::Input;

use super::LogError;
use super::index::LogIndex;
use super::record::{Frame, decode, frame_at, record_follows};
use super::segment::SEGMENT_HEADER;
use crate::storage::Storage;

/// The end of the last segment could not be read. This is what a crash in
/// the middle of a write leaves behind. The reader ignores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TornTail {
    /// Name of the segment.
    pub segment: String,
    /// Sequence number that the record would have.
    pub seq: u64,
    /// Position of the record in the segment file, in bytes.
    pub offset: u64,
    /// Bytes from that position to the end of the file. They are ignored.
    pub ignored_bytes: u64,
    /// Why the record cannot be read.
    pub reason: String,
}

/// Where a read of the log stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LogEnd {
    /// Sequence number of the first input that was not visited.
    pub next_seq: u64,
    /// The torn end of the last segment, when there is one.
    pub torn: Option<TornTail>,
}

/// How a segment ended.
enum SegmentEnd {
    /// A later segment starts here.
    Limit,
    /// The file ends after a whole record.
    Clean,
    /// The file ends with bytes that are not a record.
    Torn(&'static str),
}

/// Gives every valid record with a sequence number from `from` up to, but
/// not including, `until` to `visit`, in order. `visit` returns false to
/// stop before the record that it was given; that record then counts as not
/// visited. Nothing at or after `until` is read, so damage there does not
/// matter.
///
/// A log that ends before `from` gives no record. That is not an error: a
/// snapshot can be newer than the end of the log.
pub(crate) fn read_from(
    storage: &dyn Storage,
    index: &LogIndex,
    from: u64,
    until: u64,
    visit: &mut dyn FnMut(u64, Input) -> bool,
) -> Result<LogEnd, LogError> {
    let mut expected = from;
    let mut torn = None;
    for slot in &index.segments {
        if expected >= until {
            break;
        }
        if slot.limit <= expected {
            continue;
        }
        if slot.id.first_seq > expected {
            return Err(LogError::Gap { from: expected, to: slot.id.first_seq });
        }
        let bytes = storage.read(&slot.name)?;
        slot.id
            .check_header(&bytes)
            .map_err(|message| LogError::Segment { name: slot.name.clone(), message })?;

        let mut offset = SEGMENT_HEADER;
        let mut seq = slot.id.first_seq;
        let end = loop {
            if seq >= slot.limit || seq >= until {
                break SegmentEnd::Limit;
            }
            match frame_at(&bytes, offset) {
                Frame::End => break SegmentEnd::Clean,
                Frame::Torn(reason) => break SegmentEnd::Torn(reason),
                Frame::Record { payload, next } => {
                    if seq >= expected {
                        let corrupt = |message: String| LogError::Corrupt {
                            segment: slot.name.clone(),
                            seq,
                            offset: offset as u64,
                            message,
                        };
                        let record = decode(payload).map_err(corrupt)?;
                        if record.seq != seq {
                            return Err(corrupt(format!(
                                "the record says that it is number {}",
                                record.seq
                            )));
                        }
                        if !visit(seq, record.input) {
                            return Ok(LogEnd { next_seq: seq, torn: None });
                        }
                        expected = seq + 1;
                    }
                    seq += 1;
                    offset = next;
                }
            }
        };
        let last = slot.limit == u64::MAX;
        match end {
            SegmentEnd::Limit => {}
            // The end of the last segment is the end of the log.
            SegmentEnd::Clean if last => {}
            // A crash cuts the log off at its end. Damage with a valid
            // record behind it is something else: inputs that were applied
            // would be lost, so the reader does not pass over it.
            SegmentEnd::Torn(reason) if last && !record_follows(&bytes, offset) => {
                torn = Some(TornTail {
                    segment: slot.name.clone(),
                    seq,
                    offset: offset as u64,
                    ignored_bytes: (bytes.len() - offset.min(bytes.len())) as u64,
                    reason: reason.to_string(),
                });
            }
            // A segment in the middle must reach the start of the next one.
            SegmentEnd::Clean => return Err(LogError::Gap { from: seq, to: slot.limit }),
            SegmentEnd::Torn(reason) => {
                return Err(LogError::Corrupt {
                    segment: slot.name.clone(),
                    seq,
                    offset: offset as u64,
                    message: format!("{reason}, and the log goes on after it"),
                });
            }
        }
    }
    Ok(LogEnd { next_seq: expected, torn })
}
