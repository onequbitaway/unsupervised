//! The reads of the simulation.
//!
//! This module is the only place of the crate that holds the handle of the
//! simulation. No route and no viewer task can read the simulation: they
//! have the cache, and the cache gets everything through the delay of the
//! stream (`delay.rs`).
//!
//! # The stamp of a read
//!
//! A frame of the simulation says which frame it is. A read of the world
//! does not: the view of a zone has no frame number. It shows the world at
//! the moment of the read, with what the inputs since the last frame did.
//! Those changes are part of the next frame of the feed.
//!
//! So each read gets a stamp: the number of the frame after the newest
//! frame that the simulation had when the read was over. The delay holds
//! the read back until the simulation is `stream.delay_frames` frames past
//! the stamp. Nothing that a read shows can then reach a viewer before
//! the frame of the feed that shows the same change.
//!
//! One case needs no frame more: a runtime that has applied nothing but
//! frames since it started. No input has changed the world between two of
//! its frames, so a read shows nothing that is newer than its newest
//! frame, and the stamp is the number of that frame.
//!
//! A world can come to the runtime with inputs after its last frame: a
//! prepared world, or a recovered world whose log ends with inputs. The
//! feed reports those changes in the first frame of the runtime, and the
//! first read of the world shows them one frame before that. They are
//! older than the start of the runtime, so they are still at least the
//! delay old when a viewer gets them.
//!
//! The stamp comes from the simulation, after the read. The newest frame
//! that this service received is no stamp: the task that takes the frames
//! can be behind the simulation, and the read would then leave the delay
//! too early.

use std::sync::Arc;

use tokio::sync::broadcast;
use unsup_protocol::{FrameOutput, SimError, ViewQuery, WorldView, ZoneId};
use unsup_runtime::SimHandle;

use crate::delay::WorldRead;

/// Something that was read from the simulation, with the frame from which
/// the delay of the stream counts.
#[derive(Debug)]
pub(crate) struct Stamped<T> {
    /// The frame after the newest frame of the simulation at the end of
    /// the read. The newest frame itself when the simulation has applied
    /// nothing but frames.
    pub stamp: u64,
    /// What was read.
    pub value: T,
}

/// The door to the simulation. The service only reads through it.
pub(crate) struct Source {
    sim: SimHandle,
}

impl Source {
    /// Wraps the handle of the simulation.
    pub fn new(sim: SimHandle) -> Source {
        Source { sim }
    }

    /// Subscribes to the frames of the simulation.
    pub fn frames(&self) -> broadcast::Receiver<Arc<FrameOutput>> {
        self.sim.frames()
    }

    /// Returns the stamp for a read that ended before this call.
    async fn stamp(&self) -> Result<u64, SimError> {
        let stats = self.sim.stats().await?;
        // The runtime counts every input that it applied since it started,
        // frames included. When the two numbers are equal, no spawn, action
        // or command has changed the world between two of its frames.
        let only_frames = stats.inputs_applied == stats.frames_run;
        Ok(if only_frames { stats.frame } else { stats.frame.saturating_add(1) })
    }

    /// Reads the whole world: the layout, then every zone. Every frame
    /// after the frame of the layout must be applied.
    ///
    /// A zone view can be newer than the layout, because the simulation
    /// goes on between the calls. That does no harm to the cache: a delta
    /// sets positions and activities, it does not add to them, and the
    /// cache counts `entered` and `left` against what it holds. So a frame
    /// that a view already shows can be applied again. The stamp is taken
    /// after the last view, so it covers the newest of them.
    pub async fn read_world(&self) -> Result<Stamped<WorldRead>, SimError> {
        let WorldView::Layout { frame, clock, cities, zones } =
            self.sim.view(ViewQuery::Layout).await?
        else {
            return Err(SimError::Invalid);
        };
        let mut views = Vec::with_capacity(zones.len());
        for zone in &zones {
            match self.sim.view(ViewQuery::Zone { zone: zone.id }).await {
                Ok(view) => views.push(view),
                // Another process hosts the zone. The cache serves it empty.
                Err(SimError::NotHosted) => {}
                Err(error) => return Err(error),
            }
        }
        let stamp = self.stamp().await?;
        Ok(Stamped { stamp, value: WorldRead { frame, clock, cities, zones, views } })
    }

    /// Reads the tiles and the sites of one zone again.
    pub async fn read_map(&self, zone: ZoneId) -> Result<Stamped<WorldView>, SimError> {
        let value = self.sim.view(ViewQuery::Zone { zone }).await?;
        let stamp = self.stamp().await?;
        Ok(Stamped { stamp, value })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use unsup_kernel::{Builder, Plugin, Rect, Scope, Site, SiteId, Stage, World, WorldConfig};
    use unsup_runtime::RuntimeConfig;

    use super::*;

    /// A plugin with one site in each zone whose name is the number of the
    /// last frame of the zone. So a view of a zone says how old it is.
    struct Clock;

    impl Plugin for Clock {
        fn name(&self) -> &'static str {
            "clock"
        }

        fn build(&self, b: &mut Builder) {
            if b.scope() != Scope::Zone {
                return;
            }
            b.on_worldgen(0, |realm, _| {
                let bounds = realm.zone_info().expect("a zone").bounds;
                let id = SiteId(realm.alloc_id());
                let rect = Rect::sized(bounds.x0, bounds.y0, 1, 1);
                realm.sites_mut().insert(Site::new(id, "clock", "0", rect));
            });
            b.system(Stage::Update, 0, "clock.tick", |realm| {
                let frame = realm.frame().to_string();
                for site in realm.sites_mut().iter_mut() {
                    site.name.clone_from(&frame);
                }
            });
        }
    }

    /// Returns the frame that the view of a zone shows.
    fn frame_of(view: &WorldView) -> u64 {
        let WorldView::Zone { sites, .. } = view else {
            panic!("not a zone view: {view:?}");
        };
        sites[0].name.parse().expect("the clock site has a number as its name")
    }

    fn start() -> (SimHandle, unsup_runtime::RuntimeJoin) {
        let world = World::new(&[&Clock], WorldConfig::with_seed(3));
        unsup_runtime::start(world, RuntimeConfig::manual())
    }

    fn spawn_spec() -> unsup_protocol::SpawnSpec {
        unsup_protocol::SpawnSpec {
            owner: unsup_protocol::OwnerId(1),
            name: "Ada".into(),
            model: None,
            city: None,
        }
    }

    #[tokio::test]
    async fn the_stamp_of_a_read_comes_from_the_simulation_after_the_read() {
        let (sim, join) = start();
        let source = Source::new(sim.clone());

        // A world that has run only frames is the world of its newest
        // frame: the stamp is exact.
        let read = source.read_world().await.expect("the world");
        assert_eq!((read.value.frame, read.stamp), (0, 0));
        assert_eq!(read.value.views.len(), read.value.zones.len());
        assert!(!read.value.views.is_empty());
        assert_eq!(sim.step(5).await, Ok(5));
        let read = source.read_world().await.expect("the world");
        assert_eq!((read.value.frame, read.stamp), (5, 5));

        // After an input between two frames, a read can show what frame 6
        // will report. The delay counts from frame 6.
        sim.spawn(spawn_spec()).await.expect("a resident");
        let read = source.read_world().await.expect("the world");
        assert_eq!((read.value.frame, read.stamp), (5, 6));
        let map = source.read_map(ZoneId(0)).await.expect("the map of a zone");
        assert_eq!((frame_of(&map.value), map.stamp), (5, 6));

        // The stamp follows the simulation, not what this service took
        // from the stream of frames: nobody took a frame here.
        assert_eq!(sim.step(2).await, Ok(7));
        let map = source.read_map(ZoneId(0)).await.expect("the map of a zone");
        assert_eq!((frame_of(&map.value), map.stamp), (7, 8));
        assert_eq!(
            source.read_map(ZoneId(99)).await.map(|read| read.stamp),
            Err(SimError::Invalid)
        );
        drop(join);
    }

    /// Runs a read until it waits for the simulation. Returns the result
    /// when the read is over.
    async fn advance<T>(read: &mut Pin<Box<impl Future<Output = T>>>) -> Option<T> {
        match futures::poll!(read.as_mut()) {
            std::task::Poll::Ready(value) => Some(value),
            std::task::Poll::Pending => None,
        }
    }

    #[tokio::test]
    async fn a_world_that_is_read_while_frames_run_is_stamped_after_its_newest_view() {
        let (sim, join) = start();
        let source = Source::new(sim.clone());
        let mut read = Box::pin(source.read_world());

        // The read asks one question at a time. After each question the
        // simulation runs one frame, so each answer is one frame newer
        // than the answer before it.
        let mut frames = 0;
        let done = loop {
            if let Some(done) = advance(&mut read).await {
                break done.expect("the world");
            }
            frames = sim.step(1).await.expect("a frame");
            assert!(frames < 100, "the read does not end");
        };

        // The layout is from before the first frame. The views of the
        // zones are from the frames 1, 2, 3 and 4.
        assert_eq!(done.value.frame, 0);
        let shown: Vec<u64> = done.value.views.iter().map(frame_of).collect();
        assert_eq!(shown, [1, 2, 3, 4]);
        // The read then asked for the frame of the simulation, which was
        // 5. The delay counts from there, not from the frame of the
        // layout. So the newest view waits its whole delay too. Only
        // frames ran, so the stamp is the frame itself.
        assert_eq!(done.stamp, 5);
        assert!(done.stamp > *shown.iter().max().expect("a view"));
        // One more frame ran while the last answer was on its way.
        assert_eq!(frames, 6);
        drop(read);
        drop(join);
    }
}
