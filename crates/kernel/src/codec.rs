//! MessagePack helpers for snapshots, messages and migration.
//!
//! Everything is written with field names (`rmp_serde::to_vec_named`). A
//! struct can then gain a field with `#[serde(default)]` and still read old
//! data.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A value could not be read from MessagePack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodeError {
    /// What was being read, for example the name of a component.
    pub what: String,
    /// The message of the decoder.
    pub message: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot decode {}: {}", self.what, self.message)
    }
}

impl std::error::Error for DecodeError {}

/// Writes a value as MessagePack with field names.
///
/// # Panics
///
/// Panics when the `Serialize` implementation of the value fails. That is a
/// bug in the type (for example a map whose keys are not strings or
/// numbers), not a runtime condition.
pub(crate) fn encode<T: Serialize + ?Sized>(what: &str, value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(what, value, &mut out);
    out
}

/// Appends a value as MessagePack with field names to `out`.
pub(crate) fn encode_into<T: Serialize + ?Sized>(what: &str, value: &T, out: &mut Vec<u8>) {
    if let Err(e) = rmp_serde::encode::write_named(out, value) {
        panic!("cannot encode {what} as MessagePack: {e}");
    }
}

/// Reads a value from MessagePack.
pub(crate) fn decode<T: DeserializeOwned>(what: &str, bytes: &[u8]) -> Result<T, DecodeError> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| DecodeError { what: what.to_string(), message: e.to_string() })
}

/// A reader that decodes several values that were written one after another.
pub(crate) type SeqReader<'a> = rmp_serde::Deserializer<rmp_serde::decode::ReadRefReader<'a, [u8]>>;

/// Makes a reader over values that were written with [`encode_into`].
pub(crate) fn seq_reader(bytes: &[u8]) -> SeqReader<'_> {
    rmp_serde::Deserializer::from_read_ref(bytes)
}

/// Serde adapter that writes a `Vec<u8>` as a MessagePack binary instead of
/// an array of numbers. Use it with `#[serde(with = "crate::codec::bytes")]`.
pub(crate) mod bytes {
    use std::fmt;

    use serde::de::{Error, SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};

    pub(crate) fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<u8>, D::Error> {
        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a byte array")
            }

            fn visit_bytes<E: Error>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }

            fn visit_byte_buf<E: Error>(self, v: Vec<u8>) -> Result<Vec<u8>, E> {
                Ok(v)
            }

            fn visit_str<E: Error>(self, v: &str) -> Result<Vec<u8>, E> {
                Ok(v.as_bytes().to_vec())
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<u8>, A::Error> {
                let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(1 << 16));
                while let Some(byte) = seq.next_element::<u8>()? {
                    out.push(byte);
                }
                Ok(out)
            }
        }

        deserializer.deserialize_byte_buf(BytesVisitor)
    }
}

/// Serde adapter that writes a `BTreeMap` as a list of `(key, value)` pairs.
/// It lets a map have keys that are not strings (tuples, enums) in every
/// format. Use it with `#[serde(with = "crate::codec::pairs")]`.
pub(crate) mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<K: Serialize, V: Serialize, S: Serializer>(
        map: &BTreeMap<K, V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }

    pub(crate) fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        let pairs: Vec<(K, V)> = Vec::deserialize(deserializer)?;
        Ok(pairs.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Blob {
        #[serde(with = "bytes")]
        data: Vec<u8>,
        n: u32,
    }

    #[test]
    fn bytes_are_written_as_a_binary() {
        let blob = Blob { data: vec![200; 300], n: 7 };
        let out = encode("blob", &blob);
        // 300 bytes as a binary cost 300 + a few header bytes. As an array of
        // numbers above 127 they would cost two bytes each.
        assert!(out.len() < 320, "{}", out.len());
        assert_eq!(decode::<Blob>("blob", &out).unwrap(), blob);
    }

    #[test]
    fn values_written_in_a_row_are_read_in_a_row() {
        let mut out = Vec::new();
        encode_into("a", &1u32, &mut out);
        encode_into("b", "two", &mut out);
        encode_into("c", &Blob { data: vec![1, 2], n: 3 }, &mut out);
        let mut reader = seq_reader(&out);
        assert_eq!(u32::deserialize(&mut reader).unwrap(), 1);
        assert_eq!(String::deserialize(&mut reader).unwrap(), "two");
        assert_eq!(Blob::deserialize(&mut reader).unwrap(), Blob { data: vec![1, 2], n: 3 });
        assert!(u32::deserialize(&mut reader).is_err());
    }

    #[test]
    fn decode_errors_say_what_failed() {
        let error = decode::<Blob>("component `needs`", &[0xc1]).unwrap_err();
        assert!(error.to_string().starts_with("cannot decode component `needs`:"), "{error}");
    }
}
