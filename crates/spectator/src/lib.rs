//! Live feed for viewers.
//!
//! Viewers read the world through this crate. It can never change the
//! simulation: it only calls the read methods of [`SimHandle`].
//!
//! # How it works
//!
//! One task reads the frames of the simulation ([`SimHandle::frames`]) and
//! keeps a read-only cache: the residents of every zone, the number of
//! residents per zone, tiles and sites, the recent public events, the
//! published state and the last residents that left the world. At the
//! start, and after it lost frames, the task reads the whole world with
//! [`SimHandle::view`].
//!
//! Every answer for a viewer comes from the cache. A viewer costs the
//! simulation nothing, however many viewers there are and however slow
//! they are. The one exception is the map of a zone (tiles and sites): a
//! request reads it again when the copy is old, at most one request at a
//! time for a zone.
//!
//! # Routes
//!
//! | Route | Answer |
//! | --- | --- |
//! | `GET /v1/world` | `WorldView::layout` |
//! | `GET /v1/world/zones/{zone}` | `WorldView::zone` |
//! | `GET /v1/spectate` | The WebSocket feed: [`wire::ClientMessage`] in, [`wire::ServerMessage`] out |
//! | `GET /v1/spectate/stats` | [`wire::FeedStats`] |
//! | `GET /v1/feed` | [`wire::FeedPage`] |
//! | `GET /v1/residents/{id}/profile` | [`wire::Profile`] |
//! | `GET /v1/published` | [`wire::PublishedIndex`] |
//! | `GET /v1/published/{kind}` | [`wire::PublishedKind`] |
//! | `GET /v1/published/{kind}/{key}` | [`wire::Published`] |
//! | `GET /v1/stats/economy` | [`wire::PublishedKind`] of the statistics kind |
//! | `GET /v1/leaderboards/{board}` | [`wire::Published`] of the leaderboard kind |
//! | `GET /debug` | A page without a build step that draws every resident as a dot |
//!
//! `docs/design/protocol.md`, section "Viewer feed", is the contract of
//! the feed.

mod cache;
mod config;
mod digest;
mod http;
mod hub;
mod pump;
mod session;
pub mod wire;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::response::{Html, Response};
use axum::routing::get;
use unsup_runtime::SimHandle;

pub use crate::config::{
    Config, ConfigError, FeedConfig, MapConfig, ProfileConfig, PublishedConfig, SocketConfig,
    StartConfig,
};
use crate::hub::Hub;

/// A page without a build step that draws every resident as a dot. It is a
/// development tool. The real viewer is in `web/`.
const DEBUG_PAGE: &str = include_str!("debug.html");

/// Returns the routes that viewers use, with the limits of
/// `data/spectator.toml`.
///
/// Call it inside a Tokio runtime: the task that feeds the cache starts at
/// once. Outside a runtime the task starts with the first request, and the
/// feed has no events from before that.
pub fn router(sim: SimHandle) -> Router {
    build(sim, Config::default())
}

/// Returns the routes that viewers use, with other limits. See [`router`].
///
/// # Errors
///
/// Fails when [`Config::validate`] refuses the limits.
pub fn router_with(sim: SimHandle, config: Config) -> Result<Router, ConfigError> {
    config.validate()?;
    Ok(build(sim, config))
}

fn build(sim: SimHandle, config: Config) -> Router {
    let hub = Hub::new(sim, config);
    if tokio::runtime::Handle::try_current().is_ok() {
        hub.start();
    }
    Router::new()
        .route("/v1/world", get(http::world::layout))
        .route("/v1/world/zones/{zone}", get(http::world::zone))
        .route("/v1/spectate", get(spectate))
        .route("/v1/spectate/stats", get(http::world::stats))
        .route("/v1/feed", get(http::feed::feed))
        .route("/v1/residents/{id}/profile", get(http::feed::profile))
        .route("/v1/published", get(http::published::index))
        .route("/v1/published/{kind}", get(http::published::kind))
        .route("/v1/published/{kind}/{key}", get(http::published::series))
        .route("/v1/stats/economy", get(http::published::economy))
        .route("/v1/leaderboards/{board}", get(http::published::leaderboard))
        .route("/debug", get(|| async { Html(DEBUG_PAGE) }))
        .with_state(hub)
}

/// Upgrades a request to the WebSocket feed.
async fn spectate(State(hub): State<Arc<Hub>>, upgrade: WebSocketUpgrade) -> Response {
    upgrade
        .max_message_size(hub.config.socket.max_message_bytes)
        .on_upgrade(move |socket| session::run(socket, hub))
}
