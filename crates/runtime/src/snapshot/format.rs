//! Names and headers of snapshot files.

/// The directory of the snapshot files.
pub(crate) const SNAPSHOT_DIR: &str = "snapshots";

/// Marks the start of a snapshot file.
const MAGIC: &[u8; 8] = b"UNSUPSNP";

/// Version of the snapshot file format. The format of the payload has its
/// own version inside the kernel.
const VERSION: u32 = 1;

/// Length of the header of a snapshot file in bytes.
pub(crate) const SNAPSHOT_HEADER: usize = 112;

/// What a snapshot file says about its content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// Number of the last frame of the world.
    pub frame: u64,
    /// Number of inputs that the world had applied. A recovery replays the
    /// log from this sequence number.
    pub seq: u64,
    /// Index of the newest log segment when the snapshot was taken.
    pub segment: u64,
    /// `World::state_hash` when the snapshot was taken.
    pub state_hash: [u8; 32],
}

/// Returns the storage name of the snapshot after input `seq - 1`, at
/// `frame`.
pub(crate) fn name(seq: u64, frame: u64) -> String {
    format!("{SNAPSHOT_DIR}/{seq:020}-{frame:020}.snap")
}

/// Reads a storage name. Returns the sequence number and the frame, or
/// `None` for a name that [`name`] does not make.
pub(crate) fn parse_name(text: &str) -> Option<(u64, u64)> {
    let stem = text.strip_prefix(SNAPSHOT_DIR)?.strip_prefix('/')?.strip_suffix(".snap")?;
    let (seq, frame) = stem.split_once('-')?;
    let (seq, frame) = (seq.parse().ok()?, frame.parse().ok()?);
    (name(seq, frame) == text).then_some((seq, frame))
}

fn check_of(bytes: &[u8]) -> [u8; 4] {
    let hash = blake3::hash(bytes);
    let bytes = hash.as_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

/// Returns the header for a payload.
pub(crate) fn encode_header(meta: &SnapshotMeta, payload: &[u8]) -> [u8; SNAPSHOT_HEADER] {
    let mut header = [0; SNAPSHOT_HEADER];
    header[..8].copy_from_slice(MAGIC);
    header[8..12].copy_from_slice(&VERSION.to_le_bytes());
    header[12..20].copy_from_slice(&meta.frame.to_le_bytes());
    header[20..28].copy_from_slice(&meta.seq.to_le_bytes());
    header[28..36].copy_from_slice(&meta.segment.to_le_bytes());
    header[36..68].copy_from_slice(&meta.state_hash);
    header[68..76].copy_from_slice(&(payload.len() as u64).to_le_bytes());
    header[76..108].copy_from_slice(blake3::hash(payload).as_bytes());
    let check = check_of(&header[..108]);
    header[108..].copy_from_slice(&check);
    header
}

fn u64_at(bytes: &[u8; SNAPSHOT_HEADER], at: usize) -> u64 {
    let mut number = [0; 8];
    number.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(number)
}

/// The header of a snapshot file, as it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
    pub meta: SnapshotMeta,
    pub payload_len: u64,
    pub payload_hash: [u8; 32],
}

/// Reads the header at the start of a snapshot file.
pub(crate) fn decode_header(bytes: &[u8]) -> Result<Header, String> {
    let Some(header) = bytes.first_chunk::<SNAPSHOT_HEADER>() else {
        return Err("the file is shorter than a snapshot header".into());
    };
    if &header[..8] != MAGIC || header[108..] != check_of(&header[..108]) {
        return Err("the file does not start with a snapshot header".into());
    }
    let version = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if version != VERSION {
        return Err(format!(
            "the snapshot file has format version {version}, this build reads version {VERSION}"
        ));
    }
    let mut state_hash = [0; 32];
    state_hash.copy_from_slice(&header[36..68]);
    let mut payload_hash = [0; 32];
    payload_hash.copy_from_slice(&header[76..108]);
    Ok(Header {
        meta: SnapshotMeta {
            frame: u64_at(header, 12),
            seq: u64_at(header, 20),
            segment: u64_at(header, 28),
            state_hash,
        },
        payload_len: u64_at(header, 68),
        payload_hash,
    })
}

/// Checks the payload of a snapshot file against its header.
pub(crate) fn check_payload(header: &Header, payload: &[u8]) -> Result<(), String> {
    if payload.len() as u64 != header.payload_len {
        return Err(format!(
            "the payload has {} bytes, the header says {}",
            payload.len(),
            header.payload_len
        ));
    }
    if blake3::hash(payload).as_bytes() != &header.payload_hash {
        return Err("the checksum of the payload is wrong".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> SnapshotMeta {
        SnapshotMeta { frame: 120, seq: 4_321, segment: 7, state_hash: [0xab; 32] }
    }

    #[test]
    fn names_sort_like_sequence_numbers_and_read_back() {
        assert_eq!(name(5, 2), "snapshots/00000000000000000005-00000000000000000002.snap");
        assert!(name(9, 100) < name(10, 1));
        assert_eq!(parse_name(&name(4_321, 120)), Some((4_321, 120)));
        assert_eq!(parse_name("snapshots/5-2.snap"), None, "not the canonical form");
        assert_eq!(parse_name("snapshots/latest"), None);
    }

    #[test]
    fn a_header_reads_back_and_guards_the_payload() {
        let payload = b"the state of the world";
        let header = encode_header(&meta(), payload);
        let read = decode_header(&header).unwrap();
        assert_eq!(read.meta, meta());
        assert_eq!(check_payload(&read, payload), Ok(()));
        assert_eq!(
            check_payload(&read, b"the state of the w0rld"),
            Err("the checksum of the payload is wrong".into())
        );
        assert_eq!(
            check_payload(&read, b"short"),
            Err("the payload has 5 bytes, the header says 22".into())
        );
    }

    #[test]
    fn a_damaged_header_is_refused() {
        let header = encode_header(&meta(), b"x");
        for position in 0..SNAPSHOT_HEADER {
            let mut damaged = header;
            damaged[position] ^= 0x04;
            assert!(decode_header(&damaged).is_err(), "flip at {position}");
        }
        assert_eq!(
            decode_header(&header[..50]),
            Err("the file is shorter than a snapshot header".into())
        );
    }
}
