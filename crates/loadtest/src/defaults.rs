//! The default numbers of the load tools.
//!
//! Every tunable number of this crate lives in `data/defaults.toml`. The
//! file is part of the binary. An option on the command line replaces its
//! default.

use std::sync::OnceLock;

use serde::Deserialize;

/// All defaults, one table for each topic.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Defaults {
    /// Size and seed of a world that a tool builds.
    pub world: WorldDefaults,
    /// The driver that makes residents act in one process.
    pub driver: DriverDefaults,
    /// The `bench` command.
    pub bench: BenchDefaults,
    /// The `soak` command.
    pub soak: SoakDefaults,
    /// The `swarm` command.
    pub swarm: SwarmDefaults,
}

/// Defaults for the size of a world.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldDefaults {
    /// Seed of the world and of the behaviour of the residents.
    pub seed: u64,
    /// Residents for each zone when the tool chooses the number of zones.
    pub residents_per_zone: u32,
    /// Greatest number of districts on one side of a city.
    pub max_districts: u32,
    /// Allowed distance between the wanted and the real number of zones, in
    /// percent.
    pub shape_tolerance_percent: u32,
    /// Tile kinds on which a resident cannot stand.
    pub blocked_tiles: Vec<u8>,
    /// Tries to find a walkable tile for one resident of the scatter
    /// placement.
    pub scatter_attempts: u32,
    /// Residents of one owner in a world that a tool fills.
    pub residents_per_owner: u64,
}

/// Defaults of the driver.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DriverDefaults {
    /// Tries to find an action whose arguments can be made.
    pub pick_attempts: u32,
    /// Ticks before the driver looks again at a resident whose action waits
    /// for the global realm or for the end of a walk to another zone.
    pub pending_recheck_ticks: u32,
    /// Tries to find a walkable tile for one walk target.
    pub tile_attempts: u32,
    /// Number of nearest sites of a kind among which a resident chooses.
    pub site_choices: usize,
}

/// Defaults of `bench`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BenchDefaults {
    /// Number of residents.
    pub residents: u32,
    /// Frames that are measured.
    pub frames: u32,
    /// Frames that run before the measurement.
    pub warmup_frames: u32,
    /// Milliseconds between two samples of the resident set size.
    pub memory_sample_ms: u64,
    /// The footprint is read at every n-th sample.
    pub footprint_sample_every: u32,
    /// The run stops above this many percent of the installed memory.
    pub max_memory_percent_of_installed: u64,
    /// The run stops above this many percent of the memory that was free
    /// when it started.
    pub max_memory_percent_of_available: u64,
    /// Residents that are asked how full their inbox is.
    pub inbox_sample: usize,
    /// Percent of the memory of a run that the system can compress or swap
    /// before the run counts as under memory pressure.
    pub pressure_percent: u64,
    /// Seconds between two progress lines of a long run.
    pub progress_secs: u64,
    /// Greatest number of requests in one call of `World::observe_batch`.
    /// 0: one `World::observe` for each observation, on one thread.
    pub observe_batch: usize,
}

/// Defaults of `soak`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoakDefaults {
    /// Number of residents.
    pub residents: u32,
    /// Number of ticks.
    pub ticks: u64,
    /// Ticks between two audits of the whole world.
    pub audit_every: u64,
    /// Ticks between two comparisons of the state hash.
    pub hash_every: u64,
}

/// Defaults of `swarm`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwarmDefaults {
    /// Number of agents.
    pub agents: u32,
    /// Seconds over which the agents start.
    pub ramp_up_secs: f64,
    /// Seconds of the measurement after the ramp-up.
    pub duration_secs: f64,
    /// Name of the think time distribution.
    pub think: String,
    /// Mean think time in seconds.
    pub think_mean_secs: f64,
    /// Shape of the log-normal distribution.
    pub think_sigma: f64,
    /// Share of the agents that use the WebSocket.
    pub ws_share: f64,
    /// Seconds after which a request counts as failed.
    pub request_timeout_secs: f64,
    /// Seconds that an agent waits after a failed request.
    pub retry_secs: f64,
    /// Failed requests in a row after which an agent gives up.
    pub max_failures_in_row: u32,
    /// Greatest number of seconds of one long poll.
    pub wait_timeout_secs: f64,
    /// Zones whose map the tool reads at the same time.
    pub map_requests: usize,
    /// Real milliseconds for one game minute of a server that the tool
    /// starts.
    pub tick_ms: u64,
    /// Target for the 99th percentile of the acknowledgement latency, in
    /// milliseconds.
    pub p99_target_ms: f64,
    /// Greatest share of failed requests of a sustained run.
    pub max_error_share: f64,
    /// Open files that the process needs beside its connections.
    pub fd_slack: u64,
    /// Greatest latency that the histogram can hold, in seconds.
    pub histogram_max_secs: u64,
    /// Significant digits of the histogram.
    pub histogram_sigfig: u8,
    /// Path of the route with the statistics of the server.
    pub stats_path: String,
}

/// Returns the defaults.
///
/// # Panics
///
/// Panics when `data/defaults.toml` does not parse. The file is part of the
/// binary and a unit test reads it, so a released binary cannot fail here.
#[must_use]
pub fn defaults() -> &'static Defaults {
    static DEFAULTS: OnceLock<Defaults> = OnceLock::new();
    DEFAULTS.get_or_init(|| match toml::from_str(include_str!("../data/defaults.toml")) {
        Ok(defaults) => defaults,
        Err(error) => panic!("data/defaults.toml is not valid: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_file_parses_and_gives_usable_numbers() {
        let d = defaults();
        assert!(d.world.residents_per_zone > 0);
        assert!(d.world.max_districts > 0);
        assert!(d.driver.pick_attempts > 0);
        assert!(d.driver.tile_attempts > 0);
        assert!(d.driver.site_choices > 0);
        assert!(!d.world.blocked_tiles.is_empty());
        assert!(d.bench.frames > 0);
        assert!(d.soak.audit_every > 0);
        assert!(d.soak.hash_every > 0);
        assert!((0.0..=1.0).contains(&d.swarm.ws_share));
        assert!(d.swarm.histogram_max_secs > 0);
        assert!((1..=5).contains(&d.swarm.histogram_sigfig));
    }
}
