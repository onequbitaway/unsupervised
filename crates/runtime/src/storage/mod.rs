//! Storage for the input log and the snapshots: named blobs and append-only
//! segments.
//!
//! The runtime writes two kinds of data. A *blob* is written in one piece
//! and replaces the old content of its name atomically: a reader sees the
//! old blob or the new blob, never a mixture. A *segment* grows at its end
//! and is forced to the disk when the sync policy says so.
//!
//! [`FsStorage`] keeps the data in a directory of the local file system.
//! [`MemStorage`] keeps it in memory, for tests and tools. An object store
//! can implement the same trait later.

mod fs;
mod mem;

use std::fmt;
use std::sync::Arc;

pub use fs::FsStorage;
pub use mem::{Fault, MemStorage};

/// Greatest length of a storage name in bytes.
const NAME_MAX: usize = 255;

/// Why a storage operation failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StorageErrorKind {
    /// Nothing is stored under the name.
    NotFound,
    /// The name is not a valid storage name.
    InvalidName,
    /// The device or the operating system refused the operation.
    Io,
}

/// A storage operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageError {
    /// The class of the failure.
    pub kind: StorageErrorKind,
    /// The name that the operation was for.
    pub name: String,
    /// What went wrong, for the operator.
    pub message: String,
}

impl StorageError {
    /// Makes an error of the given kind.
    #[must_use]
    pub fn new(kind: StorageErrorKind, name: &str, message: impl Into<String>) -> StorageError {
        StorageError { kind, name: name.to_string(), message: message.into() }
    }

    /// Makes the error for a name that holds nothing.
    #[must_use]
    pub fn not_found(name: &str) -> StorageError {
        StorageError::new(StorageErrorKind::NotFound, name, "nothing is stored under this name")
    }

    /// Makes an error from a failure of the operating system.
    #[must_use]
    pub fn io(name: &str, action: &str, error: &std::io::Error) -> StorageError {
        let kind = if error.kind() == std::io::ErrorKind::NotFound {
            StorageErrorKind::NotFound
        } else {
            StorageErrorKind::Io
        };
        StorageError::new(kind, name, format!("cannot {action}: {error}"))
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "storage `{}`: {}", self.name, self.message)
    }
}

impl std::error::Error for StorageError {}

/// A place for named blobs and append-only segments.
///
/// A name is a relative path with `/` between its parts, for example
/// `log/0000000001-00000000000000000000.seg`. Each part is made of ASCII
/// letters, digits, `.`, `_` and `-`, and does not start with a dot.
///
/// All methods can be called from several threads.
pub trait Storage: Send + Sync + 'static {
    /// Returns the names in a directory, sorted. A directory that does not
    /// exist is empty.
    fn list(&self, dir: &str) -> Result<Vec<String>, StorageError>;

    /// Returns the whole content of a blob or a segment.
    fn read(&self, name: &str) -> Result<Vec<u8>, StorageError>;

    /// Returns at most the first `len` bytes of a blob or a segment. The
    /// default reads everything; an implementation can do better.
    fn read_head(&self, name: &str, len: usize) -> Result<Vec<u8>, StorageError> {
        let mut bytes = self.read(name)?;
        bytes.truncate(len);
        Ok(bytes)
    }

    /// Writes a blob made of `parts` in a row. The write is atomic and
    /// durable: after a crash the name holds the old content or the new
    /// content, and after the call returns it holds the new content.
    fn write(&self, name: &str, parts: &[&[u8]]) -> Result<(), StorageError>;

    /// Removes a name. A name that holds nothing is not an error.
    fn delete(&self, name: &str) -> Result<(), StorageError>;

    /// Opens a segment to add data at its end. A name that holds nothing
    /// becomes an empty segment.
    fn append(&self, name: &str) -> Result<Arc<dyn Segment>, StorageError>;
}

/// An open segment. One thread appends; another thread can force the data
/// to the disk at the same time.
pub trait Segment: Send + Sync {
    /// Adds bytes at the end. After a failure a part of the bytes can be in
    /// the segment.
    fn append(&self, bytes: &[u8]) -> Result<(), StorageError>;

    /// Forces everything that was appended to the disk.
    fn sync(&self) -> Result<(), StorageError>;
}

/// Checks a storage name. `what` is the name itself or a directory.
pub(crate) fn check_name(name: &str) -> Result<(), StorageError> {
    let fail = |message: &str| Err(StorageError::new(StorageErrorKind::InvalidName, name, message));
    if name.is_empty() {
        return fail("the name is empty");
    }
    if name.len() > NAME_MAX {
        return fail("the name is longer than 255 bytes");
    }
    for part in name.split('/') {
        if part.is_empty() {
            return fail("a part of the name is empty");
        }
        if part.starts_with('.') {
            return fail("a part of the name starts with a dot");
        }
        if !part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-')) {
            return fail("only ASCII letters, digits, `.`, `_`, `-` and `/` are allowed");
        }
    }
    Ok(())
}
