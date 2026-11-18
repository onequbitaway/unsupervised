//! Checks before a swarm starts: does the machine allow this many
//! connections?
//!
//! A run that needs more open files or more local ports than the system
//! gives would fail in the middle with errors that are hard to read. The
//! swarm stops before it starts, with a message that says what to change.

use serde::{Deserialize, Serialize};

use super::error::{NO_LOCAL_PORT, RequestError};
use crate::machine::FileLimits;
use crate::report::thousands;

/// Local ports that the system and other programs may use at the same time.
const PORT_MARGIN: u64 = 512;

/// How the HTTP agents hold connections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpConnections {
    /// Each agent has its own client and keeps one connection open, like a
    /// real agent.
    #[default]
    PerAgent,
    /// All HTTP agents share one pool of connections. A connection is open
    /// only while a request needs it, so many more agents fit on one
    /// machine.
    Shared,
}

impl std::str::FromStr for HttpConnections {
    type Err = String;

    fn from_str(text: &str) -> Result<HttpConnections, String> {
        match text {
            "per-agent" | "per_agent" => Ok(HttpConnections::PerAgent),
            "shared" => Ok(HttpConnections::Shared),
            _ => Err(format!("unknown value `{text}`; use per-agent or shared")),
        }
    }
}

impl std::fmt::Display for HttpConnections {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            HttpConnections::PerAgent => "per-agent",
            HttpConnections::Shared => "shared",
        })
    }
}

/// What a swarm needs from the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Needs {
    /// Connections that stay open for the whole run: one for each
    /// WebSocket agent, and one for each HTTP agent with its own client.
    pub held_connections: u64,
    /// Open files that the process needs for them. A server in the same
    /// process holds the other end of each connection, so it needs two.
    pub open_files: u64,
}

impl Needs {
    /// Computes the needs of a swarm.
    #[must_use]
    pub fn of(
        ws_agents: u64,
        http_agents: u64,
        http: HttpConnections,
        server_in_process: bool,
        slack: u64,
    ) -> Needs {
        let held_connections = ws_agents
            + match http {
                HttpConnections::PerAgent => http_agents,
                HttpConnections::Shared => 0,
            };
        let per_connection = if server_in_process { 2 } else { 1 };
        Needs { held_connections, open_files: held_connections * per_connection + slack }
    }

    /// Checks the needs against the limits of the system.
    ///
    /// # Errors
    ///
    /// Returns the message for the operator when the run cannot work.
    pub fn check(&self, limits: &FileLimits, server_in_process: bool) -> Result<(), String> {
        if let Some(usable) = limits.usable().filter(|&usable| self.open_files > usable) {
            let both_ends = if server_in_process {
                " The server runs in this process and holds the other end of each connection."
            } else {
                ""
            };
            return Err(format!(
                "This run needs about {} open files for {} connections.{both_ends} This process \
                 can open {} (ulimit -n is {}, the kernel allows {} for one process). Raise the \
                 limit with `ulimit -n {}` in this shell, or use fewer agents, or use \
                 `--http-connections shared`.",
                thousands(self.open_files),
                thousands(self.held_connections),
                thousands(usable),
                limits.soft.map_or_else(|| "unknown".to_string(), |limit| limit.to_string()),
                limits.per_process_cap.map_or_else(|| "any number".to_string(), thousands),
                self.open_files,
            ));
        }
        if let Some(ports) = limits.ephemeral_ports
            && self.held_connections + PORT_MARGIN > ports
        {
            return Err(format!(
                "This run needs {} connections from this machine to one server address. The \
                 system has {} local ports for outgoing connections, and some are in use. Use \
                 at most {} agents with a connection of their own, or use `--http-connections \
                 shared` with a smaller `--ws-share`, or start the agents from several machines.",
                thousands(self.held_connections),
                thousands(ports),
                thousands(ports.saturating_sub(PORT_MARGIN)),
            ));
        }
        Ok(())
    }
}

/// What the process held when a run reached a limit of the machine.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Held {
    /// WebSocket connections that were open.
    pub ws_open: u64,
    /// HTTP agents that held a connection of their own.
    pub http_agents: u64,
    /// Open files of the process, when the system tells them.
    pub open_files: Option<u64>,
}

/// Says why a run stopped at a limit of this machine and what to change.
/// `error` is the failure that showed the limit
/// ([`RequestError::machine_limit`]).
#[must_use]
pub fn limit_message(error: &RequestError, held: Held, limits: &FileLimits) -> String {
    let count = |n: Option<u64>| n.map_or_else(|| "an unknown number of".to_string(), thousands);
    if error.code == NO_LOCAL_PORT {
        format!(
            "The run stopped at a limit of this machine: the system has no free local port for \
             one more connection ({}). {} WebSockets and {} HTTP connections of agents were \
             open. The system has {} local ports, and a port stays in use for some time after \
             its connection closes. Wait one minute after a large run and start again, or use \
             fewer agents, or use `--http-connections shared`, or start the agents from more \
             than one machine.",
            error.message,
            thousands(held.ws_open),
            thousands(held.http_agents),
            count(limits.ephemeral_ports),
        )
    } else {
        format!(
            "The run stopped at a limit of this machine: the process cannot open one more file \
             ({}). {} files were open, with {} WebSockets and {} HTTP connections of agents. \
             The limit is {} (ulimit -n), and the kernel allows {} for one process. Raise the \
             limit with `ulimit -n` in this shell, or use fewer agents.",
            error.message,
            count(held.open_files),
            thousands(held.ws_open),
            thousands(held.http_agents),
            limits.soft.map_or_else(|| "unknown".to_string(), |limit| limit.to_string()),
            limits.per_process_cap.map_or_else(|| "any number".to_string(), thousands),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine::Limit;
    use crate::swarm::error::TOO_MANY_OPEN_FILES;

    #[test]
    fn a_limit_that_a_run_reaches_is_named_with_what_was_open_and_the_way_out() {
        let held = Held { ws_open: 7_000, http_agents: 8_500, open_files: Some(31_013) };
        let ports = limit_message(
            &RequestError::local(NO_LOCAL_PORT, "Can't assign requested address (os error 49)"),
            held,
            &limits(1_000_000, 16_384),
        );
        assert!(ports.starts_with("The run stopped at a limit of this machine"), "{ports}");
        assert!(ports.contains("no free local port"), "{ports}");
        assert!(ports.contains("(Can't assign requested address (os error 49))"), "{ports}");
        assert!(ports.contains("7,000 WebSockets and 8,500 HTTP connections"), "{ports}");
        assert!(ports.contains("16,384 local ports"), "{ports}");
        assert!(ports.contains("--http-connections shared"), "{ports}");

        let files = limit_message(
            &RequestError::local(TOO_MANY_OPEN_FILES, "Too many open files (os error 24)"),
            held,
            &limits(256, 16_384),
        );
        assert!(files.contains("cannot open one more file"), "{files}");
        assert!(files.contains("31,013 files were open"), "{files}");
        assert!(files.contains("The limit is 256 (ulimit -n)"), "{files}");
        assert!(files.contains("Raise the limit with `ulimit -n`"), "{files}");
    }

    fn limits(soft: u64, ports: u64) -> FileLimits {
        FileLimits {
            soft: Some(Limit::At(soft)),
            hard: Some(Limit::Unlimited),
            per_process_cap: None,
            ephemeral_ports: Some(ports),
        }
    }

    #[test]
    fn a_server_in_the_same_process_doubles_the_open_files() {
        let alone = Needs::of(300, 700, HttpConnections::PerAgent, false, 100);
        assert_eq!(alone, Needs { held_connections: 1_000, open_files: 1_100 });
        let both = Needs::of(300, 700, HttpConnections::PerAgent, true, 100);
        assert_eq!(both, Needs { held_connections: 1_000, open_files: 2_100 });
        let shared = Needs::of(300, 700, HttpConnections::Shared, true, 100);
        assert_eq!(shared, Needs { held_connections: 300, open_files: 700 });
    }

    #[test]
    fn a_run_that_does_not_fit_is_refused_with_the_way_out() {
        let needs = Needs::of(0, 1_000, HttpConnections::PerAgent, true, 100);
        assert_eq!(needs.check(&limits(2_100, 30_000), true), Ok(()));
        let files = needs.check(&limits(2_099, 30_000), true).expect_err("one file too few");
        assert!(files.contains("ulimit -n 2100"), "{files}");
        assert!(files.contains("2,099"), "{files}");
        let ports = needs.check(&limits(1_000_000, 1_400), true).expect_err("too few ports");
        assert!(ports.contains("1,400 local ports"), "{ports}");
        assert!(ports.contains("at most 888 agents"), "{ports}");
        // Limits that the system does not tell are not a reason to stop.
        assert_eq!(needs.check(&FileLimits::default(), true), Ok(()));
    }
}
