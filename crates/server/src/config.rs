//! The configuration of the server.
//!
//! The values come from four places. A later place replaces an earlier
//! one:
//!
//! 1. `data/defaults.toml`, which is part of the binary;
//! 2. a configuration file in TOML with the same shape;
//! 3. environment variables (`UNSUPERVISED_BIND` and others);
//! 4. flags of the command line.
//!
//! The development mode sits between 2 and 3: it applies the `[dev]` table.
//!
//! The data directory holds three things: the store of accounts and keys
//! (one SQLite file), the world (the directory `world` with the input log
//! and the snapshots) and what the viewer service keeps (the directory
//! `spectator` with the life histories and the pinned clips). The third
//! is always next to the second: see [`WorldStore`].

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use unsup_gateway::{GatewayConfig, merge_tables};
use unsup_runtime::{DurableConfig, SyncPolicy};

use crate::sponsor::SponsorConfig;
use crate::world::WorldShape;

/// The defaults of the server, as TOML text.
pub const DEFAULTS: &str = include_str!("../data/defaults.toml");

/// Name of the SQLite file in the data directory.
const STORE_FILE: &str = "store.sqlite3";
/// Name of the directory of the world in the data directory.
const WORLD_DIR: &str = "world";

/// Which storage engine keeps accounts and keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StoreKind {
    /// One SQLite file in the data directory.
    Sqlite,
    /// Memory only. Everything is lost when the server stops.
    Memory,
}

impl StoreKind {
    const fn as_str(self) -> &'static str {
        match self {
            StoreKind::Sqlite => "sqlite",
            StoreKind::Memory => "memory",
        }
    }
}

/// Where the world lives between two runs: the name in a file or a flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorldStoreKind {
    /// The directory `world` in the data directory.
    Disk,
    /// Memory only. The world is lost when the server stops.
    Memory,
}

impl WorldStoreKind {
    const fn as_str(self) -> &'static str {
        match self {
            WorldStoreKind::Disk => "disk",
            WorldStoreKind::Memory => "memory",
        }
    }
}

/// Where the world lives between two runs of the server.
///
/// In both cases the world has an input log and snapshots, so an
/// administrator can roll it back.
///
/// The viewer service keeps the life histories and the pinned clips of
/// the world in the same place: in memory, or in the directory
/// `spectator` next to the world directory. So they come back with the
/// world after a restart. A new world starts without them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldStore {
    /// Memory only. Each start makes a new world.
    Memory,
    /// This directory. The server makes it when it does not exist. When
    /// it holds a world, the server recovers that world at the start.
    Dir(PathBuf),
}

/// How the server keeps the world: the input log, the snapshots, time
/// dilation and the audit. `None` takes the number of the runtime
/// (`crates/runtime/data/runtime.toml`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Durability {
    /// When the input log is forced to the disk.
    pub sync: Option<SyncPolicy>,
    /// Frames between two snapshots.
    pub snapshot_every: Option<u64>,
    /// Number of snapshots to keep. A rollback can go to each of them.
    pub keep_snapshots: Option<usize>,
    /// A log segment rolls when it is larger than this many bytes.
    pub segment_bytes: Option<u64>,
    /// True: a zone whose tick takes too long skips frames.
    pub dilation: bool,
    /// Frames between two audits of the money and the goods of the whole
    /// world. A failed audit is an alert. 0: no audit while the server
    /// runs.
    pub audit_every: u64,
    /// For an operator, after a start failed because one input of the
    /// log makes the simulation panic: the recovery stops before the
    /// input with this sequence number, and that input and all later
    /// ones leave the log.
    pub drop_from: Option<u64>,
}

impl Durability {
    /// Returns the configuration of the durable journal of the runtime.
    #[must_use]
    pub fn to_runtime(&self) -> DurableConfig {
        let base = DurableConfig::default();
        DurableConfig {
            sync: self.sync.unwrap_or(base.sync),
            segment_bytes: self.segment_bytes.unwrap_or(base.segment_bytes),
            snapshot_every: self.snapshot_every.unwrap_or(base.snapshot_every),
            keep_snapshots: self.keep_snapshots.unwrap_or(base.keep_snapshots),
            drop_from: self.drop_from,
            ..base
        }
    }
}

/// Where accounts and keys live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreConfig {
    /// Memory only. Everything is lost when the server stops.
    Memory,
    /// The SQLite file at this path. The server makes the file and its
    /// directory when they do not exist.
    Sqlite(PathBuf),
}

/// Configuration of a server.
///
/// `ServerConfig::default()` has the values of `data/defaults.toml`: it
/// keeps accounts in a SQLite file and the world in a directory, both
/// under `var/`. Tests and tools start from [`ServerConfig::dev`], which
/// keeps everything in memory:
///
/// ```
/// use unsup_server::ServerConfig;
///
/// let config = ServerConfig {
///     bind: "127.0.0.1:0".parse().unwrap(),
///     web_dir: None,
///     ..ServerConfig::dev()
/// };
/// assert!(config.dev);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct ServerConfig {
    /// Address to listen on. Port 0 picks a free port.
    pub bind: SocketAddr,
    /// Real time between two frames. Zero means manual stepping.
    pub tick_interval: Duration,
    /// Shape of the world.
    pub world: WorldShape,
    /// Directory with the built viewer. It is served at `/` when it exists.
    pub web_dir: Option<PathBuf>,
    /// Where accounts and keys live.
    pub store: StoreConfig,
    /// Where the world lives. With accounts in memory the world is in
    /// memory too, whatever this says: after a restart nobody would have
    /// a key for the residents of a world on the disk.
    pub world_store: WorldStore,
    /// How the server keeps the world.
    pub durability: Durability,
    /// Files with parameter overrides: TOML in the format of the data
    /// files of the modules. The server gives them to the kernel, in this
    /// order, when it makes a new world. A world that the server recovers
    /// keeps the parameters that it has.
    pub params: Vec<PathBuf>,
    /// Limits and options of the gateway.
    pub gateway: GatewayConfig,
    /// The configuration of the viewer service: the limits of the feed,
    /// the delay of the stream, the director. The `[spectator]` table of
    /// the configuration changes the values that it names
    /// (`unsup_spectator::Config::with_overrides`).
    pub spectator: unsup_spectator::Config,
    /// How the server takes missions from human sponsors.
    pub sponsors: SponsorConfig,
    /// Development mode: the server makes a ready owner, and an admin
    /// token when the configuration has none, and returns both in
    /// [`Server::dev`](crate::Server::dev).
    pub dev: bool,
    /// Time that open connections get to end when the server stops.
    pub shutdown_grace: Duration,
}

/// Values from the environment and the command line. `None` keeps the
/// value of the file or the default.
#[derive(Clone, Debug, Default)]
pub struct Overrides {
    /// Development mode.
    pub dev: bool,
    /// Address to listen on.
    pub bind: Option<SocketAddr>,
    /// Real milliseconds for one game minute.
    pub tick_ms: Option<u64>,
    /// Seed of the world.
    pub seed: Option<u64>,
    /// Number of cities.
    pub cities: Option<u32>,
    /// Districts on one side of a city.
    pub districts: Option<u32>,
    /// Directory with the built viewer.
    pub web_dir: Option<PathBuf>,
    /// Directory for the data that the server writes.
    pub data_dir: Option<PathBuf>,
    /// The storage engine.
    pub store: Option<StoreKind>,
    /// Where the world lives.
    pub world_store: Option<WorldStoreKind>,
    /// Files with parameter overrides. An empty list keeps the list of
    /// the configuration file.
    pub params: Vec<PathBuf>,
    /// The first input of the log that a recovery drops.
    pub drop_from: Option<u64>,
    /// The file that `GET /join.md` serves.
    pub join_file: Option<PathBuf>,
    /// The token of the administrator.
    pub admin_token: Option<String>,
    /// The client id of the GitHub sign-in.
    pub github_client_id: Option<String>,
    /// The client secret of the GitHub sign-in.
    pub github_client_secret: Option<String>,
}

/// The shape of the configuration file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: SocketAddr,
    tick_ms: u64,
    data_dir: PathBuf,
    store: StoreKind,
    world_store: WorldStoreKind,
    params: Vec<PathBuf>,
    web_dir: PathBuf,
    join_file: PathBuf,
    shutdown_grace_ms: u64,
    world: WorldFile,
    durability: DurabilityFile,
    sponsors: SponsorConfig,
    spectator: toml::Table,
    gateway: toml::Table,
}

/// When the input log is forced to the disk: the name in a file.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum SyncKind {
    Record,
    Batch,
    Interval,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DurabilityFile {
    #[serde(default)]
    sync: Option<SyncKind>,
    #[serde(default)]
    sync_interval_ms: Option<u64>,
    #[serde(default)]
    snapshot_every_frames: Option<u64>,
    #[serde(default)]
    keep_snapshots: Option<usize>,
    #[serde(default)]
    segment_bytes: Option<u64>,
    dilation: bool,
    audit_every_frames: u64,
    #[serde(default)]
    drop_from: Option<u64>,
}

impl DurabilityFile {
    fn into_durability(self) -> anyhow::Result<Durability> {
        let interval = self.sync_interval_ms.map(Duration::from_millis);
        let sync = match (self.sync, interval) {
            (Some(SyncKind::Record), _) => Some(SyncPolicy::EveryRecord),
            (Some(SyncKind::Batch), _) => Some(SyncPolicy::EveryBatch),
            (Some(SyncKind::Interval) | None, Some(interval)) => {
                Some(SyncPolicy::Interval(interval))
            }
            (None, None) => None,
            // The interval of the runtime, when its own policy has one.
            (Some(SyncKind::Interval), None) => match DurableConfig::default().sync {
                policy @ SyncPolicy::Interval(_) => Some(policy),
                _ => anyhow::bail!(
                    "`durability.sync = \"interval\"` needs `durability.sync_interval_ms`"
                ),
            },
        };
        if let Some(keep) = self.keep_snapshots {
            anyhow::ensure!(keep >= 1, "`durability.keep_snapshots` must be at least 1");
        }
        Ok(Durability {
            sync,
            snapshot_every: self.snapshot_every_frames,
            keep_snapshots: self.keep_snapshots,
            segment_bytes: self.segment_bytes,
            dilation: self.dilation,
            audit_every: self.audit_every_frames,
            drop_from: self.drop_from,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldFile {
    seed: u64,
    cities: u32,
    districts: u32,
}

fn path_value(path: &Path) -> toml::Value {
    toml::Value::String(path.to_string_lossy().into_owned())
}

fn integer(name: &str, value: u64) -> anyhow::Result<toml::Value> {
    let value = i64::try_from(value).with_context(|| format!("`{name}` is too large"))?;
    Ok(toml::Value::Integer(value))
}

impl Overrides {
    /// Writes the values that are set into a table with the shape of the
    /// configuration file.
    fn into_table(self) -> anyhow::Result<toml::Table> {
        let mut table = toml::Table::new();
        let mut world = toml::Table::new();
        let mut gateway = toml::Table::new();
        let mut durability = toml::Table::new();
        let mut github = toml::Table::new();
        if let Some(bind) = self.bind {
            table.insert("bind".into(), toml::Value::String(bind.to_string()));
        }
        if let Some(tick_ms) = self.tick_ms {
            table.insert("tick_ms".into(), integer("tick_ms", tick_ms)?);
        }
        if let Some(seed) = self.seed {
            world.insert("seed".into(), integer("seed", seed)?);
        }
        if let Some(cities) = self.cities {
            world.insert("cities".into(), toml::Value::Integer(i64::from(cities)));
        }
        if let Some(districts) = self.districts {
            world.insert("districts".into(), toml::Value::Integer(i64::from(districts)));
        }
        if let Some(web_dir) = &self.web_dir {
            table.insert("web_dir".into(), path_value(web_dir));
        }
        if let Some(data_dir) = &self.data_dir {
            table.insert("data_dir".into(), path_value(data_dir));
        }
        if let Some(store) = self.store {
            table.insert("store".into(), toml::Value::String(store.as_str().into()));
        }
        if let Some(world_store) = self.world_store {
            table.insert("world_store".into(), toml::Value::String(world_store.as_str().into()));
        }
        if !self.params.is_empty() {
            let files = self.params.iter().map(|file| path_value(file)).collect();
            table.insert("params".into(), toml::Value::Array(files));
        }
        if let Some(drop_from) = self.drop_from {
            durability.insert("drop_from".into(), integer("drop_from", drop_from)?);
        }
        if let Some(client_id) = self.github_client_id {
            github.insert("client_id".into(), toml::Value::String(client_id));
        }
        if let Some(client_secret) = self.github_client_secret {
            github.insert("client_secret".into(), toml::Value::String(client_secret));
        }
        if let Some(join_file) = &self.join_file {
            table.insert("join_file".into(), path_value(join_file));
        }
        if let Some(admin_token) = self.admin_token {
            gateway.insert("admin_token".into(), toml::Value::String(admin_token));
        }
        gateway.insert("github".into(), toml::Value::Table(github));
        table.insert("world".into(), toml::Value::Table(world));
        table.insert("durability".into(), toml::Value::Table(durability));
        table.insert("gateway".into(), toml::Value::Table(gateway));
        Ok(table)
    }
}

impl ServerConfig {
    /// Builds the configuration from the defaults, an optional file and
    /// the values of the environment and the command line.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read, is not TOML, has a key that
    /// the server does not know, or has a value that is not valid.
    pub fn load(file: Option<&Path>, overrides: Overrides) -> anyhow::Result<ServerConfig> {
        let mut table: toml::Table =
            toml::from_str(DEFAULTS).context("the built-in defaults are not valid TOML")?;
        if let Some(path) = file {
            let text = std::fs::read_to_string(path).with_context(|| {
                format!("cannot read the configuration file {}", path.display())
            })?;
            let from_file: toml::Table = toml::from_str(&text)
                .with_context(|| format!("{} is not valid TOML", path.display()))?;
            merge_tables(&mut table, from_file);
        }
        let dev = overrides.dev;
        if let Some(toml::Value::Table(for_dev)) = table.remove("dev")
            && dev
        {
            merge_tables(&mut table, for_dev);
        }
        merge_tables(&mut table, overrides.into_table()?);

        let file: FileConfig = table.try_into().map_err(|e: toml::de::Error| {
            anyhow::anyhow!("the configuration is not valid: {}", e.message())
        })?;
        anyhow::ensure!(file.world.cities >= 1, "`world.cities` must be at least 1");
        anyhow::ensure!(file.world.districts >= 1, "`world.districts` must be at least 1");

        anyhow::ensure!(
            file.sponsors.answer_timeout_ms >= 1,
            "`sponsors.answer_timeout_ms` must be at least 1"
        );
        // The viewer service reads its own table. It gets the table as
        // text, and it names what is wrong in it.
        let spectator = toml::to_string(&file.spectator)
            .context("the `[spectator]` table cannot be written as TOML")
            .and_then(|text| {
                unsup_spectator::Config::with_overrides(&text)
                    .map_err(|error| anyhow::anyhow!("the `[spectator]` table: {}", error.message))
            })?;

        let mut gateway = file.gateway;
        // `join_file` at the top is the default; `[gateway]` can replace it.
        if !gateway.contains_key("join_file") {
            gateway.insert("join_file".into(), path_value(&file.join_file));
        }
        Ok(ServerConfig {
            bind: file.bind,
            tick_interval: Duration::from_millis(file.tick_ms),
            world: WorldShape {
                seed: file.world.seed,
                cities: file.world.cities,
                districts: file.world.districts,
            },
            web_dir: Some(file.web_dir),
            store: match file.store {
                StoreKind::Memory => StoreConfig::Memory,
                StoreKind::Sqlite => StoreConfig::Sqlite(file.data_dir.join(STORE_FILE)),
            },
            // Accounts in memory: a world on the disk would have residents
            // without keys after a restart.
            world_store: match (file.world_store, file.store) {
                (WorldStoreKind::Disk, StoreKind::Sqlite) => {
                    WorldStore::Dir(file.data_dir.join(WORLD_DIR))
                }
                (WorldStoreKind::Memory, _) | (_, StoreKind::Memory) => WorldStore::Memory,
            },
            durability: file.durability.into_durability()?,
            params: file.params,
            gateway: GatewayConfig::from_table(gateway)?,
            spectator,
            sponsors: file.sponsors,
            dev,
            shutdown_grace: Duration::from_millis(file.shutdown_grace_ms),
        })
    }

    /// Returns the configuration of the development mode: fast ticks,
    /// accounts and the world in memory, webhooks to local addresses, a
    /// viewer on any origin, and a ready owner.
    #[must_use]
    pub fn dev() -> ServerConfig {
        // The defaults are part of the binary, and a test reads them.
        ServerConfig::load(None, Overrides { dev: true, ..Overrides::default() })
            .expect("the defaults in data/defaults.toml are a complete configuration")
    }
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        // The defaults are part of the binary, and a test reads them.
        ServerConfig::load(None, Overrides::default())
            .expect("the defaults in data/defaults.toml are a complete configuration")
    }
}
