//! How a cluster is put together.

use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use unsup_kernel::WorldConfig;
use unsup_protocol::ZoneId;

use super::error::ClusterError;
use crate::durable::DurableConfig;
use crate::journal::{NoDilation, Pacer};
use crate::settings::settings;
use crate::storage::Storage;

/// Where a process keeps its input logs and snapshots, and how it writes
/// them.
///
/// Each realm of the process gets its own directory in the storage:
/// `global`, `zone-00000`, `zone-00001` and so on. Each directory holds the
/// input log, the snapshots and the seals of that realm.
#[derive(Clone)]
pub struct ClusterDurable {
    /// The storage of this process. Two processes must not share one.
    pub storage: Arc<dyn Storage>,
    /// How every realm of the process writes its log and its snapshots.
    /// `drop_from` is not used: the cluster sets it in a recovery.
    pub config: DurableConfig,
}

impl fmt::Debug for ClusterDurable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterDurable").field("config", &self.config).finish_non_exhaustive()
    }
}

/// Returns the number of zones of a world configuration.
pub(crate) fn zone_count(world: &WorldConfig) -> usize {
    world.cities.iter().map(|city| (city.districts as usize).pow(2)).sum()
}

/// Splits the zones of a world over `workers` workers.
///
/// Each worker gets a row of zones with numbers that follow each other.
/// Zones are numbered city by city and row by row, so the zones of a
/// worker are near each other. The first workers get one zone more when
/// the zones do not divide evenly. A worker gets no zone when there are
/// more workers than zones.
#[must_use]
pub fn partition(world: &WorldConfig, workers: usize) -> Vec<Vec<ZoneId>> {
    let zones = zone_count(world);
    let workers = workers.max(1);
    let (share, extra) = (zones / workers, zones % workers);
    let mut next = 0u32;
    (0..workers)
        .map(|worker| {
            let count = share + usize::from(worker < extra);
            let list = (next..next + count as u32).map(ZoneId).collect();
            next += count as u32;
            list
        })
        .collect()
}

/// How the coordinator runs a cluster.
///
/// The numbers come from `data/runtime.toml`. Build a value with
/// [`ClusterConfig::manual`] or [`ClusterConfig::real_time`] and the struct
/// update syntax.
pub struct ClusterConfig {
    /// The world. Every process must have the same one; the handshake
    /// checks it. The field `hosted` is not used.
    pub world: WorldConfig,
    /// The zones of each worker. The index is the slot of the worker.
    /// Every zone of the world must be in exactly one list.
    pub workers: Vec<Vec<ZoneId>>,
    /// The zone in which every new resident is made. See the limits in
    /// `README.md`: with the kernel as it is, one world gives out the ids
    /// of the residents.
    pub spawn_zone: ZoneId,
    /// Real time between two frames. Zero means manual stepping.
    pub tick_interval: Duration,
    /// Greatest number of calls that wait in the mailbox of the
    /// coordinator.
    pub mailbox: usize,
    /// Frames that a slow subscriber can fall behind.
    pub frame_buffer: usize,
    /// Decides which zones skip a frame, from the zone tick times that the
    /// workers measure.
    pub pacer: Box<dyn Pacer>,
    /// Greatest number of `wait` calls that wait at the same time.
    pub max_waiters: usize,
    /// Greatest number of waiting calls for one resident.
    pub max_waiters_per_resident: usize,
    /// Number of frames in the window of the frame time percentiles.
    pub stats_window: usize,
    /// Length of the window of the input rate.
    pub rate_window: Duration,
    /// How long the coordinator waits for all workers to join.
    pub join_timeout: Duration,
    /// How long the coordinator waits for the answer of a worker. When the
    /// time passes, the cluster stops.
    pub reply_timeout: Duration,
    /// The storage of the global realm. `None`: nothing is recorded.
    pub durable: Option<ClusterDurable>,
}

impl ClusterConfig {
    /// Makes a configuration with manual stepping and `workers` workers
    /// that share the zones (see [`partition`]). Nothing is recorded.
    #[must_use]
    pub fn manual(world: WorldConfig, workers: usize) -> ClusterConfig {
        let s = settings();
        ClusterConfig {
            workers: partition(&world, workers),
            world,
            spawn_zone: ZoneId(0),
            tick_interval: Duration::ZERO,
            mailbox: s.runtime.mailbox,
            frame_buffer: s.runtime.frame_buffer,
            pacer: Box::new(NoDilation),
            max_waiters: s.runtime.max_waiters,
            max_waiters_per_resident: s.runtime.max_waiters_per_resident,
            stats_window: s.runtime.stats_window_frames,
            rate_window: Duration::from_millis(s.runtime.rate_window_ms),
            join_timeout: Duration::from_millis(s.cluster.join_timeout_ms),
            reply_timeout: Duration::from_millis(s.cluster.reply_timeout_ms),
            durable: None,
        }
    }

    /// Makes a configuration that steps one frame per `tick_interval`.
    #[must_use]
    pub fn real_time(world: WorldConfig, workers: usize, tick_interval: Duration) -> ClusterConfig {
        ClusterConfig { tick_interval, ..ClusterConfig::manual(world, workers) }
    }

    /// Checks the list of workers and returns the slot of each zone.
    pub(crate) fn zone_slots(&self) -> Result<Vec<usize>, ClusterError> {
        let zones = zone_count(&self.world);
        let mut slots: Vec<Option<usize>> = vec![None; zones];
        if self.workers.is_empty() {
            return Err(ClusterError::Config("the cluster has no worker".into()));
        }
        for (slot, list) in self.workers.iter().enumerate() {
            for zone in list {
                let Some(entry) = slots.get_mut(zone.0 as usize) else {
                    return Err(ClusterError::Config(format!(
                        "worker {slot} has zone {zone}, but the world has {zones} zones"
                    )));
                };
                if let Some(other) = entry.replace(slot) {
                    return Err(ClusterError::Config(format!(
                        "zone {zone} belongs to the workers {other} and {slot}"
                    )));
                }
            }
        }
        if (self.spawn_zone.0 as usize) >= zones {
            return Err(ClusterError::Config(format!(
                "the spawn zone {} does not exist; the world has {zones} zones",
                self.spawn_zone
            )));
        }
        slots
            .into_iter()
            .enumerate()
            .map(|(zone, slot)| {
                slot.ok_or_else(|| ClusterError::Config(format!("zone {zone} has no worker")))
            })
            .collect()
    }
}

impl fmt::Debug for ClusterConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClusterConfig")
            .field("workers", &self.workers)
            .field("spawn_zone", &self.spawn_zone)
            .field("tick_interval", &self.tick_interval)
            .field("mailbox", &self.mailbox)
            .field("join_timeout", &self.join_timeout)
            .field("reply_timeout", &self.reply_timeout)
            .field("durable", &self.durable)
            .finish_non_exhaustive()
    }
}

/// How a worker joins a cluster.
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// The world. It must be the one of the coordinator. The field
    /// `hosted` is not used.
    pub world: WorldConfig,
    /// The slot of this worker: its index in `ClusterConfig::workers`.
    pub slot: u32,
    /// The address of the coordinator.
    pub coordinator: SocketAddr,
    /// How long the worker tries to connect. The coordinator can start
    /// after the worker.
    pub connect_timeout: Duration,
    /// How long the worker waits for each answer of the coordinator in the
    /// handshake.
    pub reply_timeout: Duration,
    /// The storage of the zones of this worker. `None`: nothing is
    /// recorded.
    pub durable: Option<ClusterDurable>,
}

impl WorkerConfig {
    /// Makes the configuration of the worker in `slot` for a coordinator at
    /// `coordinator`. Nothing is recorded.
    #[must_use]
    pub fn new(world: WorldConfig, slot: u32, coordinator: SocketAddr) -> WorkerConfig {
        let s = &settings().cluster;
        WorkerConfig {
            world,
            slot,
            coordinator,
            connect_timeout: Duration::from_millis(s.connect_timeout_ms),
            reply_timeout: Duration::from_millis(s.reply_timeout_ms),
            durable: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use unsup_kernel::CityConfig;
    use unsup_protocol::Pos;

    use super::*;

    fn world(districts: u32) -> WorldConfig {
        WorldConfig {
            cities: vec![CityConfig::new("Ashford", Pos::new(0, 0), districts)],
            ..WorldConfig::default()
        }
    }

    fn zones(list: &[u32]) -> Vec<ZoneId> {
        list.iter().copied().map(ZoneId).collect()
    }

    #[test]
    fn zones_are_split_in_rows_of_numbers() {
        assert_eq!(
            partition(&world(3), 3),
            [zones(&[0, 1, 2]), zones(&[3, 4, 5]), zones(&[6, 7, 8])]
        );
        assert_eq!(partition(&world(2), 3), [zones(&[0, 1]), zones(&[2]), zones(&[3])]);
        assert_eq!(partition(&world(1), 2), [zones(&[0]), zones(&[])]);
        assert_eq!(partition(&world(2), 0), [zones(&[0, 1, 2, 3])]);
    }

    #[test]
    fn every_zone_needs_exactly_one_worker() {
        let config = ClusterConfig::manual(world(2), 3);
        assert_eq!(config.zone_slots().unwrap(), [0, 0, 1, 2]);

        let message = |workers: Vec<Vec<ZoneId>>, spawn_zone: u32| {
            let config = ClusterConfig {
                workers,
                spawn_zone: ZoneId(spawn_zone),
                ..ClusterConfig::manual(world(2), 1)
            };
            match config.zone_slots() {
                Err(ClusterError::Config(message)) => message,
                other => panic!("no configuration error: {other:?}"),
            }
        };
        assert_eq!(message(vec![], 0), "the cluster has no worker");
        assert_eq!(message(vec![zones(&[0, 1, 2])], 0), "zone 3 has no worker");
        assert_eq!(
            message(vec![zones(&[0, 1]), zones(&[1, 2, 3])], 0),
            "zone 1 belongs to the workers 0 and 1"
        );
        assert_eq!(
            message(vec![zones(&[0, 1, 2, 3, 4])], 0),
            "worker 0 has zone 4, but the world has 4 zones"
        );
        assert_eq!(
            message(vec![zones(&[0, 1, 2, 3])], 9),
            "the spawn zone 9 does not exist; the world has 4 zones"
        );
    }
}
