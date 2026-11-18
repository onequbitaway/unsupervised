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

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use serde::Deserialize;
use unsup_gateway::{GatewayConfig, merge_tables};

use crate::world::WorldShape;

/// The defaults of the server, as TOML text.
pub const DEFAULTS: &str = include_str!("../data/defaults.toml");

/// Name of the SQLite file in the data directory.
const STORE_FILE: &str = "store.sqlite3";

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
/// keeps accounts in a SQLite file under `var/`. Tests and tools start
/// from [`ServerConfig::dev`], which keeps everything in memory:
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
    /// Limits and options of the gateway.
    pub gateway: GatewayConfig,
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
    /// The file that `GET /join.md` serves.
    pub join_file: Option<PathBuf>,
    /// The token of the administrator.
    pub admin_token: Option<String>,
}

/// The shape of the configuration file.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: SocketAddr,
    tick_ms: u64,
    data_dir: PathBuf,
    store: StoreKind,
    web_dir: PathBuf,
    join_file: PathBuf,
    shutdown_grace_ms: u64,
    world: WorldFile,
    gateway: toml::Table,
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
        if let Some(join_file) = &self.join_file {
            table.insert("join_file".into(), path_value(join_file));
        }
        if let Some(admin_token) = self.admin_token {
            gateway.insert("admin_token".into(), toml::Value::String(admin_token));
        }
        table.insert("world".into(), toml::Value::Table(world));
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
            gateway: GatewayConfig::from_table(gateway)?,
            dev,
            shutdown_grace: Duration::from_millis(file.shutdown_grace_ms),
        })
    }

    /// Returns the configuration of the development mode: fast ticks, a
    /// store in memory, webhooks to local addresses, a viewer on any
    /// origin, and a ready owner.
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
