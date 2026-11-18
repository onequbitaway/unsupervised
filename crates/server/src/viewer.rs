//! The viewer service of a server: its routes, and the storage for the
//! life histories and the pinned clips.
//!
//! The spectator crate keeps the chronicle and the pinned clips on a
//! storage, so that they are there again after a restart. The server
//! gives it a storage in the same place as the world: in memory, or in
//! the directory `spectator` next to the world directory. A new world
//! starts with an empty storage: the history of an earlier world is not
//! its history.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use axum::Router;
use tokio::sync::oneshot;
use unsup_runtime::SimHandle;
use unsup_runtime::storage::{FsStorage, MemStorage, Segment, Storage, StorageError};

use crate::config::WorldStore;

/// Name of the directory of the viewer service, next to the world
/// directory.
const SPECTATOR_DIR: &str = "spectator";
/// The directories that the spectator crate uses on its storage: the
/// journal of the chronicle and the pinned clips
/// (`unsup_spectator::router_with_storage`).
const SPECTATOR_NAMES: [&str; 2] = ["chronicle", "clips"];

/// Returns the directory of the viewer service for a world directory.
pub(crate) fn directory(world_dir: &Path) -> PathBuf {
    world_dir.parent().unwrap_or(Path::new("")).join(SPECTATOR_DIR)
}

/// A storage that tells the server when its user let go of it.
///
/// The viewer service writes the chronicle one last time when the
/// simulation stops, in a task of its own. The server waits for the end
/// of that task before it says that it stopped: the task drops the last
/// reference to the storage, and the sender goes with it.
struct Lent {
    inner: Arc<dyn Storage>,
    _returned: oneshot::Sender<()>,
}

impl Storage for Lent {
    fn list(&self, dir: &str) -> Result<Vec<String>, StorageError> {
        self.inner.list(dir)
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        self.inner.read(name)
    }

    fn read_head(&self, name: &str, len: usize) -> Result<Vec<u8>, StorageError> {
        self.inner.read_head(name, len)
    }

    fn write(&self, name: &str, parts: &[&[u8]]) -> Result<(), StorageError> {
        self.inner.write(name, parts)
    }

    fn delete(&self, name: &str) -> Result<(), StorageError> {
        self.inner.delete(name)
    }

    fn append(&self, name: &str) -> Result<Arc<dyn Segment>, StorageError> {
        self.inner.append(name)
    }
}

/// The storage of the viewer service, before the service has it.
pub(crate) struct ViewerStorage {
    storage: Arc<dyn Storage>,
    /// True for a directory: the server waits at its stop until the
    /// service let go of it.
    on_disk: bool,
}

/// Resolves when the viewer service let go of its storage: its last write
/// is done, and the lock of its directory is free.
pub(crate) type Returned = oneshot::Receiver<()>;

impl ViewerStorage {
    /// Opens the storage of the viewer service where the world is. With
    /// `new_world`, what an earlier world left there is removed.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be opened or is in use by another
    /// server, and when the data of an earlier world cannot be removed.
    pub(crate) fn open(world: &WorldStore, new_world: bool) -> anyhow::Result<ViewerStorage> {
        let WorldStore::Dir(world_dir) = world else {
            return Ok(ViewerStorage { storage: Arc::new(MemStorage::new()), on_disk: false });
        };
        let dir = directory(world_dir);
        let storage = FsStorage::open(dir.clone()).map_err(|error| {
            anyhow::anyhow!(
                "cannot open the directory {} of the viewer service: {error}",
                dir.display()
            )
        })?;
        if new_world {
            let removed = clear(&storage).with_context(|| {
                format!("cannot remove the history of an earlier world from {}", dir.display())
            })?;
            if removed > 0 {
                tracing::info!(files = removed, "history of an earlier world removed");
            }
        }
        Ok(ViewerStorage { storage: Arc::new(storage), on_disk: true })
    }

    /// Starts the viewer service on the storage and returns its routes.
    /// With a directory, it also returns the signal for the stop of the
    /// server.
    ///
    /// # Errors
    ///
    /// Fails when the spectator crate refuses the configuration.
    pub(crate) fn serve(
        self,
        sim: SimHandle,
        config: unsup_spectator::Config,
    ) -> anyhow::Result<(Router, Option<Returned>)> {
        let (returned, signal) = oneshot::channel();
        let lent = Arc::new(Lent { inner: self.storage, _returned: returned });
        let router = unsup_spectator::router_with_storage(sim, config, lent)
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok((router, self.on_disk.then_some(signal)))
    }
}

/// Removes everything that the spectator crate keeps on a storage.
/// Returns the number of files that it removed.
fn clear(storage: &dyn Storage) -> Result<usize, StorageError> {
    let mut removed = 0;
    for dir in SPECTATOR_NAMES {
        for name in storage.list(dir)? {
            storage.delete(&name)?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_directory_of_the_viewer_service_is_next_to_the_world() {
        assert_eq!(directory(Path::new("/srv/data/world")), PathBuf::from("/srv/data/spectator"));
        assert_eq!(directory(Path::new("var/world")), PathBuf::from("var/spectator"));
        assert_eq!(directory(Path::new("world")), PathBuf::from("spectator"));
    }

    #[test]
    fn clearing_removes_the_chronicle_and_the_clips_and_nothing_else() {
        let storage = MemStorage::new();
        for name in ["chronicle/journal-0000000001.jsonl", "clips/120-3.json", "other/keep.txt"] {
            storage.write(name, &[b"x"]).expect("the file is written");
        }
        assert_eq!(clear(&storage), Ok(2));
        assert_eq!(storage.list("chronicle"), Ok(Vec::new()));
        assert_eq!(storage.list("clips"), Ok(Vec::new()));
        assert_eq!(storage.list("other"), Ok(vec!["other/keep.txt".to_string()]));
    }
}
