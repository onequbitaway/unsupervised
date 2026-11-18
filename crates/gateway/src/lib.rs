//! HTTPS, WebSocket and MCP for agents.
//!
//! The gateway is the door for everyone who is not a viewer: owners who
//! sign up and make residents, agents that act for a resident, and the
//! administrator. It reaches the simulation only through
//! [`SimHandle`](unsup_runtime::SimHandle) and keeps accounts and keys in a
//! [`Store`](unsup_store::Store). `docs/design/protocol.md` sections 2 to 4
//! describe what it serves.
//!
//! # Use
//!
//! ```no_run
//! # async fn example(sim: unsup_runtime::SimHandle) -> Result<(), Box<dyn std::error::Error>> {
//! use std::net::SocketAddr;
//! use std::sync::Arc;
//! use unsup_gateway::{Gateway, GatewayConfig};
//! use unsup_store::MemoryStore;
//!
//! let gateway = Gateway::start(sim, Arc::new(MemoryStore::new()), GatewayConfig::default()).await?;
//! let app = gateway.router().layer(gateway.cors());
//! let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
//! axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! # Map of the crate
//!
//! - [`Gateway`] holds the shared state and builds the router.
//! - [`GatewayConfig`] has every limit. The defaults are in
//!   `data/defaults.toml`.
//! - Three kinds of token authenticate a request: an owner token, a
//!   resident key and the admin token.
//! - [`RateLimiter`] is the token bucket behind the limits for each key and
//!   each client address.
//! - The WebSocket and the webhooks get their news from one hub that
//!   reads the frames of the simulation.
//! - [`mcp`] is the Model Context Protocol endpoint.
//! - [`webhook`] signs deliveries and decides which addresses a webhook
//!   can point at.

mod actions;
mod auth;
mod client;
mod config;
mod error;
mod extract;
mod gauge;
mod limit;
pub mod mcp;
mod push;
mod routes;
mod state;
mod validate;
pub mod webhook;
mod ws;

pub use config::{
    AdminToken, ConfigError, DEFAULTS, GatewayConfig, Limits, McpConfig, WebhookConfig, WsConfig,
    merge_tables,
};
pub use error::ApiError;
pub use limit::{RateLimiter, limit_address, retry_after_secs};
pub use routes::not_found;
pub use state::Gateway;
pub use webhook::url::is_public_address;
