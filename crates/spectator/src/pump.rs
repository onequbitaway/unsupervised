//! The task that feeds the cache and the show from the simulation.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast::error::RecvError;
use unsup_protocol::{FrameOutput, SimError, ViewQuery, WorldView};

use crate::delay::{Held, WorldRead};
use crate::hub::{Hub, Pulse};

/// Reads the whole world from the simulation: the layout, then every
/// zone. Every frame after the frame of the layout must be applied.
///
/// A zone view can be newer than the layout, because the simulation goes
/// on between the calls. That does no harm: a delta sets positions and
/// activities, it does not add to them, and the cache counts `entered` and
/// `left` against what it holds. So a frame that a view already shows can
/// be applied again.
async fn read_world(hub: &Hub) -> Result<WorldRead, SimError> {
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
    Ok(WorldRead { frame, clock, cities, zones, views })
}

/// Reads the world until it works. Returns `None` when the simulation
/// stopped.
async fn load(hub: &Hub) -> Option<WorldRead> {
    loop {
        match read_world(hub).await {
            Ok(read) => return Some(read),
            Err(SimError::Shutdown) => return None,
            Err(error) => {
                tracing::warn!(%error, "the spectator cache cannot read the world; it tries again");
                tokio::time::sleep(Duration::from_millis(hub.config.start.retry_ms)).await;
            }
        }
    }
}

/// Writes what the storage does not have yet: new records of the
/// chronicle and new pinned clips.
pub(crate) async fn flush(hub: &Arc<Hub>) {
    let _one_at_a_time = hub.storage_gate.lock().await;
    let rewrite = hub.vault().wants_rewrite();
    let work = hub.studio_mut().take_work(rewrite);
    if work.is_empty() {
        return;
    }
    let writer = Arc::clone(hub);
    // The storage can block: the write runs on a thread for that.
    if let Err(error) = tokio::task::spawn_blocking(move || writer.vault().apply(work)).await {
        tracing::error!(%error, "the task that writes to the storage failed");
    }
}

/// Puts a world into the cache. The first world also brings back what the
/// storage holds, and opens the service for viewers.
async fn apply_world(hub: &Arc<Hub>, read: WorldRead) {
    let first = !hub.is_loaded();
    let (frame, world) = {
        let mut cache = hub.cache_mut();
        cache.set_layout(read.frame, read.clock, read.cities, read.zones);
        for view in read.views {
            cache.load_zone(view);
        }
        hub.studio_mut().reset(cache.residents_by_zone());
        (cache.frame(), cache.signature())
    };
    if first {
        let reader = Arc::clone(hub);
        match tokio::task::spawn_blocking(move || reader.vault().open(world)).await {
            Ok(restored) => hub.studio_mut().restore(restored, frame),
            Err(error) => tracing::error!(%error, "the task that reads the storage failed"),
        }
        hub.set_ready();
    } else {
        hub.publish(Pulse::Reset);
    }
}

/// Puts one frame into the cache and the show, and tells the viewers.
async fn apply_frame(hub: &Arc<Hub>, frame: Arc<FrameOutput>) {
    let viewers = hub.viewers();
    let mut digest = hub.cache_mut().apply_frame(frame, &hub.config, viewers);
    let beat = {
        let cache = hub.cache();
        hub.studio_mut().frame(digest.frame, digest.clock(), &digest.events, &*cache)
    };
    digest.set_show(beat.shot, beat.stories);
    let digest = Arc::new(digest);
    hub.studio_mut().record(Arc::clone(&digest));
    let number = digest.frame;
    hub.publish(Pulse::Frame(digest));
    if number.is_multiple_of(hub.config.chronicle.flush_frames.max(1)) {
        flush(hub).await;
    }
}

/// Gives the cache everything that waited long enough in the delay of the
/// stream.
async fn release(hub: &Arc<Hub>) {
    loop {
        let item = hub.delay().pop_due(hub.live());
        match item {
            Some(Held::Frame(frame)) => apply_frame(hub, frame).await,
            Some(Held::World(read)) => apply_world(hub, *read).await,
            Some(Held::Map(view)) => hub.cache_mut().set_map(view),
            None => return,
        }
    }
}

/// Feeds the cache until the simulation stops.
pub(crate) async fn run(hub: Arc<Hub>) {
    // Subscribe first, so that no frame is lost between the read of the
    // world and the first frame.
    let mut frames = hub.sim.frames();
    let Some(read) = load(&hub).await else {
        hub.end();
        return;
    };
    // The world that the service read already holds the frames up to here.
    let mut base = read.frame;
    hub.set_live(base);
    hub.delay().push(base, Held::World(Box::new(read)));
    release(&hub).await;
    loop {
        match frames.recv().await {
            Ok(frame) if frame.frame <= base => {}
            Ok(frame) => {
                hub.set_live(frame.frame);
                hub.delay().push(frame.frame, Held::Frame(frame));
                release(&hub).await;
            }
            Err(RecvError::Lagged(lost)) => {
                tracing::warn!(lost, "the spectator cache lost frames; it reads the world again");
                frames = hub.sim.frames();
                let Some(read) = load(&hub).await else { break };
                base = read.frame;
                hub.set_live(base);
                hub.delay().push(base, Held::World(Box::new(read)));
                release(&hub).await;
            }
            Err(RecvError::Closed) => break,
        }
    }
    // The simulation stopped. What still waits in the delay is never shown.
    flush(&hub).await;
    hub.end();
}
