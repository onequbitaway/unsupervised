//! The error of a store call.

/// Why a store call failed.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// A record with the same key exists already.
    #[error("the record exists already: {0}")]
    Conflict(String),
    /// The record that the call names does not exist.
    #[error("the record does not exist: {0}")]
    NotFound(String),
    /// The storage engine failed: the disk, the file or the schema.
    #[error("the storage engine failed: {0}")]
    Engine(String),
    /// The operating system gave no random bytes.
    #[error("the operating system gave no random bytes: {0}")]
    Random(String),
}
