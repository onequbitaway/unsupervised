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
//! # The show
//!
//! The same task gives every public event to the parts of the show:
//!
//! - The director scores what happens and chooses a shot: a subject, a
//!   zoom level and a caption. A broadcast follows it without a person.
//! - The stories group events that belong together: a crime and its
//!   investigation, an election, a feud, a bankruptcy.
//! - The chronicle keeps a bounded history for each resident and each
//!   site. A profile gives it as a life story.
//! - The recorder keeps the last frames of the feed. A clip shows the
//!   frames around one event again. A pinned clip is kept.
//!
//! The weights, the rules of the stories and the words for events are
//! data: `data/director.toml`. [`Config::with_overrides`] changes some of
//! them. With a storage ([`router_with_storage`]) the chronicle and the
//! pinned clips survive a restart.
//!
//! `stream.delay_frames` in the configuration holds back everything that
//! viewers get by that number of frames. The live view shows what residents
//! cannot see; the delay keeps an owner from giving it to an agent.
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
//! | `GET /v1/residents/{id}/profile` | [`wire::Profile`], with the [`wire::LifeStory`] |
//! | `GET /v1/places/{site}/history` | [`wire::PlaceHistory`] |
//! | `GET /v1/director` | [`wire::DirectorState`] |
//! | `GET /v1/stories` | [`wire::StoriesPage`] |
//! | `GET /v1/stories/{id}` | [`wire::Story`] |
//! | `GET /v1/clips` | [`wire::ClipList`] |
//! | `GET /v1/clips/{event}` | [`wire::Clip`] |
//! | `POST /v1/clips/{event}/pin` | [`wire::PinAnswer`] |
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
mod chronicle;
mod clips;
mod config;
mod delay;
mod digest;
mod director;
mod facts;
mod http;
mod hub;
mod pump;
mod session;
mod stories;
mod studio;
mod vault;
pub mod wire;

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::response::{Html, Response};
use axum::routing::{get, post};
use unsup_runtime::SimHandle;
use unsup_runtime::storage::{MemStorage, Storage};

pub use crate::config::{
    ChaptersConfig, ChronicleConfig, ClipsConfig, Config, ConfigError, DirectorConfig, FactsConfig,
    FeedConfig, MapConfig, Phrases, ProfileConfig, PublishedConfig, SocketConfig, StartConfig,
    StoriesConfig, StoryKind, StreamConfig,
};
use crate::hub::Hub;

/// A page without a build step that draws every resident as a dot. It is a
/// development tool. The real viewer is in `web/`.
const DEBUG_PAGE: &str = include_str!("debug.html");

/// Returns the routes that viewers use, with the configuration of
/// `data/spectator.toml` and `data/director.toml`. The chronicle and the
/// pinned clips live in memory only.
///
/// Call it inside a Tokio runtime: the task that feeds the cache starts at
/// once. Outside a runtime the task starts with the first request, and the
/// feed has no events from before that.
pub fn router(sim: SimHandle) -> Router {
    build(sim, Config::default(), Arc::new(MemStorage::new()))
}

/// Returns the routes that viewers use, with another configuration. The
/// chronicle and the pinned clips live in memory only. See [`router`].
///
/// # Errors
///
/// Fails when [`Config::validate`] refuses the configuration.
pub fn router_with(sim: SimHandle, config: Config) -> Result<Router, ConfigError> {
    router_with_storage(sim, config, Arc::new(MemStorage::new()))
}

/// Returns the routes that viewers use, with another configuration and
/// with a storage. The service writes the chronicle and the pinned clips
/// to the storage and reads them again at the next start, so that life
/// stories and shared clips survive a restart. See [`router`].
///
/// `unsup_runtime::storage::FsStorage` keeps the data in a directory. Give
/// the service a directory of its own: it uses the names `chronicle/` and
/// `clips/`.
///
/// # Errors
///
/// Fails when [`Config::validate`] refuses the configuration.
pub fn router_with_storage(
    sim: SimHandle,
    config: Config,
    storage: Arc<dyn Storage>,
) -> Result<Router, ConfigError> {
    config.validate()?;
    Ok(build(sim, config, storage))
}

fn build(sim: SimHandle, config: Config, storage: Arc<dyn Storage>) -> Router {
    let hub = Hub::new(sim, config, storage);
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
        .route("/v1/director", get(http::show::director))
        .route("/v1/stories", get(http::show::stories))
        .route("/v1/stories/{id}", get(http::show::story))
        .route("/v1/places/{site}/history", get(http::show::place))
        .route("/v1/clips", get(http::clips::list))
        .route("/v1/clips/{event}", get(http::clips::clip))
        .route("/v1/clips/{event}/pin", post(http::clips::pin))
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
