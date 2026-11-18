//! `swarm`: many virtual agents over the network against a server.
//!
//! Each agent is one task. It signs up as an owner with one resident (or
//! uses a stored key), then it observes, thinks and acts until the run
//! ends, like an agent that a language model drives. A share of the agents
//! uses the WebSocket; the others send HTTP requests and wait in the long
//! poll. The run measures the acknowledgement latency of every action, the
//! throughput and the errors.
//!
//! The run has two parts. In the **ramp-up** the agents start one after
//! the other. The **window** comes after it; all latencies and rates in
//! the result are those of the window.
//!
//! A request has one of four ends. The server acknowledges it (the game
//! can accept or reject the action: both are valid answers). A limit of
//! the server refuses it with status 429: a class of its own, because the
//! server then works as its configuration says. The request fails: no
//! usable answer. Or this machine cannot open one more connection, which
//! stops the run.

mod agent;
mod error;
mod http;
mod metrics;
mod preflight;
mod result;
mod server;
mod signup;
mod wire;
mod world;
mod ws;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, anyhow};
use serde_json::Value;
use unsup_protocol::Catalogue;

pub use agent::Timing;
pub use error::RequestError;
pub use http::Urls;
pub use metrics::Link;
pub use preflight::{Held, HttpConnections, Needs, limit_message};
pub use result::{Connections, SwarmRun};
pub use server::{ServerStats, SimWindow, SpawnServer};
pub use signup::{Credential, CredentialFile};

use crate::bench::ProfileUse;
use crate::defaults::defaults;
use crate::machine::FileLimits;
use crate::process;
use crate::profile::Profile;
use crate::report::thousands;
use crate::stats::Counts;
use crate::think::ThinkTime;
use agent::{Seat, Shared};
use metrics::Metrics;

/// Time between two samples of the run: open files, acknowledgements.
const SAMPLE: Duration = Duration::from_secs(1);

/// Samples between two progress lines.
const PROGRESS_EVERY: u64 = 5;

/// The server that a swarm talks to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A server that already runs, by its address, for example
    /// `http://127.0.0.1:8080`.
    Url(String),
    /// A server that the tool starts in this process on a free port.
    Spawn(SpawnServer),
}

/// What to run.
#[derive(Clone, Debug)]
pub struct SwarmConfig {
    /// The server.
    pub target: Target,
    /// Number of agents.
    pub agents: u32,
    /// Time over which the agents start, one after the other.
    pub ramp_up: Duration,
    /// Length of the measurement window after the ramp-up.
    pub duration: Duration,
    /// Think time of an agent between two requests.
    pub think: ThinkTime,
    /// Share of the agents that use the WebSocket, from 0 to 1.
    pub ws_share: f64,
    /// How the HTTP agents hold connections.
    pub http_connections: HttpConnections,
    /// What the agents do.
    pub profile: Profile,
    /// Seed of the choices of the agents.
    pub seed: u64,
    /// File with the keys of an earlier run. Keys in it are used again;
    /// the keys of this run are written to it.
    pub credentials: Option<PathBuf>,
    /// Times and limits of requests.
    pub timing: Timing,
    /// Target for the 99th percentile of the acknowledgement latency.
    pub p99_target: Duration,
    /// Greatest share of failed requests of a sustained run.
    pub max_error_share: f64,
    /// Path of the route with the statistics of the server.
    pub stats_path: String,
}

/// Returns true when agent `index` uses the WebSocket. The WebSocket
/// agents are spread evenly over the start order, and their number is
/// `agents * share`, rounded down.
#[must_use]
pub fn uses_ws(index: u32, share: f64) -> bool {
    let share = share.clamp(0.0, 1.0);
    let upto = |n: u32| (f64::from(n) * share).floor();
    upto(index + 1) > upto(index)
}

/// Runs a swarm.
///
/// It prints its progress to the standard error stream. A server that
/// fails under the load is not an `Err`: the result says that the load was
/// not sustained, and why.
///
/// # Errors
///
/// Fails when the run cannot start: the machine does not allow that many
/// connections, the server cannot be reached, or the server has none of
/// the verbs of the profile.
pub async fn run(config: &SwarmConfig) -> anyhow::Result<SwarmRun> {
    let d = &defaults().swarm;
    let agents = u64::from(config.agents);
    let ws_planned = (0..config.agents).filter(|&i| uses_ws(i, config.ws_share)).count() as u64;
    let in_process = matches!(config.target, Target::Spawn(_));
    let limits = FileLimits::detect();
    let needs =
        Needs::of(ws_planned, agents - ws_planned, config.http_connections, in_process, d.fd_slack);
    needs.check(&limits, in_process).map_err(|message| anyhow!(message))?;

    let (server, spawned) = match &config.target {
        Target::Spawn(spawn) => (Some(server::spawn(spawn).await?), Some(*spawn)),
        Target::Url(_) => (None, None),
    };
    let base = match (&server, &config.target) {
        (Some(server), _) => format!("http://{}", server.addr),
        (None, Target::Url(url)) => url.clone(),
        (None, Target::Spawn(_)) => unreachable_spawn()?,
    };
    let urls = Arc::new(Urls::new(&base, &config.stats_path).map_err(|message| anyhow!(message))?);
    let mut warnings = Vec::new();

    // What the server offers: verbs for the profile, and the map for walk
    // targets and sites.
    let control = http::client(config.timing.request_timeout, false)?;
    let (catalogue, _): (Catalogue, _) = http::get_json(&control, &urls.catalogue, None)
        .await
        .with_context(|| format!("cannot read the catalogue of the server at {}", urls.base))?;
    let mut profile = config.profile.resolve(&catalogue)?;
    let map = world::load_map(&control, &urls, config.timing.retry, config.timing.max_failures)
        .await
        .with_context(|| format!("cannot read the map of the server at {}", urls.base))?;
    profile.check_map(&map);
    for warning in &profile.warnings {
        eprintln!("swarm: warning: {warning}");
    }
    let profile_use = ProfileUse::of(&profile);

    let mut stored = Vec::new();
    if let Some(path) = &config.credentials {
        if in_process {
            warnings.push(
                "a server that the tool starts has a new world; stored keys are not used".into(),
            );
        } else {
            let (keys, warning) = CredentialFile::load(path, &urls.base)?;
            stored = keys;
            warnings.extend(warning);
        }
    }
    let credentials_reused = (stored.len() as u64).min(agents);

    let max_latency = Duration::from_secs(d.histogram_max_secs.max(1));
    let metrics = Metrics::new(max_latency, d.histogram_sigfig).map_err(|m| anyhow!(m))?;
    // One pool of connections for the agents that hold no HTTP connection
    // of their own: all agents with `HttpConnections::Shared`, and each
    // WebSocket agent for its sign-up.
    let pool = http::client(config.timing.request_timeout, false)?;
    let open_files_before = process::open_files();
    let start = Instant::now();
    let steady_at = start + config.ramp_up;
    let end = steady_at + config.duration;
    let shared = Arc::new(Shared {
        profile,
        map: Arc::new(map),
        think: config.think,
        urls: Arc::clone(&urls),
        metrics,
        timing: config.timing,
        seed: config.seed,
        end,
        pool,
        http_connections: config.http_connections,
    });

    eprintln!(
        "swarm: {} agents against {} ({} on the WebSocket), ramp-up {:.0} s, window {:.0} s",
        thousands(agents),
        urls.base,
        thousands(ws_planned),
        config.ramp_up.as_secs_f64(),
        config.duration.as_secs_f64()
    );
    let mut tasks = Vec::with_capacity(config.agents as usize);
    for index in 0..config.agents {
        let offset = config.ramp_up.mul_f64(f64::from(index) / f64::from(config.agents.max(1)));
        let seat = Seat {
            index,
            start: start + offset,
            link: if uses_ws(index, config.ws_share) { Link::Ws } else { Link::Http },
            credential: stored.get(index as usize).cloned(),
        };
        tasks.push(tokio::spawn(agent::run(seat, Arc::clone(&shared))));
    }

    // Watch the run: once each second, sample the open files and the
    // acknowledgements, and open and close the measurement window.
    let m = &shared.metrics;
    let sim = server.as_ref().map(|server| server.sim.clone());
    let mut open_files_peak = open_files_before;
    let mut per_second: Vec<u64> = Vec::new();
    let mut window: Option<(Instant, u64)> = None;
    let mut stats = ServerStats::default();
    let mut sim_start = None;
    let mut samples = 0u64;
    let mut machine_limit: Option<String> = None;
    let window_secs = loop {
        let now = Instant::now();
        if window.is_none() && now >= steady_at {
            m.set_steady(true);
            window = Some((now, 0));
            if let Some(sim) = &sim {
                sim_start = server::sim_stats(sim).await;
            }
            stats.route_start = http::get_json::<Value>(&control, &urls.stats, None)
                .await
                .ok()
                .map(|(value, _)| value);
        }
        let open_files = process::open_files();
        if let Some(files) = open_files {
            open_files_peak = Some(open_files_peak.map_or(files, |peak| peak.max(files)));
        }
        if let Some(error) = m.machine_limit.get() {
            // The machine cannot hold more connections. What the run would
            // measure from here on is the machine, not the server.
            m.set_steady(false);
            let held =
                Held { ws_open: m.ws_open.now(), http_agents: m.http_agents.now(), open_files };
            machine_limit = Some(limit_message(error, held, &limits));
            break window.map_or(0.0, |(opened, _)| opened.elapsed().as_secs_f64());
        }
        if let Some((opened, last)) = &mut window {
            if now >= end {
                m.set_steady(false);
                break opened.elapsed().as_secs_f64();
            }
            let acks = m.acks_steady.get();
            if now.duration_since(*opened) >= SAMPLE {
                per_second.push(acks - *last);
            }
            *last = acks;
        }
        samples += 1;
        if samples.is_multiple_of(PROGRESS_EVERY) {
            eprintln!(
                "swarm: {:>4.0} s: {} of {} agents ready, {} acknowledgements, {} failed requests, {} refused by a limit of the server, {} WebSockets open{}",
                start.elapsed().as_secs_f64(),
                thousands(m.agents_ready.get()),
                thousands(agents),
                thousands(m.acks_total.get()),
                thousands(m.failed_steady.get() + m.errors_ramp.get().values().sum::<u64>()),
                thousands(m.throttled.get() + m.throttled_ramp.get().values().sum::<u64>()),
                thousands(m.ws_open.now()),
                if window.is_some() { "" } else { " (ramp-up)" }
            );
        }
        let next = (Instant::now() + SAMPLE).min(if window.is_some() { end } else { steady_at });
        tokio::time::sleep_until(next.into()).await;
    };
    if let (Some(sim), Some(start)) = (&sim, &sim_start)
        && let Some(end) = server::sim_stats(sim).await
    {
        stats.sim = Some(SimWindow::between(start, &end));
    }
    stats.route_end =
        http::get_json::<Value>(&control, &urls.stats, None).await.ok().map(|(value, _)| value);

    // The agents see the end by themselves. A request that is under way
    // gets the time of one timeout to finish. After a machine limit the
    // run does not wait: the agents stop where they are.
    if let Some(message) = &machine_limit {
        eprintln!("swarm: {message}");
        for task in &tasks {
            task.abort();
        }
    }
    let grace = Instant::now() + config.timing.request_timeout + SAMPLE;
    let mut keys = Vec::new();
    for task in tasks {
        let abort = task.abort_handle();
        match tokio::time::timeout_at(grace.into(), task).await {
            Ok(Ok(outcome)) => keys.extend(outcome.credential),
            Ok(Err(error)) if error.is_cancelled() => {}
            Ok(Err(_)) => m.agents_failed.add("agent_task_failed"),
            Err(_) => {
                abort.abort();
                m.agents_failed.add("did_not_stop");
            }
        }
    }
    if let Some(server) = server {
        // The run is over. A server that does not stop cleanly does not
        // change what was measured, but the operator must know.
        if let Err(error) = server.stop().await {
            warnings.push(format!("the server in this process did not stop cleanly: {error:#}"));
        }
    }
    // A run that stopped at a machine limit knows only a part of the keys.
    // It leaves the file as it is.
    if let Some(path) =
        config.credentials.as_ref().filter(|_| !in_process && machine_limit.is_none())
    {
        CredentialFile { server: urls.base.clone(), agents: keys }.save(path)?;
    }

    let acks = m.acks_steady.get();
    let observations = m.observes_steady.get();
    let failed = m.failed_steady.get();
    let requests = acks + observations + failed;
    let actions = m.actions();
    let ack_states: Counts = [
        ("done", actions.done),
        ("started", actions.started),
        ("pending", actions.pending),
        ("queued", actions.queued),
        ("rejected", actions.rejected),
    ]
    .into_iter()
    .filter(|(_, count)| *count > 0)
    .map(|(state, count)| (state.to_string(), count))
    .collect();
    let mut run = SwarmRun {
        server: urls.base.clone(),
        spawned,
        agents,
        ramp_up_secs: config.ramp_up.as_secs_f64(),
        duration_secs: window_secs,
        think: Some(config.think),
        http_connections: config.http_connections,
        profile: profile_use,
        credentials_reused,
        credentials_renewed: m.credentials_renewed.get(),
        agents_started: m.agents_started.get(),
        agents_ready: m.agents_ready.get(),
        agents_failed: m.agents_failed.get(),
        connections: Connections {
            limits,
            needs: Some(needs),
            ws_agents_planned: ws_planned,
            ws_open_peak: m.ws_open.peak(),
            ws_opened: m.ws_open.total(),
            http_agents_peak: m.http_agents.peak(),
            open_files_before,
            open_files_peak,
        },
        ack: m.ack.summary(),
        ack_http: m.ack_http.summary(),
        ack_ws: m.ack_ws.summary(),
        observe: m.observe.summary(),
        signup: m.signup.summary(),
        acks,
        acks_per_second: if window_secs > 0.0 { acks as f64 / window_secs } else { 0.0 },
        acks_per_second_min: per_second.iter().copied().min().unwrap_or(0),
        acks_per_second_max: per_second.iter().copied().max().unwrap_or(0),
        ack_states,
        rejections: actions.rejected_by_code.clone(),
        actions,
        failed_requests: failed,
        error_share: if requests > 0 { failed as f64 / requests as f64 } else { 0.0 },
        throttled_requests: m.throttled.get(),
        throttled: m.throttled_steady.get(),
        throttled_ramp: m.throttled_ramp.get(),
        errors: m.errors_steady.get(),
        errors_ramp: m.errors_ramp.get(),
        error_examples: m.error_examples.get(),
        machine_limit,
        observations,
        waits: m.waits_steady.get(),
        long_polls: m.long_polls.get(),
        pushed_frames: m.pushed.get(),
        server_stats: stats,
        p99_target_ms: config.p99_target.as_secs_f64() * 1e3,
        max_error_share: config.max_error_share,
        warnings,
        ..SwarmRun::default()
    };
    run.judge();
    Ok(run)
}

/// A spawn target always gives a server, or `run` has already returned its
/// error. This keeps the match total without a panic.
fn unreachable_spawn() -> anyhow::Result<String> {
    Err(anyhow!("the server in this process did not start"))
}
