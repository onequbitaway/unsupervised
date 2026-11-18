//! Storage in memory, for tests and tools.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::{Segment, Storage, StorageError, StorageErrorKind, check_name};

/// A failure that [`MemStorage`] imitates, to test how the runtime reacts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// Appending fails. Half of the bytes reach the segment first, as when a
    /// disk fills up in the middle of a write.
    Append,
    /// Forcing a segment to the disk fails.
    Sync,
    /// Writing a blob fails.
    Write,
}

/// One stored name.
#[derive(Clone, Default)]
struct Entry {
    data: Vec<u8>,
    /// Number of bytes at the start of `data` that would survive a loss of
    /// power.
    durable: usize,
}

#[derive(Default)]
struct Inner {
    entries: BTreeMap<String, Entry>,
    fault: Option<Fault>,
}

/// Storage in memory.
///
/// A clone is a second handle to the same data, like a second path to the
/// same disk. The storage knows which bytes were forced to the disk, so a
/// test can look at what a crash would leave behind:
///
/// - [`MemStorage::fork`] copies everything that was written. That is the
///   disk after the process was killed.
/// - [`MemStorage::fork_durable`] copies only what was forced to the disk.
///   That is the disk after a loss of power.
#[derive(Clone, Default)]
pub struct MemStorage {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for MemStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemStorage").field("names", &self.lock().entries.len()).finish()
    }
}

impl MemStorage {
    /// Makes an empty storage.
    #[must_use]
    pub fn new() -> MemStorage {
        MemStorage::default()
    }

    /// The data is plain bytes: a thread that panicked while it held the
    /// lock cannot have left it in a state that is unsafe to read.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Returns an independent copy of everything that was written: what the
    /// disk holds after the process was killed.
    #[must_use]
    pub fn fork(&self) -> MemStorage {
        let inner = self.lock();
        MemStorage {
            inner: Arc::new(Mutex::new(Inner { entries: inner.entries.clone(), fault: None })),
        }
    }

    /// Returns an independent copy of everything that was forced to the
    /// disk: what the disk holds after a loss of power.
    #[must_use]
    pub fn fork_durable(&self) -> MemStorage {
        let inner = self.lock();
        let entries = inner
            .entries
            .iter()
            .map(|(name, entry)| {
                let data = entry.data[..entry.durable].to_vec();
                (name.clone(), Entry { durable: data.len(), data })
            })
            .collect();
        MemStorage { inner: Arc::new(Mutex::new(Inner { entries, fault: None })) }
    }

    /// Makes later operations fail in the given way, or work again with
    /// `None`.
    pub fn set_fault(&self, fault: Option<Fault>) {
        self.lock().fault = fault;
    }

    /// Returns the number of bytes stored under all names.
    #[must_use]
    pub fn total_bytes(&self) -> usize {
        self.lock().entries.values().map(|entry| entry.data.len()).sum()
    }
}

fn injected(name: &str, action: &str) -> StorageError {
    StorageError::new(
        StorageErrorKind::Io,
        name,
        format!("cannot {action}: the test storage imitates a failure"),
    )
}

impl Storage for MemStorage {
    fn list(&self, dir: &str) -> Result<Vec<String>, StorageError> {
        check_name(dir)?;
        let prefix = format!("{dir}/");
        let inner = self.lock();
        Ok(inner
            .entries
            .keys()
            .filter(|name| {
                name.strip_prefix(&prefix)
                    .is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
            })
            .cloned()
            .collect())
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        check_name(name)?;
        self.lock()
            .entries
            .get(name)
            .map(|entry| entry.data.clone())
            .ok_or_else(|| StorageError::not_found(name))
    }

    fn write(&self, name: &str, parts: &[&[u8]]) -> Result<(), StorageError> {
        check_name(name)?;
        let mut inner = self.lock();
        if inner.fault == Some(Fault::Write) {
            return Err(injected(name, "write the blob"));
        }
        let data = parts.concat();
        inner.entries.insert(name.to_string(), Entry { durable: data.len(), data });
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), StorageError> {
        check_name(name)?;
        self.lock().entries.remove(name);
        Ok(())
    }

    fn append(&self, name: &str) -> Result<Arc<dyn Segment>, StorageError> {
        check_name(name)?;
        self.lock().entries.entry(name.to_string()).or_default();
        Ok(Arc::new(MemSegment { storage: self.clone(), name: name.to_string() }))
    }
}

/// A segment of a [`MemStorage`] that is open for appending.
struct MemSegment {
    storage: MemStorage,
    name: String,
}

impl Segment for MemSegment {
    fn append(&self, bytes: &[u8]) -> Result<(), StorageError> {
        let mut inner = self.storage.lock();
        let failing = inner.fault == Some(Fault::Append);
        let entry = inner.entries.entry(self.name.clone()).or_default();
        if failing {
            entry.data.extend_from_slice(&bytes[..bytes.len() / 2]);
            return Err(injected(&self.name, "append to the segment"));
        }
        entry.data.extend_from_slice(bytes);
        Ok(())
    }

    fn sync(&self) -> Result<(), StorageError> {
        let mut inner = self.storage.lock();
        if inner.fault == Some(Fault::Sync) {
            return Err(injected(&self.name, "sync the segment"));
        }
        if let Some(entry) = inner.entries.get_mut(&self.name) {
            entry.durable = entry.data.len();
        }
        Ok(())
    }
}
