//! The transport frame: a length and a MessagePack payload.
//!
//! ```text
//! frame   = length u32 | payload
//! payload = MessagePack with field names of one protocol message
//! ```
//!
//! The length is little-endian and counts the payload. A frame is written
//! with one call, so that the operating system sends it as one piece.

use std::fmt;
use std::io::{ErrorKind, Read, Write};

use serde::de::{DeserializeOwned, Error as _, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Bytes before the payload.
const HEADER: usize = 4;

/// A frame could not be sent or received.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum WireError {
    /// The other side closed the connection.
    Closed,
    /// The operating system reported an error.
    Io(String),
    /// A frame is larger than the limit.
    TooLarge {
        /// Bytes of the payload.
        len: u64,
        /// The limit in bytes.
        max: u32,
    },
    /// A message cannot be written as MessagePack.
    Encode(String),
    /// A payload is not a protocol message.
    Decode(String),
}

impl fmt::Display for WireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WireError::Closed => f.write_str("the connection is closed"),
            WireError::Io(message) => write!(f, "the connection failed: {message}"),
            WireError::TooLarge { len, max } => {
                write!(f, "a frame of {len} bytes is above the limit of {max} bytes")
            }
            WireError::Encode(message) => write!(f, "a message cannot be written: {message}"),
            WireError::Decode(message) => write!(f, "a frame cannot be read: {message}"),
        }
    }
}

/// Maps a failure of the operating system.
fn io_error(error: &std::io::Error) -> WireError {
    match error.kind() {
        ErrorKind::UnexpectedEof
        | ErrorKind::ConnectionReset
        | ErrorKind::ConnectionAborted
        | ErrorKind::BrokenPipe => WireError::Closed,
        _ => WireError::Io(error.to_string()),
    }
}

/// Writes one message as a frame. Returns the bytes that were written.
pub(crate) fn write_frame<W: Write, T: Serialize>(
    out: &mut W,
    message: &T,
    max: u32,
) -> Result<usize, WireError> {
    let mut frame = vec![0; HEADER];
    rmp_serde::encode::write_named(&mut frame, message)
        .map_err(|e| WireError::Encode(e.to_string()))?;
    let len = frame.len() - HEADER;
    let Some(len) = u32::try_from(len).ok().filter(|&len| len <= max) else {
        return Err(WireError::TooLarge { len: len as u64, max });
    };
    frame[..HEADER].copy_from_slice(&len.to_le_bytes());
    out.write_all(&frame).map_err(|e| io_error(&e))?;
    out.flush().map_err(|e| io_error(&e))?;
    Ok(frame.len())
}

/// Reads one frame and decodes its message. Returns the message and the
/// bytes that were read.
pub(crate) fn read_frame<R: Read, T: DeserializeOwned>(
    input: &mut R,
    max: u32,
) -> Result<(T, usize), WireError> {
    let mut header = [0; HEADER];
    input.read_exact(&mut header).map_err(|e| io_error(&e))?;
    let len = u32::from_le_bytes(header);
    if len > max {
        return Err(WireError::TooLarge { len: u64::from(len), max });
    }
    let mut payload = vec![0; len as usize];
    input.read_exact(&mut payload).map_err(|e| io_error(&e))?;
    let message = rmp_serde::from_slice(&payload).map_err(|e| WireError::Decode(e.to_string()))?;
    Ok((message, HEADER + payload.len()))
}

/// A byte string. It is written as a MessagePack binary, not as an array
/// of numbers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Blob(pub Vec<u8>);

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Blob, D::Error> {
        struct BlobVisitor;

        impl<'de> Visitor<'de> for BlobVisitor {
            type Value = Blob;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a byte string")
            }

            fn visit_bytes<E: serde::de::Error>(self, bytes: &[u8]) -> Result<Blob, E> {
                Ok(Blob(bytes.to_vec()))
            }

            fn visit_byte_buf<E: serde::de::Error>(self, bytes: Vec<u8>) -> Result<Blob, E> {
                Ok(Blob(bytes))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Blob, A::Error> {
                let mut bytes = Vec::with_capacity(seq.size_hint().unwrap_or(0));
                while let Some(byte) = seq.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(Blob(bytes))
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Blob, E> {
                Err(E::custom(format!("text `{text}` where bytes must be")))
            }
        }

        deserializer.deserialize_bytes(BlobVisitor)
    }
}

/// A hash of 32 bytes, written as a MessagePack binary.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Hash(pub [u8; 32]);

impl Serialize for Hash {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Hash {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Hash, D::Error> {
        let Blob(bytes) = Blob::deserialize(deserializer)?;
        <[u8; 32]>::try_from(bytes.as_slice())
            .map(Hash)
            .map_err(|_| D::Error::custom(format!("a hash of {} bytes, not 32", bytes.len())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        data: Blob,
        hash: Hash,
    }

    fn sample() -> Sample {
        Sample { name: "zone 3".into(), data: Blob(vec![0, 1, 2, 255]), hash: Hash([7; 32]) }
    }

    #[test]
    fn a_frame_carries_one_message_and_its_length() {
        let mut wire = Vec::new();
        let written = write_frame(&mut wire, &sample(), 1_000).unwrap();
        assert_eq!(written, wire.len());
        assert_eq!(
            u32::from_le_bytes([wire[0], wire[1], wire[2], wire[3]]) as usize,
            wire.len() - 4
        );
        // The blob is a binary of four bytes: marker, length, content.
        assert!(wire.windows(6).any(|w| w == [0xc4, 4, 0, 1, 2, 255]), "{wire:?}");
        let (message, read): (Sample, usize) = read_frame(&mut wire.as_slice(), 1_000).unwrap();
        assert_eq!((message, read), (sample(), written));
    }

    #[test]
    fn two_frames_in_a_row_are_read_in_a_row() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &1u32, 100).unwrap();
        write_frame(&mut wire, &"two", 100).unwrap();
        let mut input = wire.as_slice();
        assert_eq!(read_frame::<_, u32>(&mut input, 100).unwrap().0, 1);
        assert_eq!(read_frame::<_, String>(&mut input, 100).unwrap().0, "two");
        assert_eq!(read_frame::<_, u32>(&mut input, 100).unwrap_err(), WireError::Closed);
    }

    #[test]
    fn a_frame_above_the_limit_is_refused_on_both_sides() {
        let mut wire = Vec::new();
        let error = write_frame(&mut wire, &sample(), 8).unwrap_err();
        assert!(matches!(error, WireError::TooLarge { max: 8, .. }), "{error}");
        assert_eq!(wire, Vec::<u8>::new(), "nothing was sent");
        write_frame(&mut wire, &sample(), 1_000).unwrap();
        let error = read_frame::<_, Sample>(&mut wire.as_slice(), 8).unwrap_err();
        assert!(matches!(error, WireError::TooLarge { max: 8, .. }), "{error}");
    }

    #[test]
    fn a_cut_frame_and_a_wrong_payload_are_errors() {
        let mut wire = Vec::new();
        write_frame(&mut wire, &sample(), 1_000).unwrap();
        for cut in 0..wire.len() {
            let error = read_frame::<_, Sample>(&mut &wire[..cut], 1_000).unwrap_err();
            assert_eq!(error, WireError::Closed, "cut at {cut}");
        }
        let error = read_frame::<_, u32>(&mut wire.as_slice(), 1_000).unwrap_err();
        assert!(matches!(error, WireError::Decode(_)), "{error}");
        // A hash must have 32 bytes.
        let mut short = Vec::new();
        write_frame(&mut short, &Blob(vec![1; 31]), 1_000).unwrap();
        let error = read_frame::<_, Hash>(&mut short.as_slice(), 1_000).unwrap_err();
        assert!(matches!(&error, WireError::Decode(m) if m.contains("31 bytes")), "{error}");
    }
}
