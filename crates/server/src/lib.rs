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
//!   The viewer service keeps the life histories and the pinned clips
//!   next to the world, so they come back with it.
//! - The route `POST /v1/missions` is here: an owner publishes a mission
//!   as a sponsor ([`SponsorConfig`]). It needs the missions module, and
//!   the gateway knows no module.
//! - [`audit_world`] replays the log of a data directory with all game
//!   modules and checks it, for the `replay` command.
//! - [`build_world`] builds a world with all game modules, for tools that
//!   need no server.

mod audit;
mod config;
mod durable;
pub mod plugins;
mod server;
mod sponsor;
mod viewer;
mod world;

pub use audit::{CheckedSnapshot, MoneyTotals, WorldAudit, audit_world};
pub use config::{
    DEFAULTS, Durability, Overrides, ServerConfig, StoreConfig, StoreKind, WorldStore,
    WorldStoreKind,
};
pub use durable::WorldStart;
pub use plugins::plugins;
pub use server::{DevAccess, Server, Stopped, start};
pub use sponsor::SponsorConfig;
pub use world::{WorldShape, build_world, try_build_world};
