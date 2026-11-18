//! The list of segment files and which part of each is still valid.

use super::LogError;
use super::segment::{LOG_DIR, SegmentId};
use crate::storage::Storage;

/// One segment file.
#[derive(Clone, Debug)]
pub(crate) struct Slot {
    /// Index and first sequence number, from the name.
    pub id: SegmentId,
    /// Storage name.
    pub name: String,
    /// Records of this segment with this sequence number or a higher one
    /// are void: a later segment starts there. `u64::MAX` for the last
    /// segment.
    pub limit: u64,
}

/// The segment files of a log, in the order of their indexes.
#[derive(Clone, Debug, Default)]
pub(crate) struct LogIndex {
    pub segments: Vec<Slot>,
}

impl LogIndex {
    /// Lists the segment files. Other files in the directory are ignored.
    pub(crate) fn load(storage: &dyn Storage) -> Result<LogIndex, LogError> {
        let mut segments: Vec<Slot> = storage
            .list(LOG_DIR)?
            .into_iter()
            .filter_map(|name| SegmentId::parse(&name).map(|id| Slot { id, name, limit: u64::MAX }))
            .collect();
        segments.sort_by_key(|slot| slot.id.index);
        if let Some(pair) = segments.windows(2).find(|pair| pair[0].id.index == pair[1].id.index) {
            return Err(LogError::Segment {
                name: pair[1].name.clone(),
                message: format!("`{}` has the same index", pair[0].name),
            });
        }
        // A segment is cut where the lowest later segment starts.
        let mut limit = u64::MAX;
        for slot in segments.iter_mut().rev() {
            slot.limit = limit;
            limit = limit.min(slot.id.first_seq);
        }
        Ok(LogIndex { segments })
    }

    /// Returns the index of the newest segment.
    pub(crate) fn last_index(&self) -> Option<u64> {
        self.segments.last().map(|slot| slot.id.index)
    }

    /// Returns true when the log has no segment.
    pub(crate) fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Returns true when a snapshot holds inputs that the log dropped
    /// later. The snapshot was taken after the input `seq - 1`, when
    /// `segment` was the newest segment.
    pub(crate) fn voids(&self, seq: u64, segment: u64) -> bool {
        self.segments.iter().any(|slot| slot.id.index > segment && slot.id.first_seq < seq)
    }

    /// Returns the segments that hold no valid record with the sequence
    /// number `seq` or a higher one. They are the first segments of the row.
    pub(crate) fn dead_before(&self, seq: u64) -> impl Iterator<Item = &Slot> {
        self.segments.iter().take_while(move |slot| slot.limit <= seq)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::MemStorage;

    fn log_of(segments: &[(u64, u64)]) -> LogIndex {
        let storage = MemStorage::new();
        for &(index, first_seq) in segments {
            let id = SegmentId { index, first_seq };
            storage.write(&id.name(), &[&id.header()]).unwrap();
        }
        storage.write("log/README.txt", &[b"not a segment"]).unwrap();
        LogIndex::load(&storage).unwrap()
    }

    #[test]
    fn a_later_segment_cuts_the_earlier_ones() {
        // Segment 2 starts again at 15: a rollback. It cuts both 0 and 1.
        let index = log_of(&[(0, 0), (1, 10), (2, 15), (3, 40)]);
        let limits: Vec<u64> = index.segments.iter().map(|s| s.limit).collect();
        assert_eq!(limits, [10, 15, 40, u64::MAX]);
        let index = log_of(&[(0, 0), (1, 10), (2, 4)]);
        let limits: Vec<u64> = index.segments.iter().map(|s| s.limit).collect();
        assert_eq!(limits, [4, 4, u64::MAX]);
        assert_eq!(index.last_index(), Some(2));
    }

    #[test]
    fn a_snapshot_is_void_when_a_later_segment_starts_before_it() {
        let index = log_of(&[(0, 0), (1, 10), (2, 15)]);
        // Taken at 20 while segment 1 was the newest; segment 2 went back
        // to 15.
        assert!(index.voids(20, 1));
        // The snapshot at 15 is where segment 2 starts: still good.
        assert!(!index.voids(15, 1));
        // Taken at 20 in the new history.
        assert!(!index.voids(20, 2));
    }

    #[test]
    fn dead_segments_are_those_wholly_before_a_sequence_number() {
        let index = log_of(&[(0, 0), (1, 10), (2, 20)]);
        let dead = |seq| index.dead_before(seq).map(|s| s.id.index).collect::<Vec<_>>();
        assert_eq!(dead(9), Vec::<u64>::new());
        assert_eq!(dead(10), [0]);
        assert_eq!(dead(25), [0, 1], "the last segment is never dead");
    }

    #[test]
    fn two_segments_with_one_index_are_an_error() {
        let storage = MemStorage::new();
        for first_seq in [0, 5] {
            let id = SegmentId { index: 1, first_seq };
            storage.write(&id.name(), &[&id.header()]).unwrap();
        }
        let error = LogIndex::load(&storage).unwrap_err();
        assert!(matches!(error, LogError::Segment { .. }), "{error}");
    }
}
