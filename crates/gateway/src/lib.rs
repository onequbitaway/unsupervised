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
//!   [`GatewayBuilder`] adds identity providers and text filters.
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
//! - [`moderation`] is the door for text: every text that a client sends
//!   passes the [`TextFilter`]s before it enters the simulation.
//!   [`RuleFilter`] is the filter with fixed rules.
//! - Sanctions: an administrator mutes or freezes a resident and bans an
//!   owner. The store keeps them across a restart.
//! - Operations: the funnel detector with its [`Flag`] list, the
//!   [`Alerts`], the admin dashboard, and the rollback of the world with
//!   [`Gateway::reconcile`].
//! - [`identity`] has the sign-in with GitHub.
//! - [`host`] is the hook for a route that the host adds for owners
//!   ([`OwnerRoute`]), for example a route that speaks to a game module.

mod actions;
mod auth;
mod client;
mod config;
mod error;
mod extract;
mod gauge;
pub mod host;
pub mod identity;
mod limit;
pub mod mcp;
pub mod moderation;
mod ops;
mod push;
mod recover;
mod routes;
mod sanctions;
mod state;
mod validate;
pub mod webhook;
mod ws;

pub use config::{
    AdminToken, ConfigError, DEFAULTS, GatewayConfig, Limits, McpConfig, WebhookConfig, WsConfig,
    merge_tables,
};
pub use error::ApiError;
pub use host::{OwnerCall, OwnerRoute, OwnerRouteFuture};
pub use identity::{ClientSecret, GitHubConfig, GitHubSignIn};
pub use limit::{RateLimiter, limit_address, retry_after_secs};
pub use moderation::{
    Author, FilterFuture, LinkRule, ModerationConfig, Refusal, RuleFilter, Text, TextFilter,
    TextUse,
};
pub use ops::alerts::{Alert, Alerts};
pub use ops::funnel::{Flag, FunnelConfig, Gift, GiftEvent, Giver};
pub use ops::{AbuseConfig, OpsConfig};
pub use recover::Reconciled;
pub use routes::not_found;
pub use state::{Gateway, GatewayBuilder, StartError};
pub use webhook::url::is_public_address;
