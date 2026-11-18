//! The show: `/v1/director`, `/v1/stories` and `/v1/places/{site}/history`.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use unsup_protocol::SiteId;

use super::error::ApiError;
use crate::facts::Atlas;
use crate::hub::Hub;
use crate::wire::{DirectorState, PlaceHistory, StoriesPage, Story};

/// Answers `/v1/director`: the shot on air.
pub(crate) async fn director(State(hub): State<Arc<Hub>>) -> Result<Json<DirectorState>, ApiError> {
    hub.ready().await?;
    let state = hub.studio().director_state();
    Ok(Json(state))
}

/// The query of `/v1/stories`.
#[derive(Debug, Deserialize)]
pub(crate) struct StoriesParams {
    /// `open`: only open stories. `all`: also the closed ones.
    status: Option<String>,
    /// Greatest number of stories.
    limit: Option<usize>,
}

/// Answers `/v1/stories`: the running stories.
pub(crate) async fn stories(
    State(hub): State<Arc<Hub>>,
    params: Result<Query<StoriesParams>, QueryRejection>,
) -> Result<Json<StoriesPage>, ApiError> {
    hub.ready().await?;
    let Query(params) = params?;
    let open_only = match params.status.as_deref() {
        None | Some("all") => false,
        Some("open") => true,
        Some(_) => return Err(ApiError::invalid("`status` is `open` or `all`.")),
    };
    let limits = &hub.config.stories;
    let limit = params.limit.unwrap_or(limits.page_default).clamp(1, limits.page_max.max(1));
    let studio = hub.studio();
    let page = StoriesPage {
        frame: studio.stories.frame(),
        stories: studio.stories.list(limit, open_only),
    };
    drop(studio);
    Ok(Json(page))
}

/// Answers `/v1/stories/{id}`: one story.
pub(crate) async fn story(
    State(hub): State<Arc<Hub>>,
    Path(id): Path<String>,
) -> Result<Json<Story>, ApiError> {
    hub.ready().await?;
    let id = id.parse::<u64>().map_err(|_| ApiError::invalid("A story id is a number."))?;
    let story = hub.studio().stories.get(id).cloned();
    story
        .map(Json)
        .ok_or_else(|| ApiError::not_found("unknown_story", format!("No story has the id {id}.")))
}

/// Answers `/v1/places/{site}/history`: what happened at a site.
pub(crate) async fn place(
    State(hub): State<Arc<Hub>>,
    Path(site): Path<String>,
) -> Result<Json<PlaceHistory>, ApiError> {
    hub.ready().await?;
    let site =
        site.parse::<u64>().map(SiteId).map_err(|_| ApiError::invalid("A site id is a number."))?;
    let cache = hub.cache();
    let known = cache.site(site);
    let history = hub.studio().chronicle.place(site, hub.config.chronicle.place_page);
    drop(cache);
    // A site is known when the cache has it on a map, or when the
    // chronicle has an event of it.
    if known.is_none() && history.is_none() {
        return Err(ApiError::not_found(
            "unknown_place",
            format!("No site with the id {site} is known."),
        ));
    }
    let (zone, events) = history.unwrap_or_default();
    Ok(Json(PlaceHistory {
        site,
        zone: known.as_ref().map(|site| site.zone).or(zone),
        name: known.map(|site| site.name),
        events,
    }))
}
