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
    /// Frame time that 99 of 100 of the last frames stayed below, at the
    /// end of the window, in milliseconds.
    #[serde(default)]
    pub p99_frame_ms: f64,
    /// Longest frame since the server started, in milliseconds.
    pub worst_frame_ms: f64,
    /// Inputs (actions, spawns, frames) that the simulation applied in the
    /// window.
    pub inputs_applied: u64,
    /// Time in the window that the simulation thread spent in the world
    /// between frames: it applied inputs and built observations. In
    /// milliseconds.
    #[serde(default)]
    pub apply_ms: f64,
    /// Time in the window that the simulation thread spent on the calls
    /// themselves: it took them from the mailbox, waited for the journal
    /// and gave the answers back. In milliseconds.
    #[serde(default)]
    pub calls_ms: f64,
    /// Calls in the window that the simulation refused with `overloaded`
    /// because its mailbox was full.
    #[serde(default)]
    pub overloaded: u64,
    /// Calls that waited in the mailbox of the simulation thread at the
    /// end of the window.
    #[serde(default)]
    pub mailbox_depth: u64,
    /// Greatest number of calls that the mailbox holds.
    #[serde(default)]
    pub mailbox_capacity: u64,
    /// Calls that waited in a long poll at the end of the window.
    pub waiting: u64,
    /// Inputs that the journal refused since the server started.
    pub journal_errors: u64,
    /// Observations that the simulation thread built in the window, for
    /// `observe` calls and for `wait` calls.
    #[serde(default)]
    pub observations: u64,
    /// Steps in which the simulation thread built them. One step builds
    /// the observations of all `observe` calls of one batch of calls on
    /// all processor cores, or the observation of one `wait` call.
    #[serde(default)]
    pub observe_steps: u64,
    /// Time in the window that the simulation thread spent in these
    /// steps, in milliseconds. The part for `observe` calls is also in
    /// `apply_ms`. The part for `wait` calls that a frame woke is in no
    /// other number.
    #[serde(default)]
    pub observe_ms: f64,
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
        let calls = |stats: &SimStats| {
            stats
                .intake_micros
                .saturating_add(stats.commit_micros)
                .saturating_add(stats.reply_micros)
        };
        let ms = |micros: u64| micros as f64 / 1e3;
        SimWindow {
            residents: end.residents,
            frames,
            mean_frame_ms: mean_micros / 1e3,
            p99_frame_ms: ms(end.p99_frame_micros),
            worst_frame_ms: ms(end.worst_frame_micros),
            inputs_applied: end.inputs_applied.saturating_sub(start.inputs_applied),
            apply_ms: ms(end.apply_micros.saturating_sub(start.apply_micros)),
            calls_ms: ms(calls(end).saturating_sub(calls(start))),
            overloaded: end.overloaded.saturating_sub(start.overloaded),
            mailbox_depth: end.mailbox_depth,
            mailbox_capacity: end.mailbox_capacity,
            waiting: end.waiting,
            journal_errors: end.journal_errors,
            observations: end.observations.saturating_sub(start.observations),
            observe_steps: end.observe_batches.saturating_sub(start.observe_batches),
            observe_ms: ms(end.observe_micros.saturating_sub(start.observe_micros)),
        }
    }

    /// Returns the time in the window that the one simulation thread
    /// worked and that the runtime counts, in milliseconds: the frames,
    /// the inputs and observations between the frames, and the calls
    /// themselves. The runtime does not count the time in which the thread
    /// answers long polls after a frame, so the thread worked at least
    /// this long. A value near the length of the window means that the
    /// thread had no time left.
    #[must_use]
    pub fn thread_ms(&self) -> f64 {
        self.mean_frame_ms * self.frames as f64 + self.apply_ms + self.calls_ms
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
            apply_micros: 2_000,
            intake_micros: 100,
            commit_micros: 200,
            reply_micros: 300,
            overloaded: 1,
            observations: 40,
            observe_batches: 10,
            observe_micros: 500,
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
            p99_frame_micros: 8_000,
            waiting: 2,
            apply_micros: 52_000,
            intake_micros: 1_100,
            commit_micros: 1_200,
            reply_micros: 1_300,
            overloaded: 4,
            mailbox_depth: 6,
            mailbox_capacity: 1_024,
            observations: 940,
            observe_batches: 40,
            observe_micros: 7_500,
            ..SimStats::default()
        };
        let window = SimWindow::between(&start, &end);
        assert_eq!(window.frames, 20);
        assert_eq!(window.inputs_applied, 600);
        assert_eq!(window.residents, 8);
        assert_eq!(window.waiting, 2);
        assert!((window.mean_frame_ms - 4.0).abs() < 1e-9, "{}", window.mean_frame_ms);
        assert!((window.worst_frame_ms - 9.0).abs() < 1e-9);
        assert!((window.p99_frame_ms - 8.0).abs() < 1e-9);
        // 50 ms in the world between frames, and 3 ms for the calls.
        assert!((window.apply_ms - 50.0).abs() < 1e-9, "{}", window.apply_ms);
        assert!((window.calls_ms - 3.0).abs() < 1e-9, "{}", window.calls_ms);
        assert_eq!(window.overloaded, 3);
        assert_eq!((window.mailbox_depth, window.mailbox_capacity), (6, 1_024));
        // 900 observations in 30 steps, which took 7 ms.
        assert_eq!((window.observations, window.observe_steps), (900, 30));
        assert!((window.observe_ms - 7.0).abs() < 1e-9, "{}", window.observe_ms);
        // 20 frames of 4 ms, 50 ms and 3 ms.
        assert!((window.thread_ms() - 133.0).abs() < 1e-9, "{}", window.thread_ms());
        assert_eq!(SimWindow::between(&end, &end).frames, 0);
        assert!(SimWindow::between(&end, &end).mean_frame_ms.abs() < 1e-9);
        assert!(SimWindow::between(&end, &end).thread_ms().abs() < 1e-9);
    }
}
