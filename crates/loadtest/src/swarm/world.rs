//! Reads the map of the world of a server: `GET /v1/world` and
//! `GET /v1/world/zones/{id}` for each zone.

use std::time::Duration;

use anyhow::{Context, anyhow};
use futures::{StreamExt, TryStreamExt};
use unsup_protocol::{WorldView, ZoneId};

use super::error::RequestError;
use super::http::{Urls, get_json};
use crate::defaults::defaults;
use crate::map::WorldMap;

/// Reads one view. An answer that a limit of the server refused is asked
/// for again after the time that the server names, a few times.
async fn view(
    client: &reqwest::Client,
    url: &str,
    retry: Duration,
    tries: u32,
) -> Result<WorldView, RequestError> {
    let mut left = tries.max(1);
    loop {
        match get_json::<WorldView>(client, url, None).await {
            Ok((view, _)) => return Ok(view),
            Err(error) if error.throttled() && left > 1 => {
                left -= 1;
                let pause = error.retry_after_secs.map_or(retry, Duration::from_secs).max(retry);
                tokio::time::sleep(pause).await;
            }
            Err(error) => return Err(error),
        }
    }
}

/// Reads the layout of the world and the tiles and sites of each zone, a
/// few zones at the same time, and builds the map.
///
/// # Errors
///
/// Fails when the server does not give the layout or a zone, or when the
/// views do not make a map.
pub async fn load_map(
    client: &reqwest::Client,
    urls: &Urls,
    retry: Duration,
    tries: u32,
) -> anyhow::Result<WorldMap> {
    let layout = view(client, &urls.world, retry, tries)
        .await
        .map_err(|error| anyhow!("{error}"))
        .with_context(|| format!("cannot read the layout of the world at {}", urls.world))?;
    let WorldView::Layout { zones, .. } = &layout else {
        return Err(anyhow!("{} did not answer with the layout of the world", urls.world));
    };
    let ids: Vec<ZoneId> = zones.iter().map(|zone| zone.id).collect();
    let at_once = defaults().swarm.map_requests.max(1);
    let views: Vec<WorldView> = futures::stream::iter(ids)
        .map(|zone| async move {
            let url = urls.zone(zone);
            view(client, &url, retry, tries)
                .await
                .map_err(|error| anyhow!("{error}"))
                .with_context(|| format!("cannot read the map of zone {zone} at {url}"))
        })
        .buffered(at_once)
        .try_collect()
        .await?;
    Ok(WorldMap::from_views(&layout, views)?)
}
