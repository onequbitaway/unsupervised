//! The chronicle on a storage: a journal with one line of JSON for each
//! record.
//!
//! The journal is a segment that grows at its end. Its first line names
//! the world that it is from. From time to time, and after every start
//! and every failed write, the service writes the journal again from what
//! it holds in memory, under a new name, and removes the old one. A
//! journal whose last line is torn (the process was killed in the middle
//! of a write) loses only that line.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use unsup_runtime::storage::{Segment, Storage, StorageError};

use super::Record;

/// The directory of the journals on the storage.
const DIR: &str = "chronicle";
/// The version of the journal format.
const VERSION: u32 = 1;

/// The first line of a journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Head {
    /// The version of the format.
    chronicle: u32,
    /// The signature of the world that the journal is from.
    world: u64,
}

fn name_of(generation: u64) -> String {
    format!("{DIR}/journal-{generation:010}.jsonl")
}

/// Reads the number of a journal from its name.
fn generation_of(name: &str) -> Option<u64> {
    name.strip_prefix(DIR)?.strip_prefix("/journal-")?.strip_suffix(".jsonl")?.parse().ok()
}

/// Reads a journal of a world. `None`: the bytes are no journal of this
/// world. A line that does not parse ends the journal.
fn read_journal(bytes: &[u8], world: u64) -> Option<Vec<Record>> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let head: Head = serde_json::from_slice(lines.next()?).ok()?;
    if head.chronicle != VERSION || head.world != world {
        return None;
    }
    let mut records = Vec::new();
    for line in lines.filter(|line| !line.is_empty()) {
        match serde_json::from_slice::<Record>(line) {
            Ok(record) => records.push(record),
            Err(error) => {
                tracing::warn!(%error, "the journal of the chronicle ends in a line that is not whole");
                break;
            }
        }
    }
    Some(records)
}

fn line_of<T: Serialize>(value: &T, out: &mut Vec<u8>) {
    // A record is plain data: it always has a JSON form.
    if serde_json::to_writer(&mut *out, value).is_ok() {
        out.push(b'\n');
    }
}

/// The journal of the chronicle on a storage.
pub(crate) struct Archive {
    storage: Arc<dyn Storage>,
    world: u64,
    /// The number of the journal that the archive writes to.
    generation: u64,
    segment: Option<Arc<dyn Segment>>,
    /// Records in the journal.
    records: usize,
    /// True when the journal must be written again before the next line:
    /// after the start, and after a write that failed.
    stale: bool,
}

impl std::fmt::Debug for Archive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Archive")
            .field("generation", &self.generation)
            .field("records", &self.records)
            .field("stale", &self.stale)
            .finish_non_exhaustive()
    }
}

impl Archive {
    /// Opens the archive of a world and returns the records of its newest
    /// journal. A journal of another world gives no records.
    ///
    /// The archive does not continue the old journal: the first write is a
    /// new journal ([`Archive::rewrite`]).
    ///
    /// # Errors
    ///
    /// Fails when the storage cannot list or read the journals.
    pub fn open(
        storage: Arc<dyn Storage>,
        world: u64,
    ) -> Result<(Archive, Vec<Record>), StorageError> {
        let mut generations: Vec<u64> =
            storage.list(DIR)?.iter().filter_map(|name| generation_of(name)).collect();
        generations.sort_unstable();
        let newest = generations.last().copied().unwrap_or(0);
        let mut records = Vec::new();
        // The newest journal that is whole enough to have its first line.
        for generation in generations.iter().rev() {
            let bytes = storage.read(&name_of(*generation))?;
            if let Some(found) = read_journal(&bytes, world) {
                records = found;
                break;
            }
        }
        let archive =
            Archive { storage, world, generation: newest, segment: None, records: 0, stale: true };
        Ok((archive, records))
    }

    /// Returns true when the next write must be [`Archive::rewrite`]: the
    /// journal is not safe to continue, or it has more than `limit`
    /// records.
    pub fn wants_rewrite(&self, limit: usize) -> bool {
        self.stale || self.records > limit
    }

    /// Adds records at the end of the journal and forces them to the disk.
    ///
    /// # Errors
    ///
    /// Fails when the storage fails. The journal is then written again at
    /// the next write.
    pub fn append(&mut self, records: &[Record]) -> Result<(), StorageError> {
        if records.is_empty() {
            return Ok(());
        }
        let Some(segment) = self.segment.clone() else {
            self.stale = true;
            return Err(StorageError::not_found(&name_of(self.generation)));
        };
        let mut bytes = Vec::new();
        for record in records {
            line_of(record, &mut bytes);
        }
        let done = segment.append(&bytes).and_then(|()| segment.sync());
        match done {
            Ok(()) => self.records += records.len(),
            // A part of the bytes can be in the journal.
            Err(_) => self.stale = true,
        }
        done
    }

    /// Writes a new journal with these records, and removes the older
    /// journals.
    ///
    /// # Errors
    ///
    /// Fails when the storage fails. The older journal then stays.
    pub fn rewrite(&mut self, records: &[Record]) -> Result<(), StorageError> {
        let mut bytes = Vec::new();
        line_of(&Head { chronicle: VERSION, world: self.world }, &mut bytes);
        for record in records {
            line_of(record, &mut bytes);
        }
        let generation = self.generation + 1;
        let name = name_of(generation);
        self.storage.write(&name, &[&bytes])?;
        self.segment = Some(self.storage.append(&name)?);
        self.generation = generation;
        self.records = records.len();
        self.stale = false;
        // An old journal that stays does no harm: the newest one wins.
        for old in self.storage.list(DIR)? {
            if generation_of(&old).is_some_and(|number| number < generation) {
                self.storage.delete(&old)?;
            }
        }
        Ok(())
    }
}
