//! Published state: `/v1/published`, `/v1/stats/economy` and
//! `/v1/leaderboards/{board}`.
//!
//! A module publishes state as an event whose kind ends in `_published`.
//! The cache keeps the newest event of each kind and key. These routes
//! serve them; they answer 404 until a module publishes the data.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use super::error::ApiError;
use crate::cache::PUBLISHED_SUFFIX;
use crate::hub::Hub;
use crate::wire::{Published, PublishedIndex, PublishedKind};

fn nothing_published(kind: &str) -> ApiError {
    ApiError::not_found("not_published", format!("No module has published `{kind}` yet."))
}

fn kind_answer(hub: &Hub, kind: &str) -> Result<Json<PublishedKind>, ApiError> {
    let series = hub.cache().published_kind(kind);
    if series.is_empty() {
        return Err(nothing_published(kind));
    }
    Ok(Json(PublishedKind { kind: kind.to_string(), series }))
}

fn check_kind(kind: &str) -> Result<(), ApiError> {
    if kind.ends_with(PUBLISHED_SUFFIX) {
        Ok(())
    } else {
        Err(ApiError::invalid(format!("The kind of published state ends in `{PUBLISHED_SUFFIX}`.")))
    }
}

/// Answers `/v1/published`: one line for every series.
pub(crate) async fn index(State(hub): State<Arc<Hub>>) -> Result<Json<PublishedIndex>, ApiError> {
    hub.ready().await?;
    let published = hub.cache().published_index();
    Ok(Json(PublishedIndex { published }))
}

/// Answers `/v1/published/{kind}`: every series of one kind.
pub(crate) async fn kind(
    State(hub): State<Arc<Hub>>,
    Path(kind): Path<String>,
) -> Result<Json<PublishedKind>, ApiError> {
    hub.ready().await?;
    check_kind(&kind)?;
    kind_answer(&hub, &kind)
}

/// Answers `/v1/published/{kind}/{key}`: one series.
pub(crate) async fn series(
    State(hub): State<Arc<Hub>>,
    Path((kind, key)): Path<(String, String)>,
) -> Result<Json<Published>, ApiError> {
    hub.ready().await?;
    check_kind(&kind)?;
    let published = hub.cache().published(&kind, Some(&key));
    published.map(Json).ok_or_else(|| {
        ApiError::not_found(
            "not_published",
            format!("No module has published `{kind}` with the key `{key}` yet."),
        )
    })
}

/// Answers `/v1/stats/economy`: every series of the statistics kind.
pub(crate) async fn economy(State(hub): State<Arc<Hub>>) -> Result<Json<PublishedKind>, ApiError> {
    hub.ready().await?;
    kind_answer(&hub, &hub.config.published.stats_kind)
}

/// Answers `/v1/leaderboards/{board}`: the newest page of one board.
pub(crate) async fn leaderboard(
    State(hub): State<Arc<Hub>>,
    Path(board): Path<String>,
) -> Result<Json<Published>, ApiError> {
    hub.ready().await?;
    let kind = &hub.config.published.leaderboard_kind;
    let published = hub.cache().published(kind, Some(&board));
    published.map(Json).ok_or_else(|| {
        ApiError::not_found(
            "not_published",
            format!("No module has published the leaderboard `{board}` (`{kind}`) yet."),
        )
    })
}
