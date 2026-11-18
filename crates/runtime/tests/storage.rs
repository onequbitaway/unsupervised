//! The storage contract, for the file system and for memory.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use unsup_runtime::{Fault, FsStorage, MemStorage, Storage, StorageErrorKind};

/// Makes an empty directory for one test, below the target directory.
fn temp_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("storage-{name}"));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("the old test directory can be removed");
    }
    dir
}

/// What every storage must do.
fn contract(storage: &dyn Storage) {
    // Nothing is there at the start.
    assert_eq!(storage.list("log").expect("list"), Vec::<String>::new());
    assert_eq!(storage.read("log/a").expect_err("absent").kind, StorageErrorKind::NotFound);

    // A blob is written in parts and read back whole.
    storage.write("snapshots/one", &[b"hello ", b"world"]).expect("write");
    assert_eq!(storage.read("snapshots/one").expect("read"), b"hello world");
    assert_eq!(storage.read_head("snapshots/one", 5).expect("head"), b"hello");
    assert_eq!(storage.read_head("snapshots/one", 500).expect("head"), b"hello world");
    // A second write replaces the first.
    storage.write("snapshots/one", &[b"new"]).expect("write");
    assert_eq!(storage.read("snapshots/one").expect("read"), b"new");

    // A segment grows at its end, also on top of a blob.
    storage.write("log/b", &[b"head:"]).expect("write");
    let segment = storage.append("log/b").expect("append");
    segment.append(b"one,").expect("append");
    segment.append(b"two").expect("append");
    segment.sync().expect("sync");
    assert_eq!(storage.read("log/b").expect("read"), b"head:one,two");
    // A segment that does not exist starts empty.
    let fresh = storage.append("log/a").expect("append");
    assert_eq!(storage.read("log/a").expect("read"), b"");
    fresh.append(b"x").expect("append");
    assert_eq!(storage.read("log/a").expect("read"), b"x");

    // Names are listed per directory, sorted.
    assert_eq!(storage.list("log").expect("list"), ["log/a", "log/b"]);
    assert_eq!(storage.list("snapshots").expect("list"), ["snapshots/one"]);
    assert_eq!(storage.list("other").expect("list"), Vec::<String>::new());

    // Deleting works once and is harmless twice.
    storage.delete("log/a").expect("delete");
    storage.delete("log/a").expect("delete again");
    assert_eq!(storage.list("log").expect("list"), ["log/b"]);

    // A name cannot leave the storage or hide.
    for bad in ["", "../etc", "log/../../x", "/abs", "log//x", "log/.hidden", "log/a b", "log\\x"] {
        assert_eq!(
            storage.write(bad, &[b"x"]).expect_err(bad).kind,
            StorageErrorKind::InvalidName,
            "{bad:?}"
        );
        assert_eq!(storage.read(bad).expect_err(bad).kind, StorageErrorKind::InvalidName);
        assert_eq!(storage.delete(bad).expect_err(bad).kind, StorageErrorKind::InvalidName);
        assert!(storage.append(bad).is_err_and(|e| e.kind == StorageErrorKind::InvalidName));
    }
}

#[test]
fn memory_storage_keeps_the_contract() {
    contract(&MemStorage::new());
}

#[test]
fn file_storage_keeps_the_contract() {
    let dir = temp_dir("contract");
    contract(&FsStorage::open(&dir).expect("open"));
    // What was written is there for the next process.
    let again = FsStorage::open(&dir).expect("open again");
    assert_eq!(again.read("log/b").expect("read"), b"head:one,two");
    assert_eq!(again.root(), dir);
}

#[test]
fn file_storage_writes_through_a_temporary_file_and_leaves_none() {
    let dir = temp_dir("atomic");
    let storage = FsStorage::open(&dir).expect("open");
    storage.write("snapshots/one", &[b"first"]).expect("write");
    storage.write("snapshots/one", &[b"second"]).expect("write");
    let files: Vec<String> = std::fs::read_dir(dir.join("snapshots"))
        .expect("the directory exists")
        .map(|entry| entry.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(files, ["one"], "only the blob is in the directory");

    // A crash in the middle of a write leaves a temporary file. It is not a
    // blob, the old content is intact, and the next start removes it.
    let leftover = dir.join("snapshots").join(".tmp-999-0");
    std::fs::write(&leftover, b"half a snaps").expect("write the leftover");
    assert_eq!(storage.list("snapshots").expect("list"), ["snapshots/one"]);
    assert_eq!(storage.read("snapshots/one").expect("read"), b"second");
    drop(storage);
    let storage = FsStorage::open(&dir).expect("open again");
    assert!(!leftover.exists(), "the temporary file is removed at the start");
    assert_eq!(storage.read("snapshots/one").expect("read"), b"second");
}

#[test]
fn file_storage_has_one_owner_per_directory() {
    let dir = temp_dir("owner");
    let first = FsStorage::open(&dir).expect("open");
    first.write("snapshots/one", &[b"kept"]).expect("write");
    // A blob that the owner is writing: a temporary file.
    let in_progress = dir.join("snapshots").join(".tmp-1-0");
    std::fs::write(&in_progress, b"half").expect("write the temporary file");

    // A second owner is refused, and it removes nothing on its way out.
    let error = FsStorage::open(&dir).expect_err("the directory has an owner");
    assert_eq!(error.kind, StorageErrorKind::Io);
    assert!(error.message.starts_with("the directory is in use"), "{error}");
    assert!(in_progress.exists(), "the temporary file of the owner is intact");
    // The owner is not disturbed.
    first.write("snapshots/two", &[b"also kept"]).expect("write");

    // When the owner is gone, the directory is free. The lock file is not
    // a blob.
    drop(first);
    let second = FsStorage::open(&dir).expect("the directory is free again");
    assert_eq!(second.list("snapshots").expect("list"), ["snapshots/one", "snapshots/two"]);
    assert!(!in_progress.exists(), "the new owner removes what a dead owner left");
}

#[test]
fn file_storage_never_shows_half_a_blob() {
    const LONG: usize = 3 << 20;
    const SHORT: usize = 1 << 20;
    let dir = temp_dir("whole");
    let storage = Arc::new(FsStorage::open(&dir).expect("open"));
    // Two contents of different length, each one byte value all over: a
    // blob that is cut off or mixed shows at once.
    let long = vec![b'a'; LONG];
    let short = vec![b'b'; SHORT];
    storage.write("snapshots/blob", &[&long]).expect("write");

    // One thread reads the blob again and again while another replaces it.
    let done = Arc::new(AtomicBool::new(false));
    let reader = std::thread::spawn({
        let (storage, done) = (Arc::clone(&storage), Arc::clone(&done));
        move || {
            let mut reads = 0u32;
            loop {
                let last = done.load(Ordering::Acquire);
                let bytes = storage.read("snapshots/blob").expect("the blob is always there");
                let whole = match bytes.len() {
                    LONG => bytes.iter().all(|&byte| byte == b'a'),
                    SHORT => bytes.iter().all(|&byte| byte == b'b'),
                    _ => false,
                };
                assert!(whole, "a reader saw a blob of {} bytes that no writer wrote", bytes.len());
                reads += 1;
                if last {
                    return reads;
                }
            }
        }
    });
    for round in 0..24 {
        // Many parts, as for a snapshot: the write is not one system call.
        let content = if round % 2 == 0 { &short } else { &long };
        let parts: Vec<&[u8]> = content.chunks(64 << 10).collect();
        storage.write("snapshots/blob", &parts).expect("write");
    }
    done.store(true, Ordering::Release);
    let reads = reader.join().expect("every read gave a whole blob");
    assert!(reads >= 1);
    assert_eq!(storage.read("snapshots/blob").expect("read").len(), LONG, "the last write");
}

#[test]
fn file_storage_reports_what_the_system_refuses() {
    let dir = temp_dir("refused");
    let storage = FsStorage::open(&dir).expect("open");
    // A directory is in the way of the blob.
    std::fs::create_dir_all(dir.join("log").join("taken")).expect("mkdir");
    let error = storage.write("log/taken", &[b"x"]).expect_err("a directory is in the way");
    assert_eq!((error.kind, error.name.as_str()), (StorageErrorKind::Io, "log/taken"));
    assert!(
        error.to_string().starts_with("storage `log/taken`: cannot write the file:"),
        "{error}"
    );
    // The failed write left no temporary file.
    assert_eq!(storage.list("log").expect("list"), Vec::<String>::new());
    let names: Vec<_> = std::fs::read_dir(dir.join("log")).expect("dir").collect();
    assert_eq!(names.len(), 1, "only the directory that was in the way");
}

#[test]
fn memory_storage_shows_what_a_crash_leaves() {
    let storage = MemStorage::new();
    storage.write("snapshots/one", &[b"blob"]).expect("write");
    let segment = storage.append("log/a").expect("append");
    segment.append(b"synced").expect("append");
    segment.sync().expect("sync");
    segment.append(b"+not synced").expect("append");

    // A second handle sees the same data.
    let shared: Arc<dyn Storage> = Arc::new(storage.clone());
    assert_eq!(shared.read("log/a").expect("read"), b"synced+not synced");

    // The process was killed: everything that was written is there.
    let killed = storage.fork();
    assert_eq!(killed.read("log/a").expect("read"), b"synced+not synced");
    // The power went off: only what was forced to the disk is there. A blob
    // is always forced.
    let dark = storage.fork_durable();
    assert_eq!(dark.read("log/a").expect("read"), b"synced");
    assert_eq!(dark.read("snapshots/one").expect("read"), b"blob");

    // A fork is a copy: later writes do not reach it.
    segment.append(b"!").expect("append");
    storage.delete("snapshots/one").expect("delete");
    assert_eq!(killed.read("log/a").expect("read"), b"synced+not synced");
    assert_eq!(killed.read("snapshots/one").expect("read"), b"blob");
    assert_eq!(storage.total_bytes(), "synced+not synced!".len());
}

#[test]
fn memory_storage_imitates_failures() {
    let storage = MemStorage::new();
    let segment = storage.append("log/a").expect("append");
    segment.append(b"good").expect("append");

    storage.set_fault(Some(Fault::Append));
    let error = segment.append(b"12345678").expect_err("the fault is on");
    assert_eq!(error.kind, StorageErrorKind::Io);
    // Half of the bytes reached the segment, as on a disk that fills up.
    assert_eq!(storage.read("log/a").expect("read"), b"good1234");
    segment.sync().expect("only appends fail");

    storage.set_fault(Some(Fault::Sync));
    segment.append(b"+").expect("only syncs fail");
    assert_eq!(segment.sync().expect_err("the fault is on").kind, StorageErrorKind::Io);
    assert_eq!(storage.fork_durable().read("log/a").expect("read"), b"good1234");

    storage.set_fault(Some(Fault::Write));
    assert_eq!(
        storage.write("snapshots/one", &[b"x"]).expect_err("the fault is on").kind,
        StorageErrorKind::Io
    );
    assert_eq!(storage.list("snapshots").expect("list"), Vec::<String>::new());

    storage.set_fault(None);
    storage.write("snapshots/one", &[b"x"]).expect("write");
    segment.sync().expect("sync");
    assert_eq!(storage.fork_durable().read("log/a").expect("read"), b"good1234+");
}
