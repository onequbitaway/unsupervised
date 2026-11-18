//! Reads, writes and rotates the snapshot files of a storage.

use super::format::{
    SNAPSHOT_DIR, SNAPSHOT_HEADER, SnapshotMeta, check_payload, decode_header, encode_header, name,
    parse_name,
};
use crate::log::LogIndex;
use crate::storage::{Storage, StorageError};

/// One snapshot file, from its name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SnapshotEntry {
    /// Storage name.
    pub name: String,
    /// Number of inputs in the snapshot.
    pub seq: u64,
    /// Frame of the snapshot.
    pub frame: u64,
}

/// Lists the snapshot files, oldest first. Other files in the directory are
/// ignored.
pub(crate) fn list(storage: &dyn Storage) -> Result<Vec<SnapshotEntry>, StorageError> {
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
