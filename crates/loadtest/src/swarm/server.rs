//! The server side of a swarm run: a server that the tool starts in this
//! process, and the statistics of the server.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_runtime::{SimHandle, SimStats};
use unsup_server::{Server, ServerConfig, StoreConfig, WorldShape};

use crate::local::Shape;

/// A server for the swarm to start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnServer {
    /// Seed of the world.
    pub seed: u64,
    /// Cities and districts of the world.
    pub shape: Shape,
    /// Real milliseconds for one game minute.
    pub tick_ms: u64,
}

/// Starts the server of the project on a free port of this machine. It is
/// the same server as the `unsupervised` binary: all game modules, the
/// runtime, the gateway.
///
/// # Errors
///
/// Fails when the server cannot start.
pub async fn spawn(config: &SpawnServer) -> anyhow::Result<Server> {
    let mut server_config = ServerConfig {
        bind: SocketAddr::from(([127, 0, 0, 1], 0)),
        tick_interval: Duration::from_millis(config.tick_ms),
        world: WorldShape {
            seed: config.seed,
            cities: config.shape.cities,
            districts: config.shape.districts,
        },
        web_dir: None,
        store: StoreConfig::Memory,
        ..ServerConfig::default()
    };
    // A swarm comes from one address and signs up thousands of residents.
    // The limits that protect a public server would refuse it, and the
    // test measures the server, not its limits. Zero turns a limit off.
    let limits = &mut server_config.gateway.limits;
    limits.residents_per_owner = 0;
    limits.key_requests_per_minute = 0;
    limits.address_requests_per_minute = 0;
    limits.signups_per_hour = 0;
    limits.waits_per_resident = 0;
    server_config.gateway.ws.connections_per_resident = 0;
    unsup_server::start(server_config).await.context("cannot start a server in this process")
}

/// The statistics of the simulation of a server in this process, for the
/// measurement window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SimWindow {
    /// Residents at the end of the window.
    pub residents: u64,
    /// Frames that ran in the window.
    pub frames: u64,
    /// Mean time of a frame in the window, in milliseconds.
    pub mean_frame_ms: f64,
    /// Longest frame since the server started, in milliseconds.
    pub worst_frame_ms: f64,
    /// Inputs (actions, spawns, frames) that the simulation applied in the
    /// window.
    pub inputs_applied: u64,
    /// Calls that waited in a long poll at the end of the window.
    pub waiting: u64,
    /// Inputs that the journal refused since the server started.
    pub journal_errors: u64,
}

impl SimWindow {
    /// Computes the window from the statistics at its start and its end.
    #[must_use]
    pub fn between(start: &SimStats, end: &SimStats) -> SimWindow {
        let frames = end.frames_run.saturating_sub(start.frames_run);
        // The server gives the mean since its start. The sum of all frame
        // times is the mean times the count.
        let total = |stats: &SimStats| stats.mean_frame_micros as f64 * stats.frames_run as f64;
        let mean_micros =
            if frames == 0 { 0.0 } else { ((total(end) - total(start)) / frames as f64).max(0.0) };
        SimWindow {
            residents: end.residents,
            frames,
            mean_frame_ms: mean_micros / 1e3,
            worst_frame_ms: end.worst_frame_micros as f64 / 1e3,
            inputs_applied: end.inputs_applied.saturating_sub(start.inputs_applied),
            waiting: end.waiting,
            journal_errors: end.journal_errors,
        }
    }
}

/// Reads the statistics of a simulation in this process. `None` when the
/// simulation does not answer.
pub async fn sim_stats(sim: &SimHandle) -> Option<SimStats> {
    sim.stats().await.ok()
}

/// The statistics of a server, as far as the swarm could read them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerStats {
    /// The simulation of a server in this process, for the window.
    pub sim: Option<SimWindow>,
    /// What the statistics route of the server answered at the start of
    /// the window, as it is. `None` when the server has no such route.
    pub route_start: Option<Value>,
    /// What the route answered at the end of the window.
    pub route_end: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_holds_only_its_own_frames() {
        let start = SimStats {
            frame: 10,
            residents: 5,
            inputs_applied: 100,
            frames_run: 10,
            mean_frame_micros: 1_000,
            worst_frame_micros: 4_000,
            ..SimStats::default()
        };
        let end = SimStats {
            frame: 30,
            residents: 8,
            inputs_applied: 700,
            frames_run: 30,
            // 10 frames of 1 ms and 20 frames of 4 ms: the mean is 3 ms.
            mean_frame_micros: 3_000,
            worst_frame_micros: 9_000,
            waiting: 2,
            ..SimStats::default()
        };
        let window = SimWindow::between(&start, &end);
        assert_eq!(window.frames, 20);
        assert_eq!(window.inputs_applied, 600);
        assert_eq!(window.residents, 8);
        assert_eq!(window.waiting, 2);
        assert!((window.mean_frame_ms - 4.0).abs() < 1e-9, "{}", window.mean_frame_ms);
        assert!((window.worst_frame_ms - 9.0).abs() < 1e-9);
        assert_eq!(SimWindow::between(&end, &end).frames, 0);
        assert!(SimWindow::between(&end, &end).mean_frame_ms.abs() < 1e-9);
    }
}
