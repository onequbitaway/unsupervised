//! The state that all tasks of the spectator service share.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Once, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use tokio::sync::{broadcast, watch};
use unsup_runtime::SimHandle;

use crate::cache::Cache;
use crate::config::Config;
use crate::digest::Digest;
use crate::wire::FeedStats;

/// What the cache tells the viewer tasks.
#[derive(Clone, Debug)]
pub(crate) enum Pulse {
    /// The cache applied one frame.
    Frame(Arc<Digest>),
    /// The cache read the whole world again. Every viewer needs new
    /// snapshots.
    Reset,
    /// The simulation stopped. No frame follows.
    Stopped,
}

/// Where the cache is in its life.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    /// The cache reads the world for the first time.
    Starting,
    /// The cache follows the simulation.
    Ready,
    /// The simulation stopped.
    Ended,
}

/// The cache is not there: the simulation stopped before the cache could
/// read the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct NoWorld;

/// The state that all tasks of the spectator service share.
pub(crate) struct Hub {
    /// The limits of the service.
    pub config: Config,
    /// The door to the simulation. The service only reads through it.
    pub sim: SimHandle,
    cache: RwLock<Cache>,
    pulses: broadcast::Sender<Pulse>,
    phase: watch::Sender<Phase>,
    /// True when the cache has read the world at least once.
    loaded: AtomicBool,
    started: Once,
    viewers: AtomicU32,
    dropped_frames: AtomicU64,
    resyncs: AtomicU64,
}

impl Hub {
    /// Makes the shared state. The cache is empty until [`Hub::start`].
    pub fn new(sim: SimHandle, config: Config) -> Arc<Hub> {
        let (pulses, _) = broadcast::channel(config.socket.frame_buffer.max(1));
        let (phase, _) = watch::channel(Phase::Starting);
        Arc::new(Hub {
            cache: RwLock::new(Cache::new(&config)),
            config,
            sim,
            pulses,
            phase,
            loaded: AtomicBool::new(false),
            started: Once::new(),
            viewers: AtomicU32::new(0),
            dropped_frames: AtomicU64::new(0),
            resyncs: AtomicU64::new(0),
        })
    }

    /// Starts the task that feeds the cache. Later calls do nothing. It
    /// must run inside a Tokio runtime.
    pub fn start(self: &Arc<Hub>) {
        self.started.call_once(|| {
            tokio::spawn(crate::pump::run(Arc::clone(self)));
        });
    }

    /// Waits until the cache holds the world.
    ///
    /// # Errors
    ///
    /// Fails when the simulation stopped before the cache could read the
    /// world.
    pub async fn ready(self: &Arc<Hub>) -> Result<(), NoWorld> {
        self.start();
        let mut phase = self.phase.subscribe();
        // The sender lives in `self`, so the wait cannot fail.
        let _ = phase.wait_for(|phase| *phase != Phase::Starting).await;
        if self.loaded.load(Ordering::Acquire) { Ok(()) } else { Err(NoWorld) }
    }

    /// Returns true when the simulation stopped.
    pub fn ended(&self) -> bool {
        *self.phase.borrow() == Phase::Ended
    }

    /// Says that the cache holds the world and follows the simulation.
    pub fn set_ready(&self) {
        self.loaded.store(true, Ordering::Release);
        self.phase.send_replace(Phase::Ready);
    }

    /// Says that the simulation stopped, and tells every viewer task.
    pub fn end(&self) {
        self.phase.send_replace(Phase::Ended);
        self.publish(Pulse::Stopped);
    }

    /// Reads the cache. Do not hold the guard across an `await`.
    pub fn cache(&self) -> RwLockReadGuard<'_, Cache> {
        self.cache.read().unwrap_or_else(PoisonError::into_inner)
    }

    /// Writes the cache. Do not hold the guard across an `await`.
    pub fn cache_mut(&self) -> RwLockWriteGuard<'_, Cache> {
        self.cache.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Sends a pulse to every viewer task.
    pub fn publish(&self, pulse: Pulse) {
        // A send fails only when no viewer is connected. That is fine.
        let _ = self.pulses.send(pulse);
    }

    /// Subscribes to the pulses of the cache.
    pub fn pulses(&self) -> broadcast::Receiver<Pulse> {
        self.pulses.subscribe()
    }

    /// Counts one more connected viewer until the guard is dropped.
    pub fn viewer(self: &Arc<Hub>) -> ViewerGuard {
        self.viewers.fetch_add(1, Ordering::AcqRel);
        ViewerGuard { hub: Arc::clone(self) }
    }

    /// Returns the number of connected viewers.
    pub fn viewers(&self) -> u32 {
        self.viewers.load(Ordering::Acquire)
    }

    /// Counts frames that one viewer lost.
    pub fn count_dropped(&self, frames: u64) {
        self.dropped_frames.fetch_add(frames, Ordering::AcqRel);
    }

    /// Counts one viewer that needs new snapshots after it lost frames.
    pub fn count_resync(&self) {
        self.resyncs.fetch_add(1, Ordering::AcqRel);
    }

    /// Returns the numbers for `/v1/spectate/stats`.
    pub fn stats(&self) -> FeedStats {
        self.cache().stats(
            self.viewers(),
            self.dropped_frames.load(Ordering::Acquire),
            self.resyncs.load(Ordering::Acquire),
        )
    }
}

/// Counts one connected viewer while it lives.
pub(crate) struct ViewerGuard {
    hub: Arc<Hub>,
}

impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.hub.viewers.fetch_sub(1, Ordering::AcqRel);
    }
}
