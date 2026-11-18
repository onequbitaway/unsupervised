//! The two codecs: JSON (text) and MessagePack (binary, written with field
//! names).
//!
//! Both carry the same types. MessagePack is always written with field names
//! (`rmp_serde::to_vec_named`) so that optional fields and tagged enums work
//! and so that a reader can skip fields that it does not know.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A wire format.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Format {
    /// JSON text.
    Json,
    /// MessagePack with field names.
    MsgPack,
}

/// An encode or decode failure.
#[derive(Debug)]
pub enum CodecError {
    /// JSON could not be written or read.
    Json(serde_json::Error),
    /// MessagePack could not be written.
    MsgPackEncode(rmp_serde::encode::Error),
    /// MessagePack could not be read.
    MsgPackDecode(rmp_serde::decode::Error),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Json(e) => write!(f, "json: {e}"),
            CodecError::MsgPackEncode(e) => write!(f, "messagepack encode: {e}"),
            CodecError::MsgPackDecode(e) => write!(f, "messagepack decode: {e}"),
        }
    }
}

impl std::error::Error for CodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            CodecError::Json(e) => Some(e),
            CodecError::MsgPackEncode(e) => Some(e),
            CodecError::MsgPackDecode(e) => Some(e),
        }
    }
}

/// Writes a value as a JSON string.
pub fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String, CodecError> {
    serde_json::to_string(value).map_err(CodecError::Json)
}

/// Writes a value as JSON bytes.
pub fn to_json_bytes<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    serde_json::to_vec(value).map_err(CodecError::Json)
}

/// Reads a value from a JSON string.
pub fn from_json<T: DeserializeOwned>(text: &str) -> Result<T, CodecError> {
    serde_json::from_str(text).map_err(CodecError::Json)
}

/// Reads a value from JSON bytes.
pub fn from_json_bytes<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    serde_json::from_slice(bytes).map_err(CodecError::Json)
}

/// Writes a value as MessagePack with field names.
pub fn to_msgpack<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, CodecError> {
    rmp_serde::to_vec_named(value).map_err(CodecError::MsgPackEncode)
}

/// Reads a value from MessagePack.
pub fn from_msgpack<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    rmp_serde::from_slice(bytes).map_err(CodecError::MsgPackDecode)
}

/// Writes a value in the given format.
pub fn encode<T: Serialize + ?Sized>(format: Format, value: &T) -> Result<Vec<u8>, CodecError> {
    match format {
        Format::Json => to_json_bytes(value),
        Format::MsgPack => to_msgpack(value),
    }
}

/// Reads a value in the given format.
pub fn decode<T: DeserializeOwned>(format: Format, bytes: &[u8]) -> Result<T, CodecError> {
    match format {
        Format::Json => from_json_bytes(bytes),
        Format::MsgPack => from_msgpack(bytes),
    }
}
