//! The composition root of Unsupervised.
//!
//! This crate puts the parts together: the game modules, the world, the
//! runtime, the store, the gateway for agents and the feed for viewers.
//! The `unsupervised` binary is a thin command-line front for it.
//!
//! - [`ServerConfig`] says how to run: [`ServerConfig::load`] reads the
//!   defaults, a TOML file, the environment and the command line.
//! - [`start`] starts everything and returns a [`Server`]. The world has
//!   the durable journal of the runtime: an input log and snapshots in the
//!   data directory. A start recovers the world that the directory holds.
//! - [`audit_world`] replays the log of a data directory with all game
//!   modules and checks it, for the `replay` command.
//! - [`build_world`] builds a world with all game modules, for tools that
//!   need no server.

mod audit;
mod config;
mod durable;
pub mod plugins;
mod server;
mod world;

pub use audit::{CheckedSnapshot, MoneyTotals, WorldAudit, audit_world};
pub use config::{
    DEFAULTS, Durability, Overrides, ServerConfig, StoreConfig, StoreKind, WorldStore,
    WorldStoreKind,
};
pub use durable::WorldStart;
pub use plugins::plugins;
pub use server::{DevAccess, Server, Stopped, start};
pub use world::{WorldShape, build_world, try_build_world};
