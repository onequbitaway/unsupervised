//! `bench`: frame time, action rate and memory of a world in one process,
//! without the network.

mod memory;
mod run;
mod scale;

use serde::{Deserialize, Serialize};

pub use memory::{Memory, default_memory_limit};
pub use run::{BenchConfig, BenchRun, ProfileUse, run};
pub use scale::{ScaleFailure, ScaleOptions, parse_size, run_scale};

use crate::report::{mib, thousands};

/// The result of a `bench` command: one run for each size.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchReport {
    /// The sizes that ran, in the order of the command line.
    pub runs: Vec<BenchRun>,
    /// The sizes that did not finish, with the reason.
    pub failures: Vec<ScaleFailure>,
}

impl BenchReport {
    /// Returns the summary that a person reads.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut out = String::new();
        for run in &self.runs {
            out.push_str(&run.summary());
            out.push('\n');
        }
        if self.runs.len() > 1 {
            out.push_str(
                "residents | zones | frame ms mean / p50 / p99 / worst | inputs ms mean | \
                 observe ms mean | server ms mean / worst | actions per frame | \
                 actions per wall second | rejected | microseconds per request / observation | \
                 RSS | bytes per resident | footprint | footprint bytes per resident\n",
            );
            let bytes = |b: Option<u64>| b.map_or_else(|| "unknown".to_string(), mib);
            let count = |n: Option<u64>| n.map_or_else(|| "unknown".to_string(), thousands);
            for run in &self.runs {
                out.push_str(&format!(
                    "{} | {} | {:.2} / {:.2} / {:.2} / {:.2} | {:.2} | {:.2} | {:.2} / {:.2} | {} | {} | {:.2} % | {:.2} / {:.2} | {} | {} | {} | {}{}\n",
                    thousands(run.residents),
                    run.zones,
                    run.frame_ms.mean,
                    run.frame_ms.p50,
                    run.frame_ms.p99,
                    run.frame_ms.max,
                    run.inputs_ms.mean,
                    run.observe_ms.mean,
                    run.server_ms.mean,
                    run.server_ms.max,
                    thousands(run.actions_per_frame.round() as u64),
                    thousands(run.actions_per_wall_second.round() as u64),
                    run.actions.rejection_rate() * 100.0,
                    run.micros_per_request,
                    run.micros_per_observation,
                    bytes(run.memory.rss_end_bytes),
                    count(run.memory.bytes_per_resident),
                    bytes(run.memory.footprint_end_bytes),
                    count(run.memory.footprint_bytes_per_resident),
                    if run.memory.under_pressure { " | UNDER MEMORY PRESSURE" } else { "" },
                ));
            }
        }
        for failure in &self.failures {
            out.push_str(&format!(
                "{} residents: NOT MEASURED. {}\n",
                thousands(failure.residents),
                failure.reason
            ));
        }
        out
    }
}
