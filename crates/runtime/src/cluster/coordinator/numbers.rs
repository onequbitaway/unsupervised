//! The numbers that the coordinator reports: those of a runtime, summed
//! over the cluster, and those that only a cluster has.

use std::sync::atomic::Ordering;

use super::super::session::add_journal;
use super::super::stats::{ClusterStats, WorkerStats};
use super::serve::Server;
use crate::stats::{SimStats, micros};

impl Server {
    pub(super) fn stats(&self) -> SimStats {
        let m = &self.meters;
        let mut journal = self.core.global.journal_stats();
        for worker in &self.core.journals {
            add_journal(&mut journal, worker);
        }
        let [p50, p95, p99] = self.frame_window.percentiles([50, 95, 99]);
        SimStats {
            frame: self.core.global.world.frame(),
            residents: self.core.residents.iter().map(|&count| u64::from(count)).sum(),
            inputs_applied: m.inputs_applied,
            frames_run: m.frames_run,
            mean_frame_micros: match u32::try_from(m.frames_run) {
                Ok(0) => 0,
                Ok(n) => micros(m.frame_total / n),
                Err(_) => micros(m.frame_total) / m.frames_run,
            },
            worst_frame_micros: micros(m.frame_worst),
            last_frame_micros: micros(m.frame_last),
            waiting: self.waiters.len() as u64,
            journal_errors: m.refused,
            p50_frame_micros: p50,
            p95_frame_micros: p95,
            p99_frame_micros: p99,
            inputs_per_second: self.input_rate.per_second(),
            mailbox_depth: self.counters.depth.load(Ordering::Relaxed) as u64,
            mailbox_capacity: self.mailbox_capacity as u64,
            overloaded: self.counters.mailbox_full.load(Ordering::Relaxed) + m.waiters_full,
            dilated_zones: self.pacer.dilated_zones() as u64,
            zone_ticks_skipped: m.zone_ticks_skipped,
            journal_records: journal.records,
            journal_lag: journal.lag,
            snapshots: journal.snapshots,
            snapshot_errors: journal.snapshot_errors,
            last_snapshot_frame: journal.last_snapshot_frame,
            last_snapshot_micros: journal.last_snapshot_capture_micros,
            last_snapshot_write_micros: journal.last_snapshot_write_micros,
            last_snapshot_bytes: journal.last_snapshot_bytes,
            // A cluster has no rollback, and no batches: the times of the
            // phases of a frame are in `ClusterStats`.
            ..SimStats::default()
        }
    }

    pub(super) fn cluster_stats(&self) -> ClusterStats {
        let m = &self.meters;
        let core = &self.core;
        let workers = core
            .links
            .iter()
            .enumerate()
            .map(|(slot, link)| WorkerStats {
                slot: slot as u32,
                zones: core.zones_of[slot].clone(),
                residents: core.zones_of[slot]
                    .iter()
                    .map(|zone| {
                        u64::from(core.residents.get(zone.0 as usize).copied().unwrap_or(0))
                    })
                    .sum(),
                bytes_sent: link.sent,
                bytes_received: link.received(),
                last_step_micros: m.last.workers.get(slot).copied().map_or(0, micros),
            })
            .collect();
        ClusterStats {
            frame: core.global.world.frame(),
            epoch: core.next_epoch.saturating_sub(1),
            workers,
            migrations: core.routed.migrations,
            messages: core.routed.messages,
            dropped_messages: core.dropped_messages(),
            last_zones_micros: micros(m.last.zones),
            last_global_micros: micros(m.last.global),
            last_exchange_micros: micros(m.last.exchange),
            zones_micros: micros(m.zones_total),
            global_micros: micros(m.global_total),
            exchange_micros: micros(m.exchange_total),
        }
    }
}
