//! One storage for a process, one directory in it for each realm.

use std::sync::Arc;

use unsup_protocol::RealmId;

use crate::storage::{Segment, Storage, StorageError};

/// Returns the directory of a realm in the storage of its process.
pub(crate) fn realm_dir(realm: RealmId) -> String {
    match realm {
        RealmId::Global => "global".to_string(),
        RealmId::Zone(zone) => format!("zone-{:05}", zone.0),
    }
}

/// A view of a storage that puts every name below one directory.
pub(crate) struct Scoped {
    inner: Arc<dyn Storage>,
    /// The directory, with the `/` at its end.
    prefix: String,
}

impl Scoped {
    /// Makes the storage of one realm inside the storage of a process.
    pub(crate) fn of_realm(inner: Arc<dyn Storage>, realm: RealmId) -> Arc<dyn Storage> {
        Arc::new(Scoped { inner, prefix: format!("{}/", realm_dir(realm)) })
    }

    fn full(&self, name: &str) -> String {
        format!("{}{name}", self.prefix)
    }
}

impl Storage for Scoped {
    fn list(&self, dir: &str) -> Result<Vec<String>, StorageError> {
        let names = self.inner.list(&self.full(dir))?;
        Ok(names
            .into_iter()
            .filter_map(|name| name.strip_prefix(&self.prefix).map(str::to_string))
            .collect())
    }

    fn read(&self, name: &str) -> Result<Vec<u8>, StorageError> {
        self.inner.read(&self.full(name))
    }

    fn read_head(&self, name: &str, len: usize) -> Result<Vec<u8>, StorageError> {
        self.inner.read_head(&self.full(name), len)
    }

    fn write(&self, name: &str, parts: &[&[u8]]) -> Result<(), StorageError> {
        self.inner.write(&self.full(name), parts)
    }

    fn delete(&self, name: &str) -> Result<(), StorageError> {
        self.inner.delete(&self.full(name))
    }

    fn append(&self, name: &str) -> Result<Arc<dyn Segment>, StorageError> {
        self.inner.append(&self.full(name))
    }
}

#[cfg(test)]
mod tests {
    use unsup_protocol::ZoneId;

    use super::*;
    use crate::storage::MemStorage;

    #[test]
    fn each_realm_has_its_own_names() {
        let disk = Arc::new(MemStorage::new());
        let global = Scoped::of_realm(disk.clone(), RealmId::Global);
        let zone = Scoped::of_realm(disk.clone(), RealmId::Zone(ZoneId(7)));
        global.write("log/a.seg", &[b"g"]).unwrap();
        zone.write("log/a.seg", &[b"z"]).unwrap();
        zone.append("log/b.seg").unwrap().append(b"more").unwrap();

        assert_eq!(global.read("log/a.seg").unwrap(), b"g");
        assert_eq!(zone.read("log/a.seg").unwrap(), b"z");
        assert_eq!(zone.read_head("log/b.seg", 2).unwrap(), b"mo");
        assert_eq!(global.list("log").unwrap(), ["log/a.seg"]);
        assert_eq!(zone.list("log").unwrap(), ["log/a.seg", "log/b.seg"]);
        // The process sees both directories.
        assert_eq!(disk.list("global/log").unwrap(), ["global/log/a.seg"]);
        assert_eq!(disk.read("zone-00007/log/a.seg").unwrap(), b"z");

        zone.delete("log/a.seg").unwrap();
        assert_eq!(zone.list("log").unwrap(), ["log/b.seg"]);
        assert_eq!(global.list("log").unwrap(), ["log/a.seg"]);
    }
}
