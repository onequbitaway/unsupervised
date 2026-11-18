//! The task that feeds the cache from the simulation.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use unsup_protocol::{SimError, ViewQuery, WorldView};

use crate::hub::{Hub, Pulse};

/// Reads the whole world into the cache: the layout, then every zone.
/// Returns the frame of the layout. Every later frame must be applied.
///
/// A zone view can be newer than the layout, because the simulation goes
/// on between the calls. That does no harm: a delta sets positions and
/// activities, it does not add to them, and the cache counts `entered` and
/// `left` against what it holds. So a frame that a view already shows can
/// be applied again.
async fn read_world(hub: &Hub) -> Result<u64, SimError> {
    let WorldView::Layout { frame, clock, cities, zones } = hub.sim.view(ViewQuery::Layout).await?
    else {
        return Err(SimError::Invalid);
    };
    let mut views = Vec::with_capacity(zones.len());
    for zone in &zones {
        match hub.sim.view(ViewQuery::Zone { zone: zone.id }).await {
            Ok(view) => views.push(view),
            // Another process hosts the zone. The cache serves it empty.
            Err(SimError::NotHosted) => {}
            Err(error) => return Err(error),
        }
    }
    let mut cache = hub.cache_mut();
    cache.set_layout(frame, clock, cities, zones);
    for view in views {
        cache.load_zone(view);
    }
    Ok(frame)
}

/// Reads the world until it works. Returns `None` when the simulation
/// stopped.
async fn load(hub: &Hub) -> Option<u64> {
    loop {
        match read_world(hub).await {
            Ok(frame) => return Some(frame),
            Err(SimError::Shutdown) => return None,
            Err(error) => {
                tracing::warn!(%error, "the spectator cache cannot read the world; it tries again");
                tokio::time::sleep(Duration::from_millis(hub.config.start.retry_ms)).await;
            }
        }
    }
}

/// Feeds the cache until the simulation stops.
pub(crate) async fn run(hub: Arc<Hub>) {
    // Subscribe first, so that no frame is lost between the read of the
    // world and the first frame.
    let mut frames = hub.sim.frames();
    let Some(mut base) = load(&hub).await else {
        hub.end();
        return;
    };
    hub.set_ready();
    loop {
        match frames.recv().await {
            // The world that the cache read already holds this frame.
            Ok(frame) if frame.frame <= base => {}
            Ok(frame) => {
                let viewers = hub.viewers();
                let digest = hub.cache_mut().apply_frame(frame, &hub.config, viewers);
                hub.publish(Pulse::Frame(Arc::new(digest)));
            }
            Err(RecvError::Lagged(lost)) => {
                tracing::warn!(lost, "the spectator cache lost frames; it reads the world again");
                frames = hub.sim.frames();
                match load(&hub).await {
                    Some(frame) => base = frame,
                    None => break,
                }
                hub.publish(Pulse::Reset);
            }
            Err(RecvError::Closed) => break,
        }
    }
    hub.end();
}
