//! Reads, writes and rotates the snapshot files of a storage.

use super::format::{
    SNAPSHOT_DIR, SNAPSHOT_HEADER, SnapshotMeta, check_payload, decode_header, encode_header, name,
    parse_name,
};
use crate::log::{LogError, LogIndex};
use crate::storage::{Storage, StorageError};

/// One snapshot file of a storage, as its name tells it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    /// Storage name of the file.
    pub name: String,
    /// Number of inputs that the world had applied when the snapshot was
    /// taken.
    pub seq: u64,
    /// Number of the last frame of the world in the snapshot.
    pub frame: u64,
}

/// Lists the snapshot files of a storage, oldest first. Other files in the
/// directory are ignored.
///
/// The call reads only the names. A host can call it for the storage of a
/// world that runs: a snapshot appears in the list when it is whole. The
/// list is what a rollback can go to. Right after a rollback it can still
/// hold a later snapshot that could not be deleted; [`newest`] passes over
/// such a file.
///
/// # Errors
///
/// Fails when the storage cannot list the directory.
pub fn list(storage: &dyn Storage) -> Result<Vec<SnapshotEntry>, StorageError> {
    // The names sort like (sequence number, frame).
    Ok(storage
        .list(SNAPSHOT_DIR)?
        .into_iter()
        .filter_map(|name| parse_name(&name).map(|(seq, frame)| SnapshotEntry { name, seq, frame }))
        .collect())
}

/// A snapshot file in memory, checked.
pub(crate) struct Loaded {
    /// The header fields.
    pub meta: SnapshotMeta,
    bytes: Vec<u8>,
}

impl Loaded {
    /// Returns what `World::snapshot` wrote.
    pub(crate) fn payload(&self) -> &[u8] {
        &self.bytes[SNAPSHOT_HEADER..]
    }
}

fn check_name(entry: &SnapshotEntry, meta: &SnapshotMeta) -> Result<(), String> {
    if (meta.seq, meta.frame) != (entry.seq, entry.frame) {
        return Err("the header does not match the name of the file".into());
    }
    Ok(())
}

/// Reads a snapshot file and checks its header and its payload. The error
/// says why the file cannot be used.
pub(crate) fn load(storage: &dyn Storage, entry: &SnapshotEntry) -> Result<Loaded, String> {
    let bytes = storage.read(&entry.name).map_err(|e| e.to_string())?;
    let header = decode_header(&bytes)?;
    check_name(entry, &header.meta)?;
    check_payload(&header, &bytes[SNAPSHOT_HEADER..])?;
    Ok(Loaded { meta: header.meta, bytes })
}

/// Reads only the header of a snapshot file.
pub(crate) fn head(storage: &dyn Storage, entry: &SnapshotEntry) -> Result<SnapshotMeta, String> {
    let bytes = storage.read_head(&entry.name, SNAPSHOT_HEADER).map_err(|e| e.to_string())?;
    let header = decode_header(&bytes)?;
    check_name(entry, &header.meta)?;
    Ok(header.meta)
}

/// Reads the header of the newest snapshot that a recovery or a rollback
/// can use: its frame, its sequence number and the state hash of the world
/// at that moment. `None` when the storage has no such snapshot.
///
/// The call is cheap. It lists the names of the snapshots and of the log
/// segments and reads the header of one file, 112 bytes. It does not read
/// the state. A host can call it for the storage of a world that runs, for
/// example for a dashboard.
///
/// A file that a rollback made void, and a file whose header cannot be
/// read, is passed over.
///
/// # Errors
///
/// Fails when the storage cannot list the snapshots or the log segments,
/// and when two log segments have the same index.
pub fn newest(storage: &dyn Storage) -> Result<Option<SnapshotMeta>, LogError> {
    let index = LogIndex::load(storage)?;
    for entry in list(storage)?.iter().rev() {
        match head(storage, entry) {
            Ok(meta) if !index.voids(meta.seq, meta.segment) => return Ok(Some(meta)),
            // Void, damaged, or deleted by the rotation since the list
            // was read.
            Ok(_) | Err(_) => {}
        }
    }
    Ok(None)
}

/// Writes a snapshot file. Returns its size in bytes.
pub(crate) fn write(
    storage: &dyn Storage,
    meta: &SnapshotMeta,
    payload: &[u8],
) -> Result<u64, StorageError> {
    let header = encode_header(meta, payload);
    storage.write(&name(meta.seq, meta.frame), &[&header, payload])?;
    Ok((header.len() + payload.len()) as u64)
}

/// Deletes the snapshots that a rollback made void, then all but the
/// newest `keep` snapshots, then the log segments that no remaining
/// snapshot needs.
pub(crate) fn prune(storage: &dyn Storage, keep: usize) -> Result<(), String> {
    let index = LogIndex::load(storage).map_err(|e| e.to_string())?;
    let mut live = Vec::new();
    for entry in list(storage).map_err(|e| e.to_string())? {
        match head(storage, &entry) {
            Ok(meta) if index.voids(meta.seq, meta.segment) => {
                storage.delete(&entry.name).map_err(|e| e.to_string())?;
            }
            // A file that cannot be read stays for the operator to look at.
            // It does not count as one of the snapshots to keep.
            Err(_) => {}
            Ok(_) => live.push(entry),
        }
    }
    let extra = live.len().saturating_sub(keep.max(1));
    for entry in &live[..extra] {
        storage.delete(&entry.name).map_err(|e| e.to_string())?;
    }
    if let Some(oldest) = live.get(extra) {
        for slot in index.dead_before(oldest.seq) {
            storage.delete(&slot.name).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
