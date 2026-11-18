//! The frame of one log record: a length, a checksum and a MessagePack
//! payload.

use serde::{Deserialize, Serialize};
use unsup_kernel::Input;

/// Bytes before the payload: the length and the checksum.
pub(crate) const FRAME_HEADER: usize = 8;

/// One record, as it is written.
#[derive(Serialize)]
struct RecordRef<'a> {
    seq: u64,
    input: &'a Input,
}

/// One record, as it is read.
#[derive(Deserialize)]
pub(crate) struct Record {
    /// Sequence number of the input, from 0.
    pub seq: u64,
    /// The input.
    pub input: Input,
}

/// Returns the checksum of a payload of the given length.
fn checksum(len: u32, payload: &[u8]) -> u32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&len.to_le_bytes());
    hasher.update(payload);
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}

/// Appends one framed record to `out`. When it fails, `out` is unchanged.
pub(crate) fn encode(seq: u64, input: &Input, max: u32, out: &mut Vec<u8>) -> Result<(), String> {
    let start = out.len();
    out.extend_from_slice(&[0; FRAME_HEADER]);
    if let Err(e) = rmp_serde::encode::write_named(out, &RecordRef { seq, input }) {
        out.truncate(start);
        return Err(format!("the input cannot be written as MessagePack: {e}"));
    }
    let payload = out.len() - start - FRAME_HEADER;
    let len = match u32::try_from(payload) {
        Ok(len) if len <= max => len,
        _ => {
            out.truncate(start);
            return Err(format!(
                "the input takes {payload} bytes, the limit for one record is {max} bytes"
            ));
        }
    };
    let check = checksum(len, &out[start + FRAME_HEADER..]);
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
    out[start + 4..start + FRAME_HEADER].copy_from_slice(&check.to_le_bytes());
    Ok(())
}

/// What is at a position of a segment.
pub(crate) enum Frame<'a> {
    /// A record whose checksum is right.
    Record {
        /// The MessagePack payload.
        payload: &'a [u8],
        /// Position of the next frame.
        next: usize,
    },
    /// The segment ends exactly here.
    End,
    /// The bytes here are not a whole record.
    Torn(&'static str),
}

/// Reads the frame at `offset`.
pub(crate) fn frame_at(bytes: &[u8], offset: usize) -> Frame<'_> {
    let rest = &bytes[offset.min(bytes.len())..];
    if rest.is_empty() {
        return Frame::End;
    }
    let Some((header, body)) = rest.split_first_chunk::<FRAME_HEADER>() else {
        return Frame::Torn("the record header is cut off");
    };
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]);
    let check = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    let Some(payload) = body.get(..len as usize) else {
        return Frame::Torn("the record is cut off");
    };
    if checksum(len, payload) != check {
        return Frame::Torn("the checksum of the record is wrong");
    }
    Frame::Record { payload, next: offset + FRAME_HEADER + payload.len() }
}

/// Returns true when the damaged record at `offset` has a readable length
/// and a whole, valid record comes right after it. Then the damage is in
/// the middle of the log, not at its torn end.
pub(crate) fn record_follows(bytes: &[u8], offset: usize) -> bool {
    let Some(header) = bytes.get(offset..).and_then(|rest| rest.first_chunk::<FRAME_HEADER>())
    else {
        return false;
    };
    let len = u32::from_le_bytes([header[0], header[1], header[2], header[3]]) as usize;
    matches!(frame_at(bytes, offset + FRAME_HEADER + len), Frame::Record { .. })
}

/// Reads the payload of a record.
pub(crate) fn decode(payload: &[u8]) -> Result<Record, String> {
    rmp_serde::from_slice(payload)
        .map_err(|e| format!("the record cannot be read as MessagePack: {e}"))
}

#[cfg(test)]
mod tests {
    use unsup_protocol::ZoneId;

    use super::*;

    fn step(zone: u32) -> Input {
        Input::Step { skip: vec![ZoneId(zone)] }
    }

    #[test]
    fn records_in_a_row_are_read_in_a_row() {
        let mut bytes = Vec::new();
        encode(7, &step(1), 1_000, &mut bytes).unwrap();
        let first = bytes.len();
        encode(8, &step(2), 1_000, &mut bytes).unwrap();

        let Frame::Record { payload, next } = frame_at(&bytes, 0) else { panic!("a record") };
        assert_eq!(next, first);
        let record = decode(payload).unwrap();
        assert_eq!(record.seq, 7);
        assert!(matches!(record.input, Input::Step { skip } if skip == [ZoneId(1)]));
        let Frame::Record { payload, next } = frame_at(&bytes, next) else { panic!("a record") };
        assert_eq!((decode(payload).unwrap().seq, next), (8, bytes.len()));
        assert!(matches!(frame_at(&bytes, next), Frame::End));
    }

    #[test]
    fn every_cut_and_every_flipped_bit_is_detected() {
        let mut bytes = Vec::new();
        encode(1, &step(3), 1_000, &mut bytes).unwrap();
        for cut in 1..bytes.len() {
            assert!(matches!(frame_at(&bytes[..cut], 0), Frame::Torn(_)), "cut at {cut}");
        }
        for position in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[position] ^= 0x10;
            assert!(matches!(frame_at(&damaged, 0), Frame::Torn(_)), "flip at {position}");
        }
    }

    #[test]
    fn damage_before_a_valid_record_is_not_a_torn_end() {
        let mut bytes = Vec::new();
        encode(1, &step(3), 1_000, &mut bytes).unwrap();
        let second = bytes.len();
        encode(2, &step(4), 1_000, &mut bytes).unwrap();
        // The payload of the first record is damaged; its length is intact.
        bytes[FRAME_HEADER + 2] ^= 1;
        assert!(matches!(frame_at(&bytes, 0), Frame::Torn(_)));
        assert!(record_follows(&bytes, 0));
        // The last record is damaged: nothing follows it.
        bytes[FRAME_HEADER + 2] ^= 1;
        bytes[second + FRAME_HEADER + 2] ^= 1;
        assert!(!record_follows(&bytes, second));
        // A record that is cut off has nothing after it either.
        assert!(!record_follows(&bytes[..second + 3], second));
    }

    #[test]
    fn an_input_above_the_limit_is_refused_and_leaves_no_bytes() {
        let mut bytes = vec![9];
        let error = encode(1, &step(3), 4, &mut bytes).unwrap_err();
        assert!(error.contains("the limit for one record is 4 bytes"), "{error}");
        assert_eq!(bytes, [9]);
    }
}
