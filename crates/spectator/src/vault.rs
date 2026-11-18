//! What the service keeps on a storage: the journal of the chronicle and
//! the pinned clips.
//!
//! Everything on the storage belongs to one world. The signature of the
//! world (its cities and zones) is written with the data. Data of another
//! world is not read. Data of frames that the world does not have (the
//! server started a new world, or went back to a snapshot) is left out.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use unsup_protocol::{CityView, ZoneView};
use unsup_runtime::storage::{Storage, StorageError};

use crate::chronicle::Record;
use crate::chronicle::archive::Archive;
use crate::wire::{Clip, EventId};

/// The directory of the pinned clips on the storage.
const CLIPS: &str = "clips";

/// Returns the signature of a world: a number that is the same for the
/// same cities and zones, on every machine and in every build.
pub(crate) fn signature(cities: &[CityView], zones: &[ZoneView]) -> u64 {
    // FNV-1a, 64 bits.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |text: &str| {
        for byte in text.bytes().chain(std::iter::once(0xff)) {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for city in cities {
        let b = city.bounds;
        mix(&format!("c{} {} {} {} {} {}", city.id, b.x0, b.y0, b.x1, b.y1, city.name));
    }
    for zone in zones {
        let b = zone.bounds;
        mix(&format!(
            "z{} {} {} {} {} {} {}",
            zone.id, zone.city, b.x0, b.y0, b.x1, b.y1, zone.name
        ));
    }
    hash
}

/// A pinned clip on the storage, with the world that it is from.
#[derive(Serialize, Deserialize)]
struct StoredClip {
    world: u64,
    clip: Clip,
}

fn clip_name(id: EventId) -> String {
    format!("{CLIPS}/{id}.json")
}

/// What the service reads from the storage at its start.
#[derive(Debug, Default)]
pub(crate) struct Restored {
    /// The records of the chronicle.
    pub records: Vec<Record>,
    /// The pinned clips.
    pub clips: Vec<Clip>,
}

/// What the service has for the storage.
#[derive(Debug, Default)]
pub(crate) struct Work {
    /// New records of the chronicle, for the end of the journal.
    pub records: Vec<Record>,
    /// Everything that the chronicle holds, for a new journal. With it,
    /// `records` is not used.
    pub rewrite: Option<Vec<Record>>,
    /// New pinned clips.
    pub pins: Vec<Arc<Clip>>,
    /// Clips that are pinned no more.
    pub unpins: Vec<EventId>,
}

impl Work {
    /// Returns true when there is nothing to write.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
            && self.rewrite.is_none()
            && self.pins.is_empty()
            && self.unpins.is_empty()
    }
}

/// The storage of the service.
pub(crate) struct Vault {
    storage: Arc<dyn Storage>,
    world: u64,
    /// The journal of the chronicle. `None` until [`Vault::open`] worked.
    archive: Option<Archive>,
    /// Records in the journal after which the service writes it again.
    compact_records: usize,
}

impl Vault {
    /// Makes the vault of a storage. It reads nothing yet.
    pub fn new(storage: Arc<dyn Storage>, compact_records: usize) -> Vault {
        Vault { storage, world: 0, archive: None, compact_records }
    }

    /// Reads what the storage has for a world.
    ///
    /// A failure of the storage is logged. The service then starts with
    /// what could be read, and writes a new journal.
    pub fn open(&mut self, world: u64) -> Restored {
        self.world = world;
        let mut restored = Restored::default();
        match Archive::open(Arc::clone(&self.storage), world) {
            Ok((archive, records)) => {
                self.archive = Some(archive);
                restored.records = records;
            }
            Err(error) => tracing::error!(%error, "the chronicle cannot be read from the storage"),
        }
        match self.read_clips() {
            Ok(clips) => restored.clips = clips,
            Err(error) => {
                tracing::error!(%error, "the pinned clips cannot be read from the storage")
            }
        }
        restored
    }

    /// Reads the pinned clips of this world. A clip of another world, and
    /// a file that is not a clip, is removed.
    fn read_clips(&self) -> Result<Vec<Clip>, StorageError> {
        let mut clips = Vec::new();
        for name in self.storage.list(CLIPS)? {
            let bytes = self.storage.read(&name)?;
            match serde_json::from_slice::<StoredClip>(&bytes) {
                Ok(stored) if stored.world == self.world => clips.push(stored.clip),
                Ok(_) | Err(_) => self.storage.delete(&name)?,
            }
        }
        Ok(clips)
    }

    /// Returns true when the next [`Work`] must carry a `rewrite`.
    pub fn wants_rewrite(&self) -> bool {
        self.archive.as_ref().is_some_and(|archive| archive.wants_rewrite(self.compact_records))
    }

    /// Writes the work to the storage. A failure is logged; the journal is
    /// then written again with the next work, and a clip that was not
    /// written stays in memory only.
    pub fn apply(&mut self, work: Work) {
        if let Some(archive) = &mut self.archive {
            let done = match &work.rewrite {
                Some(records) => archive.rewrite(records),
                None => archive.append(&work.records),
            };
            if let Err(error) = done {
                tracing::error!(%error, "the chronicle cannot be written to the storage");
            }
        }
        for clip in &work.pins {
            let stored = StoredClip { world: self.world, clip: (**clip).clone() };
            let done =
                serde_json::to_vec(&stored).map_err(|error| error.to_string()).and_then(|bytes| {
                    self.storage.write(&clip_name(clip.id), &[&bytes]).map_err(|e| e.to_string())
                });
            if let Err(error) = done {
                tracing::error!(%error, clip = %clip.id, "a pinned clip cannot be written to the storage");
            }
        }
        for id in &work.unpins {
            if let Err(error) = self.storage.delete(&clip_name(*id)) {
                tracing::error!(%error, clip = %id, "a clip cannot be removed from the storage");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use unsup_protocol::{CityId, Rect, ZoneId};

    use super::*;

    fn city(name: &str) -> CityView {
        CityView {
            id: CityId(0),
            name: name.to_string(),
            bounds: Rect::sized(0, 0, 16, 8),
            zones: vec![ZoneId(0)],
        }
    }

    fn zone(width: u32) -> ZoneView {
        ZoneView {
            id: ZoneId(0),
            city: CityId(0),
            name: "Town 0-0".to_string(),
            bounds: Rect::sized(0, 0, width, 8),
            residents: 0,
        }
    }

    #[test]
    fn the_signature_follows_the_shape_of_the_world() {
        let base = signature(&[city("Town")], &[zone(8)]);
        // A fixed number: the signature is the same in every build.
        assert_eq!(base, signature(&[city("Town")], &[zone(8)]));
        assert_ne!(base, signature(&[city("Town B")], &[zone(8)]));
        assert_ne!(base, signature(&[city("Town")], &[zone(9)]));
        assert_ne!(base, signature(&[city("Town")], &[]));
        // The number of residents is no part of the shape.
        let busy = ZoneView { residents: 50, ..zone(8) };
        assert_eq!(base, signature(&[city("Town")], &[busy]));
        assert_eq!(signature(&[], &[]), 0xcbf2_9ce4_8422_2325);
    }
}
