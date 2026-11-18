//! Storage in a directory of the local file system.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::{Segment, Storage, StorageError, StorageErrorKind, check_name};

/// Start of the name of a file that is being written. A storage name cannot
/// start with a dot, so such a file never hides a blob.
const TEMP_PREFIX: &str = ".tmp-";

/// Name of the file that marks a directory as in use. A storage name cannot
/// start with a dot, so the file is not a blob.
const LOCK_FILE: &str = ".lock";

/// Storage in a directory of the local file system.
///
/// A blob is written to a temporary file in the directory of its name,
/// forced to the disk and renamed over the name. Then the directory is
/// forced to the disk, so that the new name survives a loss of power.
/// Temporary files that a crash left behind are removed when the storage
/// is opened.
///
/// One `FsStorage` owns its directory. It holds a lock on the file `.lock`
/// in the directory until it is dropped, and [`FsStorage::open`] fails
/// while another process, or another value in this process, holds that
/// lock. Two writers in one directory would overwrite the log segments of
/// each other.
#[derive(Debug)]
pub struct FsStorage {
    root: PathBuf,
    temp: AtomicU64,
    /// Holds the lock on the directory. The operating system releases the
    /// lock when the file is closed, also when the process dies.
    _lock: File,
}

impl FsStorage {
    /// Opens a directory as storage. It makes the directory when it does
    /// not exist.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be made or read, and when another
    /// `FsStorage` has the directory open, in this process or in another
    /// one.
    pub fn open(root: impl Into<PathBuf>) -> Result<FsStorage, StorageError> {
        let root = root.into();
        let shown = root.display().to_string();
        fs::create_dir_all(&root)
            .map_err(|e| StorageError::io(&shown, "create the directory", &e))?;
        let lock = lock_directory(&root, &shown)?;
        // Only the owner of the directory removes temporary files: one of
        // them can be a blob that another owner is writing.
        remove_temporary_files(&root)
            .map_err(|e| StorageError::io(&shown, "remove temporary files", &e))?;
        Ok(FsStorage { root, temp: AtomicU64::new(0), _lock: lock })
    }

    /// Returns the directory of the storage.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, name: &str) -> Result<PathBuf, StorageError> {
        check_name(name)?;
        Ok(self.root.join(name))
    }
}

/// Takes the lock on a directory. The lock lasts as long as the file that
/// is returned.
fn lock_directory(root: &Path, shown: &str) -> Result<File, StorageError> {
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(LOCK_FILE))
        .map_err(|e| StorageError::io(shown, "open the lock file", &e))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(StorageError::new(
            StorageErrorKind::Io,
            shown,
            "the directory is in use: another process or another storage value has it open",
        )),
        Err(TryLockError::Error(e)) => Err(StorageError::io(shown, "lock the directory", &e)),
    }
}

/// Removes the temporary files below a directory.
fn remove_temporary_files(dir: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_dir() {
            remove_temporary_files(&entry.path())?;
        } else if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Forces a directory to the disk, so that a new or renamed entry survives a
/// loss of power.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    // A directory can be opened and forced to the disk on Unix. Other
    // systems write the entry with the file.
    if cfg!(unix) { File::open(dir)?.sync_all() } else { Ok(()) }
}

fn parent(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("."))
}

impl Storage for FsStorage {
    fn list(&self, dir: &str) -> Result<Vec<String>, StorageError> {
        let path = self.path(dir)?;
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StorageError::io(dir, "list the directory", &e)),
        };
        let mut names = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| StorageError::io(dir, "list the directory", &e))?;
            let is_file = entry
                .file_type()
                .map_err(|e| StorageError::io(dir, "list the directory", &e))?
                .is_file();
            let file = entry.file_name().to_string_lossy().into_owned();
            if is_file && !file.starts_with('.') {
                names.push(format!("{dir}/{file}"));
            }
        }
        names.sort_unstable();
        Ok(names)
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        fs::read(self.path(name)?).map_err(|e| StorageError::io(name, "read the file", &e))
    }

    fn read_head(&self, name: &str, len: usize) -> Result<Vec<u8>, StorageError> {
        let file = File::open(self.path(name)?)
            .map_err(|e| StorageError::io(name, "open the file", &e))?;
        let mut head = Vec::new();
        file.take(len as u64)
            .read_to_end(&mut head)
            .map_err(|e| StorageError::io(name, "read the file", &e))?;
        Ok(head)
    }

    fn write(&self, name: &str, parts: &[&[u8]]) -> Result<(), StorageError> {
        let path = self.path(name)?;
        let dir = parent(&path);
        fs::create_dir_all(dir).map_err(|e| StorageError::io(name, "create the directory", &e))?;
        let n = self.temp.fetch_add(1, Ordering::Relaxed);
        let temp = dir.join(format!("{TEMP_PREFIX}{}-{n}", std::process::id()));
        let written = (|| {
            let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
            for part in parts {
                file.write_all(part)?;
            }
            file.sync_all()?;
            fs::rename(&temp, &path)?;
            sync_dir(dir)
        })();
        written.map_err(|e| {
            // The temporary file is of no use. A failure to remove it is
            // not a second error: `open` removes what is left.
            let _ = fs::remove_file(&temp);
            StorageError::io(name, "write the file", &e)
        })
    }

    fn delete(&self, name: &str) -> Result<(), StorageError> {
        match fs::remove_file(self.path(name)?) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::io(name, "delete the file", &e)),
        }
    }

    fn append(&self, name: &str) -> Result<Arc<dyn Segment>, StorageError> {
        let path = self.path(name)?;
        let dir = parent(&path);
        let opened = (|| {
            fs::create_dir_all(dir)?;
            let existed = path.exists();
            let file = OpenOptions::new().append(true).create(true).open(&path)?;
            if !existed {
                sync_dir(dir)?;
            }
            Ok::<File, std::io::Error>(file)
        })();
        let file = opened.map_err(|e| StorageError::io(name, "open the segment", &e))?;
        Ok(Arc::new(FsSegment { file, name: name.to_string() }))
    }
}

/// A segment file that is open for appending.
struct FsSegment {
    file: File,
    name: String,
}

impl Segment for FsSegment {
    fn append(&self, bytes: &[u8]) -> Result<(), StorageError> {
        // The file is in append mode: the operating system puts every write
        // at the end, also when another handle wrote in between.
        (&self.file)
            .write_all(bytes)
            .map_err(|e| StorageError::io(&self.name, "append to the segment", &e))
    }

    fn sync(&self) -> Result<(), StorageError> {
        self.file.sync_data().map_err(|e| StorageError::io(&self.name, "sync the segment", &e))
    }
}
