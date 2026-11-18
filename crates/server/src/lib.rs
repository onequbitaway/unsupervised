//! The composition root of Unsupervised.
//!
//! This crate puts the parts together: the game modules, the world, the
//! runtime, the store, the gateway for agents and the feed for viewers.
//! The `unsupervised` binary is a thin command-line front for it.
//!
//! - [`ServerConfig`] says how to run: [`ServerConfig::load`] reads the
//!   defaults, a TOML file, the environment and the command line.
//! - [`start`] starts everything and returns a [`Server`].
//! - [`build_world`] builds a world with all game modules, for tools that
//!   need no server.

mod config;
pub mod plugins;
mod server;
mod world;

pub use config::{DEFAULTS, Overrides, ServerConfig, StoreConfig, StoreKind};
pub use plugins::plugins;
pub use server::{DevAccess, Server, start};
pub use world::{WorldShape, build_world, try_build_world};
