//! Helpers that start a whole cluster in one process: the coordinator and
//! the workers are threads, and they talk over TCP on this machine.

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::thread::JoinHandle;

use serde::Deserialize;
use serde::de::{Deserializer, Error, SeqAccess, Visitor};
use serde_json::{Value, json};
use unsup_kernel::{RealmId, World, WorldConfig};
use unsup_protocol::{ActionStatus, Detail, ObserveOpts, ResidentId, ViewQuery, WorldView};
use unsup_runtime::cluster::{
    ClusterConfig, ClusterDurable, ClusterError, ClusterHandle, ClusterJoin, Coordinator, Worker,
    WorkerAbort, WorkerConfig, WorkerExit,
};
use unsup_runtime::{DurableConfig, MemStorage, Plugins, SimHandle, SyncPolicy};

use super::{act, hex};

/// The storage of every process of a cluster, in memory.
#[derive(Clone, Default)]
pub struct Disks {
    /// The storage of the coordinator.
    pub coordinator: MemStorage,
    /// The storage of each worker, by slot.
    pub workers: Vec<MemStorage>,
}

impl Disks {
    /// Makes empty storage for a coordinator and `workers` workers.
    pub fn new(workers: usize) -> Disks {
        Disks {
            coordinator: MemStorage::new(),
            workers: (0..workers).map(|_| MemStorage::new()).collect(),
        }
    }

    /// Returns what every disk holds after all processes were killed now.
    pub fn fork(&self) -> Disks {
        Disks {
            coordinator: self.coordinator.fork(),
            workers: self.workers.iter().map(MemStorage::fork).collect(),
        }
    }
}

/// How the processes of a test cluster write: every input is forced to
/// the disk, and a snapshot is taken every `snapshot_every` frames.
pub fn durable(storage: &MemStorage, snapshot_every: u64) -> ClusterDurable {
    ClusterDurable {
        storage: Arc::new(storage.clone()),
        config: DurableConfig {
            sync: SyncPolicy::EveryBatch,
            snapshot_every,
            ..DurableConfig::default()
        },
    }
}

/// The thread of one worker and what the worker ends with.
pub type WorkerThread = JoinHandle<Result<WorkerExit, ClusterError>>;

/// A cluster that runs in this process.
pub struct Running {
    /// The door to the cluster.
    pub handle: ClusterHandle,
    /// The owner of the coordinator thread.
    pub join: ClusterJoin,
    /// The worker threads, by slot.
    pub workers: Vec<WorkerThread>,
    /// Cuts the connection of a worker, by slot.
    pub aborts: Vec<WorkerAbort>,
}

/// What a test cluster is made of.
pub struct Setup {
    /// The plugins of every process.
    pub plugins: Plugins,
    /// The world of every process.
    pub world: WorldConfig,
    /// Number of workers.
    pub workers: usize,
    /// The storage, when the cluster records.
    pub disks: Option<Disks>,
    /// Frames between two snapshots.
    pub snapshot_every: u64,
}

impl Setup {
    /// Makes a setup without storage.
    pub fn new(plugins: Plugins, world: WorldConfig, workers: usize) -> Setup {
        Setup { plugins, world, workers, disks: None, snapshot_every: 0 }
    }

    /// Starts one worker thread for `slot`. The thread connects, hands out
    /// its abort handle through the channel and serves. The channel closes
    /// without a value when the coordinator refuses the worker.
    fn spawn_worker(
        &self,
        slot: u32,
        coordinator: SocketAddr,
    ) -> (WorkerThread, Receiver<WorkerAbort>) {
        let mut config = WorkerConfig::new(self.world.clone(), slot, coordinator);
        config.durable = self
            .disks
            .as_ref()
            .map(|disks| durable(&disks.workers[slot as usize], self.snapshot_every));
        let plugins = Arc::clone(&self.plugins);
        let (tx, rx) = channel();
        let thread = std::thread::spawn(move || {
            let worker = Worker::connect(plugins, config)?;
            // The receiver waits for this value or for the end of the
            // thread.
            let _ = tx.send(worker.abort_handle()?);
            worker.run()
        });
        (thread, rx)
    }

    /// Starts one worker thread for `slot` and waits until the coordinator
    /// has welcomed or refused it.
    pub fn worker(
        &self,
        slot: u32,
        coordinator: SocketAddr,
    ) -> (WorkerThread, Option<WorkerAbort>) {
        let (thread, abort) = self.spawn_worker(slot, coordinator);
        (thread, abort.recv().ok())
    }

    /// Returns the configuration of the coordinator.
    pub fn coordinator(&self) -> ClusterConfig {
        let mut config = ClusterConfig::manual(self.world.clone(), self.workers);
        config.durable =
            self.disks.as_ref().map(|disks| durable(&disks.coordinator, self.snapshot_every));
        config
    }

    /// Starts the coordinator and the workers.
    pub fn start(&self) -> Result<Running, ClusterError> {
        self.start_with(self.coordinator())
    }

    /// Starts the workers and a coordinator with the given configuration.
    pub fn start_with(&self, config: ClusterConfig) -> Result<Running, ClusterError> {
        let coordinator = Coordinator::bind("127.0.0.1:0")?;
        let address = coordinator.local_addr()?;
        // The socket listens already, so every worker connects at once.
        // The coordinator takes them in `start`.
        let pending: Vec<_> =
            (0..self.workers as u32).map(|slot| self.spawn_worker(slot, address)).collect();
        let started = coordinator.start(Arc::clone(&self.plugins), config);
        let (threads, aborts): (Vec<_>, Vec<_>) =
            pending.into_iter().map(|(thread, abort)| (thread, abort.recv().ok())).unzip();
        match started {
            Ok((handle, join)) => Ok(Running {
                handle,
                join,
                workers: threads,
                aborts: aborts.into_iter().flatten().collect(),
            }),
            Err(error) => {
                // The workers end when the coordinator closes their
                // connections.
                for thread in threads {
                    let _ = thread.join();
                }
                Err(error)
            }
        }
    }
}

impl Running {
    /// Stops the cluster in order. Returns the world of the global realm
    /// and the worlds of the zones, in zone order.
    pub fn stop(self) -> (World, Vec<World>) {
        let Running { handle, join, workers, aborts } = self;
        drop((handle, aborts));
        let global = join.shutdown().expect("the cluster stops in order");
        let mut zones = Vec::new();
        for worker in workers {
            let exit =
                worker.join().expect("the worker thread ran").expect("the worker stops in order");
            zones.extend(exit.worlds);
        }
        (global, zones)
    }
}

/// A byte string of MessagePack.
struct Bytes(Vec<u8>);

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Bytes, D::Error> {
        struct BytesVisitor;

        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = Bytes;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a byte string")
            }

            fn visit_bytes<E: Error>(self, bytes: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(bytes.to_vec()))
            }

            fn visit_byte_buf<E: Error>(self, bytes: Vec<u8>) -> Result<Bytes, E> {
                Ok(Bytes(bytes))
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Bytes, A::Error> {
                let mut bytes = Vec::new();
                while let Some(byte) = seq.next_element::<u8>()? {
                    bytes.push(byte);
                }
                Ok(Bytes(bytes))
            }
        }

        deserializer.deserialize_bytes(BytesVisitor)
    }
}

/// The state of one realm inside a snapshot of the kernel.
#[derive(Deserialize)]
struct RealmBlob {
    id: RealmId,
    data: Bytes,
}

/// The part of a snapshot of the kernel that this file reads.
#[derive(Deserialize)]
struct SnapshotRealms {
    realms: Vec<RealmBlob>,
}

/// Returns the hash of the state of every realm in a snapshot of the
/// kernel, as text.
///
/// A world that hosts one realm and a world that hosts all realms have
/// different state hashes also when the realm is in the same state: the
/// hash covers the list of hosted realms. The kernel has no call for the
/// hash of one realm. So this function reads the list `realms` of the
/// snapshot (`id` and `data` of each entry) and hashes the state bytes of
/// each realm. It is the one place that knows this part of the snapshot
/// layout, and it fails loudly when the layout changes.
pub fn realm_digests(snapshot: &[u8]) -> BTreeMap<RealmId, String> {
    let snapshot: SnapshotRealms =
        rmp_serde::from_slice(snapshot).expect("a snapshot has the list `realms`");
    assert!(!snapshot.realms.is_empty(), "the snapshot holds a realm");
    snapshot
        .realms
        .into_iter()
        .map(|realm| (realm.id, hex(blake3::hash(&realm.data.0).as_bytes())))
        .collect()
}

/// Returns the hash of the state of every realm of a world.
pub fn world_digests(world: &World) -> BTreeMap<RealmId, String> {
    realm_digests(&world.snapshot())
}

/// Returns the hash of the state of every realm of a cluster.
pub async fn cluster_digests(handle: &ClusterHandle) -> BTreeMap<RealmId, String> {
    let mut digests = BTreeMap::new();
    for (realm, snapshot) in handle.snapshots().await.expect("the snapshots of the cluster") {
        let one = realm_digests(&snapshot);
        assert_eq!(one.keys().copied().collect::<Vec<_>>(), [realm], "one realm per world");
        digests.extend(one);
    }
    digests
}

/// Returns what a resident sees with full detail: its own section, its
/// reports and its notices.
pub async fn look(handle: &SimHandle, resident: ResidentId) -> Value {
    let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
    let observation = handle.observe(resident, opts).await.expect("observe");
    json!({
        "self": observation.sections["self"],
        "reports": observation.sections["reports"],
        "notices": observation.notices,
    })
}

/// Sends an action that the world must accept.
pub async fn do_act(
    handle: &SimHandle,
    resident: ResidentId,
    verb: &str,
    args: Value,
) -> ActionStatus {
    let ack = handle.act(resident, act(verb, args)).await.expect("the world takes the action");
    assert!(ack.status.is_accepted(), "{verb}: {ack:?}");
    ack.status
}

/// Returns the number of residents of every zone, from the layout view.
pub async fn residents_per_zone(handle: &SimHandle) -> Vec<u32> {
    match handle.view(ViewQuery::Layout).await.expect("the layout") {
        WorldView::Layout { zones, .. } => zones.iter().map(|zone| zone.residents).collect(),
        other => panic!("not a layout: {other:?}"),
    }
}

/// Returns the kind and the sender of each notice of [`look`].
pub fn notices(seen: &Value) -> Vec<(String, Option<u64>)> {
    seen["notices"]
        .as_array()
        .expect("a list of notices")
        .iter()
        .map(|notice| {
            (notice["kind"].as_str().expect("a kind").to_string(), notice["from"].as_u64())
        })
        .collect()
}
