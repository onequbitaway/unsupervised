//! The result of a swarm run and its summary.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::preflight::{HttpConnections, Needs};
use super::server::{ServerStats, SpawnServer};
use super::signup::Flow;
use crate::bench::ProfileUse;
use crate::machine::FileLimits;
use crate::report::thousands;
use crate::stats::{Counts, LatencySummary};
use crate::think::ThinkTime;

/// How agents waited for the end of an action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitRoute {
    /// No agent waited over HTTP in this run.
    #[default]
    NotUsed,
    /// The long poll of the server (`GET /v1/wait`).
    LongPoll,
    /// The server has no long poll. Agents observed again and again.
    Polling,
}

/// Connections and limits of the machine.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Connections {
    /// The limits of the system.
    pub limits: FileLimits,
    /// What the run needed by the plan.
    pub needs: Option<Needs>,
    /// WebSocket agents by the plan.
    pub ws_agents_planned: u64,
    /// Greatest number of WebSocket connections that were open at the same
    /// time. This number is exact.
    pub ws_open_peak: u64,
    /// WebSocket connections that were opened in the whole run.
    pub ws_opened: u64,
    /// WebSocket agents that used HTTP because the server has no WebSocket
    /// route.
    pub ws_fallbacks: u64,
    /// Greatest number of agents that held an HTTP client of their own at
    /// the same time. Each keeps one connection open between requests.
    pub http_agents_peak: u64,
    /// Open files of this process before the first agent started.
    pub open_files_before: Option<u64>,
    /// Greatest number of open files of this process that the sampler saw,
    /// once each second. With a server in this process, each connection
    /// counts twice.
    pub open_files_peak: Option<u64>,
}

/// The result of a swarm run. The latencies and counts are those of the
/// measurement window, after the ramp-up, unless a field says otherwise.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SwarmRun {
    /// The address of the server.
    pub server: String,
    /// The server that the tool started in this process, if it did.
    pub spawned: Option<SpawnServer>,
    /// Number of agents.
    pub agents: u64,
    /// Seconds over which the agents started.
    pub ramp_up_secs: f64,
    /// Seconds of the measurement window.
    pub duration_secs: f64,
    /// Think time between two requests of an agent.
    pub think: Option<ThinkTime>,
    /// How the HTTP agents held connections.
    pub http_connections: HttpConnections,
    /// The profile.
    pub profile: ProfileUse,
    /// The sign-up flow that the server has. `None` when every agent used
    /// a stored key.
    pub signup_flow: Option<Flow>,
    /// Agents that used a key from the credentials file.
    pub credentials_reused: u64,
    /// Agents that signed up again because the server refused their key.
    pub credentials_renewed: u64,
    /// Agents that started.
    pub agents_started: u64,
    /// Agents that signed up, connected and got their first observation.
    pub agents_ready: u64,
    /// Agents that gave up, by the code of their last error.
    pub agents_failed: Counts,
    /// Connections and limits.
    pub connections: Connections,
    /// How agents waited for the end of an action over HTTP.
    pub wait_route: WaitRoute,
    /// Acknowledgement latency of all agents.
    pub ack: LatencySummary,
    /// Acknowledgement latency of the HTTP agents.
    pub ack_http: LatencySummary,
    /// Acknowledgement latency of the WebSocket agents.
    pub ack_ws: LatencySummary,
    /// Latency of observations.
    pub observe: LatencySummary,
    /// Time of one sign-up, in the whole run.
    pub signup: LatencySummary,
    /// Acknowledgements in the window.
    pub acks: u64,
    /// Acknowledgements for each second of the window.
    pub acks_per_second: f64,
    /// The smallest and the largest number of acknowledgements in one
    /// second of the window.
    pub acks_per_second_min: u64,
    /// See `acks_per_second_min`.
    pub acks_per_second_max: u64,
    /// Acknowledgements by state.
    pub ack_states: Counts,
    /// Actions that the game rejected, by error code. A rejection is a
    /// valid acknowledgement, not a failure of the server.
    pub rejections: Counts,
    /// Requests without a usable answer.
    pub failed_requests: u64,
    /// `failed_requests / (acks + observations + failed_requests)`.
    pub error_share: f64,
    /// Failed requests by code.
    pub errors: Counts,
    /// Failed requests outside the measurement window, by code: in the
    /// ramp-up, and while the run stops.
    pub errors_ramp: Counts,
    /// The message of the first failed request of each code, in the whole
    /// run.
    pub error_examples: BTreeMap<String, String>,
    /// Why the run stopped before its end, when it reached a limit of this
    /// machine: no free local port, or too many open files. Such a run
    /// says nothing about the server.
    pub machine_limit: Option<String>,
    /// Observations in the window.
    pub observations: u64,
    /// Waits for the end of an action in the window.
    pub waits: u64,
    /// Notices and reports that the server pushed on WebSockets, in the
    /// whole run.
    pub pushed_frames: u64,
    /// Statistics of the server.
    pub server_stats: ServerStats,
    /// Target for the 99th percentile of the acknowledgement latency, in
    /// milliseconds.
    pub p99_target_ms: f64,
    /// Greatest error share of a sustained run.
    pub max_error_share: f64,
    /// True when the server sustained the load: every agent became ready
    /// and stayed, the error share is at or below the limit, and the 99th
    /// percentile is at or below the target.
    pub sustained: bool,
    /// The reasons when `sustained` is false.
    pub not_sustained_because: Vec<String>,
    /// Messages for the operator.
    pub warnings: Vec<String>,
}

impl SwarmRun {
    /// Decides whether the server sustained the load and writes the
    /// reasons.
    pub fn judge(&mut self) {
        let mut reasons = Vec::new();
        if let Some(limit) = &self.machine_limit {
            reasons.push(limit.clone());
        }
        if self.agents_ready < self.agents {
            reasons.push(format!(
                "only {} of {} agents became ready",
                thousands(self.agents_ready),
                thousands(self.agents)
            ));
        }
        let gave_up: u64 = self.agents_failed.values().sum();
        if gave_up > 0 {
            reasons.push(format!(
                "{} agents gave up: {:?}",
                thousands(gave_up),
                self.agents_failed
            ));
        }
        if self.acks == 0 {
            reasons.push("no action was acknowledged in the window".to_string());
        }
        if self.error_share > self.max_error_share {
            reasons.push(format!(
                "{:.3} % of the requests failed (limit {:.3} %): {:?}",
                self.error_share * 100.0,
                self.max_error_share * 100.0,
                self.errors
            ));
        }
        if self.ack.p99_ms > self.p99_target_ms {
            reasons.push(format!(
                "the 99th percentile of the acknowledgement latency is {:.1} ms (target {:.1} ms)",
                self.ack.p99_ms, self.p99_target_ms
            ));
        }
        self.sustained = reasons.is_empty();
        self.not_sustained_because = reasons;
    }

    /// Returns the summary of the run.
    #[must_use]
    pub fn summary(&self) -> String {
        let c = &self.connections;
        let latency = |name: &str, l: &LatencySummary| {
            format!(
                "  {name:<12} {:>9} samples   p50 {:>8.2} ms   p95 {:>8.2}   p99 {:>8.2}   max {:>8.2}\n",
                thousands(l.count),
                l.p50_ms,
                l.p95_ms,
                l.p99_ms,
                l.max_ms
            )
        };
        let optional = |n: Option<u64>| n.map_or_else(|| "unknown".to_string(), thousands);
        let mut out = format!(
            "{} agents against {}{}\n",
            thousands(self.agents),
            self.server,
            match &self.spawned {
                Some(s) => format!(
                    " (started in this process: {}, {} ms for each tick)",
                    s.shape.describe(),
                    s.tick_ms
                ),
                None => String::new(),
            }
        );
        out.push_str(&format!("  {}\n", self.profile.line()));
        out.push_str(&format!(
            "  ramp-up {:.0} s, window {:.0} s, think time {}, HTTP connections {}\n",
            self.ramp_up_secs,
            self.duration_secs,
            self.think.map_or_else(|| "unknown".to_string(), |t| t.to_string()),
            self.http_connections,
        ));
        out.push_str(&format!(
            "  agents: {} started, {} ready, {} gave up; sign-up: {}{}\n",
            thousands(self.agents_started),
            thousands(self.agents_ready),
            thousands(self.agents_failed.values().sum()),
            match self.signup_flow {
                Some(Flow::Documented) => "POST /v1/owners and POST /v1/residents",
                Some(Flow::Dev) => "POST /v1/dev/residents (the server has no /v1/owners)",
                None => "none",
            },
            if self.credentials_reused > 0 {
                format!(", {} stored keys reused", thousands(self.credentials_reused))
            } else {
                String::new()
            }
        ));
        out.push_str(&format!(
            "  connections: {} WebSockets open at the peak of {} planned ({} used HTTP: no WebSocket route), {} agents with an HTTP connection of their own\n",
            thousands(c.ws_open_peak),
            thousands(c.ws_agents_planned),
            thousands(c.ws_fallbacks),
            thousands(c.http_agents_peak),
        ));
        out.push_str(&format!(
            "  open files: limit {} (hard {}, kernel cap {}), {} before the run, {} at the peak; local ports {}\n",
            c.limits.soft.map_or_else(|| "unknown".to_string(), |l| l.to_string()),
            c.limits.hard.map_or_else(|| "unknown".to_string(), |l| l.to_string()),
            optional(c.limits.per_process_cap),
            optional(c.open_files_before),
            optional(c.open_files_peak),
            optional(c.limits.ephemeral_ports),
        ));
        out.push_str(&latency("ack (all)", &self.ack));
        out.push_str(&latency("ack (HTTP)", &self.ack_http));
        out.push_str(&latency("ack (WS)", &self.ack_ws));
        out.push_str(&latency("observe", &self.observe));
        out.push_str(&format!(
            "  throughput: {} acknowledgements, {:.1} for each second (least {}, most {} in one second)\n",
            thousands(self.acks),
            self.acks_per_second,
            thousands(self.acks_per_second_min),
            thousands(self.acks_per_second_max),
        ));
        out.push_str(&format!("  acknowledgements by state: {:?}\n", self.ack_states));
        if !self.rejections.is_empty() {
            out.push_str(&format!("  rejected by the game, by code: {:?}\n", self.rejections));
        }
        out.push_str(&format!(
            "  failed requests: {} ({:.3} %){}\n",
            thousands(self.failed_requests),
            self.error_share * 100.0,
            if self.errors.is_empty() {
                String::new()
            } else {
                format!(", by code: {:?}", self.errors)
            }
        ));
        if !self.errors_ramp.is_empty() {
            out.push_str(&format!(
                "  failed requests outside the window (ramp-up and stop): {:?}\n",
                self.errors_ramp
            ));
        }
        for (code, message) in &self.error_examples {
            out.push_str(&format!("  first failure with the code `{code}`: {message}\n"));
        }
        out.push_str(&format!(
            "  observations {}, waits for the end of an action {} ({}), pushed frames {}\n",
            thousands(self.observations),
            thousands(self.waits),
            match self.wait_route {
                WaitRoute::NotUsed => "no HTTP agent waited",
                WaitRoute::LongPoll => "long poll",
                WaitRoute::Polling => "no long poll on the server: repeated observations",
            },
            thousands(self.pushed_frames),
        ));
        match &self.server_stats.sim {
            Some(sim) => out.push_str(&format!(
                "  server: {} residents, {} frames in the window, mean frame {:.3} ms, worst frame since start {:.3} ms, {} inputs applied\n",
                thousands(sim.residents),
                thousands(sim.frames),
                sim.mean_frame_ms,
                sim.worst_frame_ms,
                thousands(sim.inputs_applied),
            )),
            None if self.server_stats.route_end.is_some() => {
                out.push_str("  server: statistics route answered; see the report file\n");
            }
            None if self.spawned.is_some() => {
                out.push_str("  server: no statistics (the window did not open)\n");
            }
            None => out.push_str("  server: no statistics (the server has no statistics route)\n"),
        }
        for warning in self.profile.warnings.iter().chain(&self.warnings) {
            out.push_str(&format!("  warning: {warning}\n"));
        }
        if self.sustained {
            out.push_str(&format!(
                "  result: SUSTAINED (p99 {:.1} ms at or below {:.0} ms, {:.3} % failed requests)\n",
                self.ack.p99_ms,
                self.p99_target_ms,
                self.error_share * 100.0
            ));
        } else {
            out.push_str("  result: NOT SUSTAINED\n");
            for reason in &self.not_sustained_because {
                out.push_str(&format!("    - {reason}\n"));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> SwarmRun {
        SwarmRun {
            agents: 10,
            agents_ready: 10,
            acks: 500,
            ack: LatencySummary { count: 500, p99_ms: 12.0, ..LatencySummary::default() },
            p99_target_ms: 250.0,
            max_error_share: 0.001,
            ..SwarmRun::default()
        }
    }

    #[test]
    fn a_clean_run_is_sustained() {
        let mut run = good();
        run.judge();
        assert!(run.sustained, "{:?}", run.not_sustained_because);
        assert!(run.not_sustained_because.is_empty());
        assert!(run.summary().contains("result: SUSTAINED"));
    }

    #[test]
    fn each_broken_rule_gives_its_reason() {
        let mut slow = good();
        slow.ack.p99_ms = 250.1;
        slow.judge();
        assert!(!slow.sustained);
        assert_eq!(slow.not_sustained_because.len(), 1);
        assert!(slow.not_sustained_because[0].contains("250.1 ms"));

        let mut failing = good();
        failing.failed_requests = 2;
        failing.error_share = 0.004;
        failing.errors.insert("overloaded".into(), 2);
        failing.judge();
        assert!(!failing.sustained);
        assert!(failing.not_sustained_because[0].contains("overloaded"));

        let mut missing = good();
        missing.agents_ready = 9;
        missing.agents_failed.insert("connect".into(), 1);
        missing.judge();
        assert_eq!(missing.not_sustained_because.len(), 2);
        assert!(missing.not_sustained_because[0].contains("9 of 10"));
        assert!(missing.summary().contains("result: NOT SUSTAINED"));

        let mut idle = good();
        idle.acks = 0;
        idle.judge();
        assert!(idle.not_sustained_because[0].contains("no action"));

        // A run that stopped at a limit of the machine is never sustained,
        // and the limit is the first reason.
        let mut limited = good();
        limited.machine_limit = Some("The run stopped at a limit of this machine: x".into());
        limited.error_examples.insert("no_local_port".into(), "os error 49".into());
        limited.judge();
        assert!(!limited.sustained);
        assert_eq!(
            limited.not_sustained_because,
            ["The run stopped at a limit of this machine: x"]
        );
        let summary = limited.summary();
        assert!(summary.contains("    - The run stopped at a limit of this machine: x"));
        assert!(summary.contains("first failure with the code `no_local_port`: os error 49"));

        // Exactly at the limits is still sustained.
        let mut edge = good();
        edge.ack.p99_ms = 250.0;
        edge.error_share = 0.001;
        edge.judge();
        assert!(edge.sustained, "{:?}", edge.not_sustained_because);
    }
}
