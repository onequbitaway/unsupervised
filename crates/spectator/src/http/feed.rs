//! `/v1/feed` and `/v1/residents/{id}/profile`: events and profiles.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use unsup_protocol::ResidentId;

use super::error::ApiError;
use crate::cache::FeedQuery;
use crate::hub::Hub;
use crate::wire::{EventId, FeedPage, Profile};

/// The query of `/v1/feed`.
#[derive(Debug, Deserialize)]
pub(crate) struct FeedParams {
    /// Return the events after this id.
    after: Option<String>,
    /// Greatest number of events.
    limit: Option<usize>,
    /// Lowest drama score.
    min_drama: Option<u8>,
    /// The kind must start with this text.
    kind: Option<String>,
}

/// Answers `/v1/feed`: recent public events, with a cursor.
pub(crate) async fn feed(
    State(hub): State<Arc<Hub>>,
    params: Result<Query<FeedParams>, QueryRejection>,
) -> Result<Json<FeedPage>, ApiError> {
    hub.ready().await?;
    let Query(params) = params?;
    let after = params
        .after
        .map(|text| text.parse::<EventId>())
        .transpose()
        .map_err(|error| ApiError::invalid(format!("`after` is not a cursor: {error}.")))?;
    let limits = &hub.config.feed;
    let query = FeedQuery {
        after,
        limit: params.limit.unwrap_or(limits.page_default).clamp(1, limits.page_max.max(1)),
        min_drama: params.min_drama.unwrap_or(0),
        kind: params.kind.filter(|kind| !kind.is_empty()),
    };
    let page = hub.cache().feed(&query);
    Ok(Json(page))
}

/// Answers `/v1/residents/{id}/profile`: who a resident is, where it is,
/// what it did, and its life story.
pub(crate) async fn profile(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<Json<Profile>, ApiError> {
    hub.ready().await?;
    let id = id
        .parse::<u64>()
        .map(ResidentId)
        .map_err(|_| ApiError::invalid("A resident id is a number."))?;
    let profile = {
        let cache = hub.cache();
        let life = hub.studio().chronicle.life(id);
        cache.profile(id, hub.config.profile.events, life)
    };
    profile.map(Json).ok_or_else(|| {
        ApiError::not_found("unknown_resident", format!("No resident with the id {id} is known."))
    })
}
