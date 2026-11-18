//! The memory of a benchmark run: what the process holds, the limit that
//! stops a run, and how full the inboxes of the residents are.

use std::time::Duration;

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use unsup_kernel::World;
use unsup_protocol::{ObserveOpts, ResidentId};

use crate::defaults::defaults;
use crate::machine::{self, GIB};
use crate::process::{self, MemorySampler};
use crate::report::{mib, thousands};

/// Returns the default limit for the memory that a run can hold: the
/// smaller of a share of the installed memory and a share of the memory
/// that is free now. `None` when the system tells neither.
#[must_use]
pub fn default_memory_limit(installed: Option<u64>) -> Option<u64> {
    let d = &defaults().bench;
    let share = |bytes: u64, percent: u64| bytes / 100 * percent;
    let of_installed = installed.map(|bytes| share(bytes, d.max_memory_percent_of_installed));
    let of_available =
        machine::available_memory_bytes().map(|b| share(b, d.max_memory_percent_of_available));
    match (of_installed, of_available) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Memory of the process during a run. Every number is `None` when the
/// system gives no answer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Memory {
    /// Resident set size before the world was built.
    pub rss_baseline_bytes: Option<u64>,
    /// Resident set size after the residents were created.
    pub rss_populated_bytes: Option<u64>,
    /// Resident set size after the warm-up, when the measurement starts.
    pub rss_warm_bytes: Option<u64>,
    /// Resident set size after the last frame. When it is much greater than
    /// `rss_warm_bytes`, the memory was still growing: the inboxes of the
    /// residents were not full yet.
    pub rss_end_bytes: Option<u64>,
    /// Largest resident set size that the sampler saw.
    pub rss_peak_bytes: Option<u64>,
    /// `(rss_end - rss_baseline) / residents`: what one more resident
    /// costs, with the share of the map, the driver and the allocator.
    pub bytes_per_resident: Option<u64>,
    /// Memory that the process owned after the last frame, with the pages
    /// that the system compressed or swapped. The resident set size does
    /// not count those pages.
    pub footprint_end_bytes: Option<u64>,
    /// Largest footprint since the process started.
    pub footprint_peak_bytes: Option<u64>,
    /// `(footprint_end - rss_baseline) / residents`. Use this number when
    /// the system compressed or swapped memory of the run.
    pub footprint_bytes_per_resident: Option<u64>,
    /// `footprint_end - rss_end`: memory of the run that the system
    /// compressed or swapped. Zero when the footprint is not greater.
    pub squeezed_bytes: Option<u64>,
    /// True when the system compressed or swapped more than a small share
    /// of the memory of the run. The times of such a run are too high and
    /// its resident set size is too low.
    pub under_pressure: bool,
    /// The estimate of the kernel (`World::memory_estimate`): the resident
    /// tables only.
    pub kernel_estimate_bytes_per_resident: u64,
    /// How full the inboxes of the residents were after the last frame,
    /// from 0 to 1, in a sample of residents. An inbox keeps the last
    /// `kernel.inbox_max` notices and reports. The memory of a world grows
    /// until the inboxes are full; below 1 the run did not reach that.
    pub inbox_fill: Option<f64>,
    /// The limit of this run, in bytes. The run stops when the process
    /// holds more: the larger of its resident set size and its footprint.
    pub limit_bytes: Option<u64>,
}

impl Memory {
    /// Returns the lines of the summary that tell the memory.
    #[must_use]
    pub fn lines(&self) -> String {
        let bytes = |b: Option<u64>| b.map_or_else(|| "unknown".to_string(), mib);
        let count = |n: Option<u64>| n.map_or_else(|| "unknown".to_string(), thousands);
        let mut out = format!(
            "  memory: RSS {} at the end, {} at the peak, {} when the measurement started, {} before the world; {} bytes for each resident (kernel estimate {})\n",
            bytes(self.rss_end_bytes),
            bytes(self.rss_peak_bytes),
            bytes(self.rss_warm_bytes),
            bytes(self.rss_baseline_bytes),
            count(self.bytes_per_resident),
            thousands(self.kernel_estimate_bytes_per_resident),
        );
        out.push_str(&format!(
            "  footprint (RSS with compressed and swapped pages): {} at the end, {} at the peak; {} bytes for each resident\n",
            bytes(self.footprint_end_bytes),
            bytes(self.footprint_peak_bytes),
            count(self.footprint_bytes_per_resident),
        ));
        if self.under_pressure {
            out.push_str(&format!(
                "  WARNING: the system compressed or swapped {} of the memory of this run. The times are too high and the RSS is too low. Use fewer residents, or a machine with more free memory\n",
                bytes(self.squeezed_bytes),
            ));
        }
        if let Some(fill) = self.inbox_fill {
            out.push_str(&format!(
                "  inboxes: {:.0} % full{}\n",
                fill * 100.0,
                if fill < 0.95 {
                    ": the memory was still growing. Use a longer warm-up for the memory of a world that has run for a while"
                } else {
                    ""
                }
            ));
        }
        out
    }
}

/// Returns how full the inboxes are, from 0 to 1: the mean number of
/// notices and reports that a sample of residents holds, over
/// `kernel.inbox_max`. `None` when the world does not have the parameter.
fn inbox_fill(world: &World, residents: &[ResidentId], sample: usize) -> Option<f64> {
    let catalogue = world.catalogue();
    let max = catalogue.params.iter().find(|param| param.name == "kernel.inbox_max")?.value;
    if max <= 0 || residents.is_empty() || sample == 0 {
        return None;
    }
    let step = (residents.len() / sample).max(1);
    let opts =
        ObserveOpts { sections: Some(vec!["reports".to_string()]), ..ObserveOpts::default() };
    let (mut entries, mut seen) = (0usize, 0usize);
    for &id in residents.iter().step_by(step) {
        if let Ok(observation) = world.observe(id, &opts) {
            let reports = observation.sections.get("reports").and_then(|r| r.as_array());
            entries += observation.notices.len() + reports.map_or(0, Vec::len);
            seen += 1;
        }
    }
    (seen > 0).then(|| entries as f64 / seen as f64 / max as f64)
}

/// Returns the memory of a process that the system compressed or swapped
/// (`footprint - rss`, zero when the footprint is not greater), and whether
/// that is more than `percent` percent of the footprint.
fn squeezed(rss: Option<u64>, footprint: Option<u64>, percent: u64) -> (Option<u64>, bool) {
    let (Some(rss), Some(footprint)) = (rss, footprint) else {
        return (None, false);
    };
    let squeezed = footprint.saturating_sub(rss);
    let pressure = u128::from(squeezed) * 100 > u128::from(footprint) * u128::from(percent);
    (Some(squeezed), pressure)
}

/// Watches the memory of one run: it samples the memory of the process,
/// stops the run at the limit, and makes the [`Memory`] of the result.
pub(super) struct Probe {
    sampler: MemorySampler,
    limit: Option<u64>,
    baseline: Option<u64>,
    populated: Option<u64>,
    warm: Option<u64>,
}

impl Probe {
    /// Starts to watch. Call it before the world is built.
    /// `max_memory_bytes` is the limit of the caller; without it the
    /// default limit applies.
    pub(super) fn start(max_memory_bytes: Option<u64>) -> Probe {
        let d = &defaults().bench;
        let baseline = process::own_rss_bytes();
        let limit = max_memory_bytes
            .or_else(|| default_memory_limit(machine::Machine::detect().memory_bytes));
        let sampler = MemorySampler::start(
            Duration::from_millis(d.memory_sample_ms.max(1)),
            d.footprint_sample_every,
            limit,
        );
        Probe { sampler, limit, baseline, populated: None, warm: None }
    }

    /// Returns the limit for the memory that the process holds, in bytes.
    pub(super) fn limit(&self) -> Option<u64> {
        self.limit
    }

    /// Notes the memory after the residents were created.
    pub(super) fn populated(&mut self) {
        self.populated = process::own_rss_bytes();
    }

    /// Notes the memory after the warm-up.
    pub(super) fn warm(&mut self) {
        self.warm = process::own_rss_bytes();
    }

    /// Stops the run with a clear message when the process holds more than
    /// the limit, before the machine swaps much. `frames_done` counts
    /// warm-up and measured frames.
    pub(super) fn check(&self, frames_done: u64, total_frames: u64) -> anyhow::Result<()> {
        match (self.sampler.over_limit(), self.limit) {
            (Some(held), Some(limit)) => Err(anyhow!(
                "The run stopped after {frames_done} of {total_frames} frames: the process held \
                 {:.1} GiB, more than the limit of {:.1} GiB. The memory of a world grows until \
                 the inboxes of the residents are full. Use fewer residents or fewer frames, or \
                 set the limit with --max-memory-gib.",
                held as f64 / GIB,
                limit as f64 / GIB
            )),
            _ => Ok(()),
        }
    }

    /// Reads the memory after the last frame and makes the result.
    pub(super) fn finish(self, world: &World, residents: &[ResidentId]) -> Memory {
        let d = &defaults().bench;
        let rss_end = process::own_rss_bytes();
        let footprint = process::own_footprint();
        let rss_peak = self.sampler.finish().rss;
        let count = residents.len() as u64;
        let baseline = self.baseline;
        let per_resident = |end: Option<u64>| match (end, baseline) {
            (Some(end), Some(base)) if count > 0 => Some(end.saturating_sub(base) / count),
            _ => None,
        };
        let footprint_end = footprint.map(|f| f.bytes);
        let (squeezed, under_pressure) = squeezed(rss_end, footprint_end, d.pressure_percent);
        Memory {
            rss_baseline_bytes: baseline,
            rss_populated_bytes: self.populated,
            rss_warm_bytes: self.warm,
            rss_end_bytes: rss_end,
            rss_peak_bytes: rss_peak,
            bytes_per_resident: per_resident(rss_end),
            footprint_end_bytes: footprint_end,
            footprint_peak_bytes: footprint.and_then(|f| f.peak_bytes),
            footprint_bytes_per_resident: per_resident(footprint_end),
            squeezed_bytes: squeezed,
            under_pressure,
            kernel_estimate_bytes_per_resident: world.memory_estimate().per_resident() as u64,
            inbox_fill: inbox_fill(world, residents, d.inbox_sample),
            limit_bytes: self.limit,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_pressure_starts_above_the_given_share_of_the_footprint() {
        // 5 of 100 bytes are squeezed: exactly 5 % is not yet pressure.
        assert_eq!(squeezed(Some(95), Some(100), 5), (Some(5), false));
        assert_eq!(squeezed(Some(94), Some(100), 5), (Some(6), true));
        // Half of the memory of a large run is compressed.
        assert_eq!(squeezed(Some(15 << 30), Some(30 << 30), 5), (Some(15 << 30), true));
        // A small process: shared pages make the resident set larger than
        // the footprint. Nothing is squeezed.
        assert_eq!(squeezed(Some(300), Some(200), 5), (Some(0), false));
        assert_eq!(squeezed(None, Some(200), 5), (None, false));
        assert_eq!(squeezed(Some(200), None, 5), (None, false));
    }

    #[test]
    fn the_summary_warns_when_the_system_squeezed_the_memory_of_the_run() {
        let calm = Memory {
            rss_end_bytes: Some(200 << 20),
            footprint_end_bytes: Some(201 << 20),
            squeezed_bytes: Some(1 << 20),
            inbox_fill: Some(1.0),
            ..Memory::default()
        };
        let lines = calm.lines();
        assert!(lines.contains("RSS 200.0 MiB at the end"), "{lines}");
        assert!(lines.contains("pages): 201.0 MiB at the end"), "{lines}");
        assert!(!lines.contains("WARNING"), "{lines}");
        assert!(lines.contains("inboxes: 100 % full\n"), "{lines}");

        let squeezed = Memory {
            squeezed_bytes: Some(100 << 20),
            under_pressure: true,
            inbox_fill: Some(0.5),
            ..calm
        };
        let lines = squeezed.lines();
        assert!(lines.contains("WARNING: the system compressed or swapped 100.0 MiB"), "{lines}");
        assert!(lines.contains("inboxes: 50 % full: the memory was still growing"), "{lines}");
    }
}
