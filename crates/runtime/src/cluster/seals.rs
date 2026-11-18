//! Seals: the points to which the whole cluster can recover.
//!
//! The processes of a cluster stop at different inputs when one of them
//! dies. A *seal* is a point at which every realm had taken every message
//! that was sent to it. The coordinator numbers the seals (the *epoch*).
//! Each realm writes down, for each seal, how many inputs its log held at
//! that point. A recovery brings every realm back to the newest seal that
//! all of them have.
//!
//! # File
//!
//! The file `seals` in the storage of a realm:
//!
//! ```text
//! file   = "UNSUPSEA" | version u32 | record*
//! record = epoch u64 | frame u64 | seq u64 | check u32
//! ```
//!
//! All numbers are little-endian. `seq` is the number of inputs in the log
//! of the realm at the seal. `check` is the first four bytes of the blake3
//! hash of the three numbers. A record that cannot be read ends the file:
//! a crash can cut the last record off.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::storage::{Segment, Storage, StorageError, StorageErrorKind};

/// Storage name of the seal file of a realm.
const NAME: &str = "seals";

/// Marks the start of a seal file.
const MAGIC: &[u8; 8] = b"UNSUPSEA";

/// Version of the file format.
const VERSION: u32 = 1;

/// Bytes of the file header.
const HEADER: usize = 12;

/// Bytes of one record.
const RECORD: usize = 28;

/// One seal of one realm.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SealPoint {
    /// Number of the seal in the cluster.
    pub epoch: u64,
    /// Frame of the world at the seal.
    pub frame: u64,
    /// Number of inputs in the log of the realm at the seal.
    pub seq: u64,
}

fn check(body: &[u8]) -> [u8; 4] {
    let hash = blake3::hash(body);
    let bytes = hash.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

fn encode(point: &SealPoint) -> [u8; RECORD] {
    let mut record = [0; RECORD];
    record[..8].copy_from_slice(&point.epoch.to_le_bytes());
    record[8..16].copy_from_slice(&point.frame.to_le_bytes());
    record[16..24].copy_from_slice(&point.seq.to_le_bytes());
    let sum = check(&record[..24]);
    record[24..].copy_from_slice(&sum);
    record
}

fn header() -> [u8; HEADER] {
    let mut header = [0; HEADER];
    header[..8].copy_from_slice(MAGIC);
    header[8..].copy_from_slice(&VERSION.to_le_bytes());
    header
}

fn number(bytes: &[u8]) -> u64 {
    let mut raw = [0; 8];
    raw.copy_from_slice(bytes);
    u64::from_le_bytes(raw)
}

/// Reads the seals of a file. A file with another header holds none.
fn decode(bytes: &[u8]) -> Vec<SealPoint> {
    let mut points: Vec<SealPoint> = Vec::new();
    if bytes.get(..HEADER) != Some(header().as_slice()) {
        return points;
    }
    for record in bytes[HEADER..].chunks(RECORD) {
        if record.len() != RECORD || check(&record[..24]) != record[24..] {
            // The end that a crash cut off.
            break;
        }
        let point = SealPoint {
            epoch: number(&record[..8]),
            frame: number(&record[8..16]),
            seq: number(&record[16..24]),
        };
        // A seal with the number of an older one starts a new history,
        // like a new segment of the input log.
        points.retain(|older| older.epoch < point.epoch);
        points.push(point);
    }
    points
}

/// Reads the seals of a realm, oldest first. A realm without the file has
/// none.
pub(crate) fn read(storage: &dyn Storage) -> Result<Vec<SealPoint>, StorageError> {
    match storage.read(NAME) {
        Ok(bytes) => Ok(decode(&bytes)),
        Err(error) if error.kind == StorageErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

/// When a seal is forced to the disk. It follows the sync policy of the
/// input log: a seal is of use only when the log has the inputs before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SealSync {
    /// Every seal, when it is written.
    Always,
    /// A seal that is written this long or longer after the last sync.
    Every(Duration),
}

/// The seal file of one realm, open for new seals.
pub(crate) struct SealLog {
    storage: Arc<dyn Storage>,
    segment: Arc<dyn Segment>,
    points: Vec<SealPoint>,
    /// Number of seals to keep.
    keep: usize,
    sync: SealSync,
    /// When the file was last forced to the disk.
    synced: Instant,
}

/// Writes the file with exactly these seals and opens it.
fn rewrite(storage: &dyn Storage, points: &[SealPoint]) -> Result<Arc<dyn Segment>, StorageError> {
    let mut bytes = header().to_vec();
    for point in points {
        bytes.extend_from_slice(&encode(point));
    }
    storage.write(NAME, &[&bytes])?;
    storage.append(NAME)
}

impl SealLog {
    /// Starts the file with the seals that are still true. The write is
    /// atomic: seals of a history that a recovery dropped are gone after
    /// it.
    pub(crate) fn start(
        storage: Arc<dyn Storage>,
        mut points: Vec<SealPoint>,
        keep: usize,
        sync: SealSync,
    ) -> Result<SealLog, StorageError> {
        let keep = keep.max(1);
        let extra = points.len().saturating_sub(keep);
        points.drain(..extra);
        let segment = rewrite(&*storage, &points)?;
        Ok(SealLog { storage, segment, points, keep, sync, synced: Instant::now() })
    }

    /// Adds a seal.
    pub(crate) fn append(&mut self, point: SealPoint) -> Result<(), StorageError> {
        self.points.push(point);
        if self.points.len() > 2 * self.keep {
            // The file is rewritten with the newest seals, so that it does
            // not grow for ever.
            let extra = self.points.len() - self.keep;
            self.points.drain(..extra);
            // A blob is on the disk when the write returns.
            self.segment = rewrite(&*self.storage, &self.points)?;
            self.synced = Instant::now();
            return Ok(());
        }
        self.segment.append(&encode(&point))?;
        let due = match self.sync {
            SealSync::Always => true,
            SealSync::Every(interval) => self.synced.elapsed() >= interval,
        };
        if due {
            self.sync()?;
        }
        Ok(())
    }

    /// Forces the file to the disk.
    pub(crate) fn sync(&mut self) -> Result<(), StorageError> {
        self.segment.sync()?;
        self.synced = Instant::now();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStorage;

    fn point(epoch: u64) -> SealPoint {
        SealPoint { epoch, frame: epoch * 2, seq: epoch * 10 }
    }

    #[test]
    fn seals_are_read_back_in_order() {
        let disk = Arc::new(MemStorage::new());
        assert_eq!(read(&*disk).unwrap(), []);
        let mut log = SealLog::start(disk.clone(), Vec::new(), 8, SealSync::Always).unwrap();
        for epoch in 0..5 {
            log.append(point(epoch)).unwrap();
        }
        assert_eq!(read(&*disk).unwrap(), (0..5).map(point).collect::<Vec<_>>());
        // A start with the seals up to 2 drops the later ones for good.
        let kept: Vec<SealPoint> = (0..=2).map(point).collect();
        let mut log = SealLog::start(disk.clone(), kept.clone(), 8, SealSync::Always).unwrap();
        assert_eq!(read(&*disk).unwrap(), kept);
        log.append(SealPoint { epoch: 3, frame: 9, seq: 99 }).unwrap();
        assert_eq!(read(&*disk).unwrap()[3], SealPoint { epoch: 3, frame: 9, seq: 99 });
    }

    #[test]
    fn a_cut_or_damaged_record_ends_the_file() {
        let disk = Arc::new(MemStorage::new());
        let mut log = SealLog::start(disk.clone(), Vec::new(), 8, SealSync::Always).unwrap();
        for epoch in 0..3 {
            log.append(point(epoch)).unwrap();
        }
        let bytes = disk.read(NAME).unwrap();
        assert_eq!(bytes.len(), HEADER + 3 * RECORD);
        for cut in 1..RECORD {
            assert_eq!(decode(&bytes[..bytes.len() - cut]), [point(0), point(1)], "cut {cut}");
        }
        for position in HEADER + RECORD..HEADER + 2 * RECORD {
            let mut damaged = bytes.clone();
            damaged[position] ^= 0x40;
            assert_eq!(decode(&damaged), [point(0)], "flip at {position}");
        }
        let mut other = bytes.clone();
        other[0] ^= 1;
        assert_eq!(decode(&other), [], "another header");
    }

    #[test]
    fn a_seal_with_an_old_number_starts_a_new_history() {
        let mut bytes = header().to_vec();
        for epoch in [0, 1, 2, 3] {
            bytes.extend_from_slice(&encode(&point(epoch)));
        }
        let again = SealPoint { epoch: 2, frame: 77, seq: 7 };
        bytes.extend_from_slice(&encode(&again));
        assert_eq!(decode(&bytes), [point(0), point(1), again]);
    }

    #[test]
    fn a_seal_reaches_the_disk_as_the_policy_says() {
        // What a loss of power leaves of the file.
        let durable = |disk: &MemStorage| read(&disk.fork_durable()).unwrap();
        let always = Arc::new(MemStorage::new());
        let mut log = SealLog::start(always.clone(), Vec::new(), 8, SealSync::Always).unwrap();
        log.append(point(0)).unwrap();
        assert_eq!(durable(&always), [point(0)]);

        // With a long interval a seal waits for the next sync.
        let lazy = Arc::new(MemStorage::new());
        let hour = SealSync::Every(Duration::from_secs(3_600));
        let mut log = SealLog::start(lazy.clone(), vec![point(0)], 8, hour).unwrap();
        log.append(point(1)).unwrap();
        assert_eq!(read(&*lazy).unwrap(), [point(0), point(1)]);
        assert_eq!(durable(&lazy), [point(0)], "the start is a blob: it is on the disk");
        log.sync().unwrap();
        assert_eq!(durable(&lazy), [point(0), point(1)]);

        // With an interval of zero every seal is due.
        let eager = Arc::new(MemStorage::new());
        let now = SealSync::Every(Duration::ZERO);
        let mut log = SealLog::start(eager.clone(), Vec::new(), 8, now).unwrap();
        log.append(point(0)).unwrap();
        assert_eq!(durable(&eager), [point(0)]);
    }

    #[test]
    fn the_file_keeps_only_the_newest_seals() {
        let disk = Arc::new(MemStorage::new());
        let mut log = SealLog::start(disk.clone(), Vec::new(), 3, SealSync::Always).unwrap();
        for epoch in 0..20 {
            log.append(point(epoch)).unwrap();
        }
        let seals = read(&*disk).unwrap();
        assert!((3..=6).contains(&seals.len()), "{seals:?}");
        assert_eq!(seals.last(), Some(&point(19)));
        assert_eq!(seals[0].epoch + seals.len() as u64, 20, "no seal is missing in the row");
    }
}
