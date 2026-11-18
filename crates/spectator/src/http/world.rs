//! `/v1/world` and `/v1/world/zones/{zone}`: the layout and the zones.

use std::sync::Arc;

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::extract::{Path, Query, State};
use serde::Deserialize;
use unsup_protocol::{ViewQuery, WorldView, ZoneId};

use super::error::ApiError;
use crate::hub::Hub;
use crate::wire::FeedStats;

/// Answers `/v1/world`: the cities and zones with their numbers of
/// residents.
pub(crate) async fn layout(State(hub): State<Arc<Hub>>) -> Result<Json<WorldView>, ApiError> {
    hub.ready().await?;
    let layout = hub.cache().layout();
    Ok(Json(layout))
}

/// The query of `/v1/world/zones/{zone}`.
#[derive(Debug, Deserialize)]
pub(crate) struct ZoneParams {
    /// False leaves the residents out.
    residents: Option<bool>,
}

/// Ends the refresh of a zone map, also when the request is dropped.
struct Refresh<'a> {
    hub: &'a Hub,
    zone: ZoneId,
}

impl Drop for Refresh<'_> {
    fn drop(&mut self) {
        self.hub.cache().refresh_done(self.zone);
    }
}

/// Reads the tiles and sites of a zone again when the copy of the cache is
/// old. One request at a time does this for a zone, so viewers cannot load
/// the simulation with it.
async fn refresh_map(hub: &Hub, zone: ZoneId) {
    if !hub.cache().claim_refresh(zone, hub.config.map.refresh_frames) {
        return;
    }
    let _refresh = Refresh { hub, zone };
    match hub.sim.view(ViewQuery::Zone { zone }).await {
        Ok(view) => hub.cache_mut().set_map(view),
        // The old map stays. A later request tries again.
        Err(error) => tracing::debug!(%zone, %error, "the map of a zone could not be read again"),
    }
}

/// Answers `/v1/world/zones/{zone}`: tiles, sites and residents of a zone.
pub(crate) async fn zone(
    State(hub): State<Arc<Hub>>,
    Path(zone): Path<String>,
    params: Result<Query<ZoneParams>, QueryRejection>,
) -> Result<Json<WorldView>, ApiError> {
    hub.ready().await?;
    let Query(params) = params?;
    let zone = zone
        .parse::<u32>()
        .map(ZoneId)
        .map_err(|_| ApiError::invalid("A zone id is a number from 0."))?;
    refresh_map(&hub, zone).await;
    let view = hub.cache().zone(zone, params.residents.unwrap_or(true));
    view.map(Json).ok_or_else(|| {
        ApiError::not_found(
            "unknown_zone",
            format!("Zone {zone} does not exist or is not served here."),
        )
    })
}

/// Answers `/v1/spectate/stats`: numbers about the feed.
pub(crate) async fn stats(State(hub): State<Arc<Hub>>) -> Result<Json<FeedStats>, ApiError> {
    hub.ready().await?;
    Ok(Json(hub.stats()))
}
