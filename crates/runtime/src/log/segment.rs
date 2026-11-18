//! Names and headers of log segments.

/// The directory of the segment files.
pub(crate) const LOG_DIR: &str = "log";

/// Marks the start of a segment file.
const MAGIC: &[u8; 8] = b"UNSUPLOG";

/// Version of the segment format.
const VERSION: u32 = 1;

/// Length of the header of a segment file in bytes.
pub(crate) const SEGMENT_HEADER: usize = 32;

/// What identifies a segment: its place in the row of segments and the
/// sequence number of its first record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SegmentId {
    /// Place in the row of segments, from 0.
    pub index: u64,
    /// Sequence number of the first record.
    pub first_seq: u64,
}

fn check_of(bytes: &[u8]) -> [u8; 4] {
    let hash = blake3::hash(bytes);
    let bytes = hash.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

impl SegmentId {
    /// Returns the storage name of the segment.
    pub(crate) fn name(&self) -> String {
        format!("{LOG_DIR}/{:010}-{:020}.seg", self.index, self.first_seq)
    }

    /// Reads a storage name. Returns `None` for a name that
    /// [`SegmentId::name`] does not make.
    pub(crate) fn parse(name: &str) -> Option<SegmentId> {
        let stem = name.strip_prefix(LOG_DIR)?.strip_prefix('/')?.strip_suffix(".seg")?;
        let (index, first_seq) = stem.split_once('-')?;
        let id = SegmentId { index: index.parse().ok()?, first_seq: first_seq.parse().ok()? };
        (id.name() == name).then_some(id)
    }

    /// Returns the header of the segment file.
    pub(crate) fn header(&self) -> [u8; SEGMENT_HEADER] {
        let mut header = [0; SEGMENT_HEADER];
        header[..8].copy_from_slice(MAGIC);
        header[8..12].copy_from_slice(&VERSION.to_le_bytes());
        header[12..20].copy_from_slice(&self.index.to_le_bytes());
        header[20..28].copy_from_slice(&self.first_seq.to_le_bytes());
        let check = check_of(&header[..28]);
        header[28..].copy_from_slice(&check);
        header
    }

    /// Checks that a segment file starts with the header of this segment.
    pub(crate) fn check_header(&self, bytes: &[u8]) -> Result<(), String> {
        let Some(header) = bytes.first_chunk::<SEGMENT_HEADER>() else {
            return Err("the file is shorter than a segment header".into());
        };
        if &header[..8] != MAGIC || header[28..] != check_of(&header[..28]) {
            return Err("the file does not start with a segment header".into());
        }
        let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
        if version != VERSION {
            return Err(format!(
                "the segment has format version {version}, this build reads version {VERSION}"
            ));
        }
        if *header != self.header() {
            return Err("the header does not match the name of the file".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_sort_like_indexes_and_read_back() {
        let a = SegmentId { index: 9, first_seq: 1_000 };
        let b = SegmentId { index: 10, first_seq: 5 };
        assert_eq!(a.name(), "log/0000000009-00000000000000001000.seg");
        assert!(a.name() < b.name());
        assert_eq!(SegmentId::parse(&a.name()), Some(a));
        assert_eq!(SegmentId::parse("log/9-1000.seg"), None, "not the canonical form");
        assert_eq!(SegmentId::parse("log/notes.txt"), None);
        assert_eq!(SegmentId::parse("snapshots/0000000009-00000000000000001000.seg"), None);
    }

    #[test]
    fn a_header_belongs_to_one_segment() {
        let id = SegmentId { index: 3, first_seq: 77 };
        let header = id.header();
        assert_eq!(id.check_header(&header), Ok(()));
        let other = SegmentId { index: 3, first_seq: 78 };
        assert_eq!(
            other.check_header(&header),
            Err("the header does not match the name of the file".into())
        );
        assert_eq!(
            id.check_header(&header[..20]),
            Err("the file is shorter than a segment header".into())
        );
        let mut damaged = header;
        damaged[15] ^= 1;
        assert_eq!(
            id.check_header(&damaged),
            Err("the file does not start with a segment header".into())
        );
    }
}
