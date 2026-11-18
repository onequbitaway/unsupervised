//! `swarm` through the library, with a few agents.
//!
//! These tests run in real time for a few seconds: a swarm measures a
//! server that steps by the clock.

mod support;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use support::DocumentedServer;
use unsup_loadtest::local::Shape;
use unsup_loadtest::profile::Profile;
use unsup_loadtest::swarm::{
    self, CredentialFile, Flow, HttpConnections, SpawnServer, SwarmConfig, Target, Timing,
    WaitRoute, uses_ws,
};
use unsup_loadtest::think::{ThinkKind, ThinkTime};

/// A short run: 8 agents, half of them on the WebSocket, about one action
/// every 50 milliseconds for each agent.
fn config(target: Target) -> SwarmConfig {
    SwarmConfig {
        target,
        agents: 8,
        ramp_up: Duration::from_millis(400),
        duration: Duration::from_millis(2_500),
        think: ThinkTime::new(ThinkKind::Exponential, 0.05, 0.0).expect("a valid think time"),
        ws_share: 0.5,
        http_connections: HttpConnections::PerAgent,
        profile: Profile::load("walkers").expect("a built-in profile"),
        seed: 3,
        credentials: None,
        timing: Timing {
            request_timeout: Duration::from_secs(10),
            retry: Duration::from_millis(50),
            max_failures: 3,
            wait_timeout: Duration::from_secs(2),
            poll_interval: Duration::from_millis(100),
        },
        // The test machine can be slow and busy. The verdict of these
        // tests is about errors, not about speed.
        p99_target: Duration::from_secs(8),
        max_error_share: 0.0,
        stats_path: "/v1/stats".into(),
    }
}

fn temp_file(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("unsup-swarm-{}-{name}", std::process::id()))
}

#[test]
fn websocket_agents_are_spread_evenly() {
    let ws: Vec<u32> = (0..10).filter(|&i| uses_ws(i, 0.3)).collect();
    assert_eq!(ws, [3, 6, 9], "3 of 10 agents, spread over the start order");
    assert_eq!((0..8).filter(|&i| uses_ws(i, 0.5)).count(), 4);
    assert_eq!((0..7).filter(|&i| uses_ws(i, 1.0)).count(), 7);
    assert_eq!((0..7).filter(|&i| uses_ws(i, 0.0)).count(), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_swarm_runs_against_the_server_of_the_project() {
    let spawn = SpawnServer { seed: 1, shape: Shape { cities: 1, districts: 1 }, tick_ms: 20 };
    let run = swarm::run(&config(Target::Spawn(spawn))).await.expect("the swarm runs");

    assert_eq!(run.spawned, Some(spawn));
    assert_eq!((run.agents, run.agents_started, run.agents_ready), (8, 8, 8));
    assert!(run.agents_failed.is_empty(), "agents gave up: {:?}", run.agents_failed);
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert!(run.errors_ramp.is_empty(), "errors in the ramp-up: {:?}", run.errors_ramp);
    assert!(run.sustained, "not sustained: {:?}", run.not_sustained_because);

    // About 8 agents * 2.5 s / 50 ms = 400 actions when nothing waits.
    // Agents that wait for the end of a walk send fewer.
    assert!(run.acks > 40, "{} acknowledgements", run.acks);
    assert_eq!(run.ack.count, run.acks, "every acknowledgement has a latency");
    assert_eq!(run.ack_http.count + run.ack_ws.count, run.acks);
    assert_eq!(run.ack_states.values().sum::<u64>(), run.acks);
    assert!(run.ack_states.get("started").copied().unwrap_or(0) > 0, "{:?}", run.ack_states);
    assert!(run.ack.p50_ms > 0.0 && run.ack.p50_ms <= run.ack.p99_ms);
    assert!(run.ack.p99_ms <= run.ack.max_ms);
    assert!(run.acks_per_second > 16.0, "{} for each second", run.acks_per_second);
    assert!(run.observations > 0);

    // Every agent signed up. The server of today has the development
    // route; a later server has the documented flow. Both are right.
    assert!(run.signup_flow.is_some());
    assert_eq!(run.signup.count, 8, "one sign-up for each agent");
    assert_eq!(run.credentials_reused, 0);

    // Four agents planned a WebSocket. Each has one, or uses HTTP because
    // the server has no WebSocket route yet.
    let c = &run.connections;
    assert_eq!(c.ws_agents_planned, 4);
    assert_eq!(c.ws_open_peak + c.ws_fallbacks, 4);
    assert_eq!(c.ws_open_peak + c.http_agents_peak, 8, "each agent holds one connection");
    assert_eq!(c.needs.expect("the needs are computed").held_connections, 8);
    if let (Some(before), Some(peak)) = (c.open_files_before, c.open_files_peak) {
        // The server is in this process: both ends of 8 connections.
        assert!(peak >= before + 16, "{before} open files before, {peak} at the peak");
    }

    // The statistics of the simulation for the window.
    let sim = run.server_stats.sim.expect("a server in this process gives statistics");
    assert_eq!(sim.residents, 8);
    // 2.5 s at 20 ms for each frame are 125 frames.
    assert!((60..=140).contains(&sim.frames), "{} frames in the window", sim.frames);
    assert!(
        sim.inputs_applied >= run.acks + sim.frames,
        "{} inputs for {} acknowledgements and {} frames",
        sim.inputs_applied,
        run.acks,
        sim.frames
    );
    assert!(run.summary().contains("result: SUSTAINED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_swarm_uses_the_documented_sign_up_the_websocket_and_the_long_poll() {
    let server = DocumentedServer::start(Duration::from_millis(20)).await;
    let keys = temp_file("keys.json");
    let _ = std::fs::remove_file(&keys);
    let mut config = config(Target::Url(server.url()));
    config.credentials = Some(keys.clone());
    let run = swarm::run(&config).await.expect("the swarm runs");

    assert_eq!(run.agents_ready, 8);
    assert!(run.agents_failed.is_empty(), "agents gave up: {:?}", run.agents_failed);
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert!(run.errors_ramp.is_empty(), "errors in the ramp-up: {:?}", run.errors_ramp);
    assert!(run.sustained, "not sustained: {:?}", run.not_sustained_because);

    // The documented flow: one owner and one resident for each agent.
    assert_eq!(run.signup_flow, Some(Flow::Documented));
    assert_eq!(server.counters.owners.load(Ordering::Relaxed), 8);
    assert_eq!(server.counters.residents.load(Ordering::Relaxed), 8);

    // Four real WebSockets, and acknowledgements on both kinds of link.
    let c = &run.connections;
    assert_eq!((c.ws_open_peak, c.ws_opened, c.ws_fallbacks), (4, 4, 0));
    assert_eq!(c.http_agents_peak, 4);
    assert_eq!(server.counters.sockets.load(Ordering::Relaxed), 4);
    assert!(run.ack_ws.count > 20, "{} acknowledgements on WebSockets", run.ack_ws.count);
    assert!(run.ack_http.count > 20, "{} acknowledgements over HTTP", run.ack_http.count);
    assert_eq!(run.ack_ws.count + run.ack_http.count, run.acks);
    // The server pushed reports of ended walks, as MessagePack frames.
    assert!(run.pushed_frames > 0);

    // HTTP agents waited in the long poll.
    assert_eq!(run.wait_route, WaitRoute::LongPoll);
    assert!(server.counters.long_polls.load(Ordering::Relaxed) > 0);
    assert!(run.waits > 0);

    // The statistics route answered at both ends of the window.
    let frames = |stats: &Option<serde_json::Value>| {
        stats.as_ref().expect("the route answers")["frames_run"].as_u64().expect("a number")
    };
    let (first, last) =
        (frames(&run.server_stats.route_start), frames(&run.server_stats.route_end));
    assert!(last > first + 60, "frames {first} at the start and {last} at the end of the window");
    assert_eq!(run.server_stats.sim, None, "the server is not in the control of the tool");

    // The keys are in the file, and a second run uses them: nobody signs
    // up again.
    let (stored, warning) = CredentialFile::load(&keys, &server.url()).expect("the file is read");
    assert_eq!(warning, None);
    assert_eq!(stored.len(), 8);
    assert!(stored.iter().all(|c| c.key == format!("key-{}", c.resident)), "{stored:?}");
    let mut again = config.clone();
    again.duration = Duration::from_millis(800);
    let second = swarm::run(&again).await.expect("the swarm runs");
    assert_eq!(second.credentials_reused, 8);
    assert_eq!(second.signup.count, 0, "no agent signed up");
    assert_eq!(second.signup_flow, None);
    assert_eq!(second.agents_ready, 8);
    assert_eq!(second.failed_requests, 0, "errors: {:?}", second.errors);
    assert_eq!(server.counters.residents.load(Ordering::Relaxed), 8, "no new resident");
    assert!(second.acks > 0);

    // Keys of residents that the server does not know: the agents sign up
    // again and go on.
    let stale = CredentialFile {
        server: server.url(),
        agents: (0..8)
            .map(|n| swarm::Credential {
                resident: 900_000 + n,
                key: format!("key-{}", 900_000 + n),
            })
            .collect(),
    };
    stale.save(&keys).expect("the file is written");
    let third = swarm::run(&again).await.expect("the swarm runs");
    assert_eq!(third.credentials_reused, 8);
    assert_eq!(third.credentials_renewed, 8, "every agent got a new key");
    assert_eq!(third.agents_ready, 8);
    assert!(third.agents_failed.is_empty(), "agents gave up: {:?}", third.agents_failed);
    assert_eq!(server.counters.residents.load(Ordering::Relaxed), 16);
    // The refused keys are counted, each agent once, in the ramp-up.
    let refused: u64 = third.errors_ramp.values().sum::<u64>() + third.failed_requests;
    assert_eq!(refused, 8, "ramp-up: {:?}, window: {:?}", third.errors_ramp, third.errors);
    std::fs::remove_file(&keys).expect("the file is removed");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_actions_is_not_sustained_and_the_errors_are_counted_by_code() {
    let server = DocumentedServer::start(Duration::from_millis(20)).await;
    server.counters.refuse_acts.store(true, Ordering::Relaxed);
    let mut config = config(Target::Url(server.url()));
    config.ws_share = 0.0;
    config.ramp_up = Duration::ZERO;
    config.duration = Duration::from_millis(1_500);
    let run = swarm::run(&config).await.expect("the swarm runs to its end");

    assert_eq!(run.agents_ready, 8, "sign-up and the first observation work");
    assert_eq!(run.acks, 0);
    // Each agent gives up after 3 failed requests in a row.
    assert_eq!(run.agents_failed.get("overloaded"), Some(&8), "{:?}", run.agents_failed);
    let overloaded = run.errors.get("overloaded").copied().unwrap_or(0)
        + run.errors_ramp.get("overloaded").copied().unwrap_or(0);
    assert_eq!(overloaded, 24, "window: {:?}, ramp-up: {:?}", run.errors, run.errors_ramp);
    assert!(!run.sustained);
    let reasons = run.not_sustained_because.join("; ");
    assert!(reasons.contains("8 agents gave up"), "{reasons}");
    assert!(reasons.contains("no action was acknowledged"), "{reasons}");
    assert!(run.summary().contains("result: NOT SUSTAINED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_pool_of_connections_serves_all_http_agents() {
    let server = DocumentedServer::start(Duration::from_millis(20)).await;
    let mut config = config(Target::Url(server.url()));
    config.http_connections = HttpConnections::Shared;
    config.ws_share = 0.25;
    config.duration = Duration::from_millis(1_200);
    let run = swarm::run(&config).await.expect("the swarm runs");
    assert_eq!(run.agents_ready, 8);
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert!(run.acks > 0);
    let c = &run.connections;
    assert_eq!(c.ws_open_peak, 2);
    assert_eq!(c.http_agents_peak, 0, "no agent holds a client of its own");
    // Only the two WebSockets are held for the whole run.
    assert_eq!(c.needs.expect("the needs are computed").held_connections, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_swarm_against_no_server_stops_with_a_clear_message() {
    // A port that was free a moment ago: nothing listens there.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        listener.local_addr().expect("an address").port()
    };
    let url = format!("http://127.0.0.1:{port}");
    let error = swarm::run(&config(Target::Url(url.clone()))).await.expect_err("no server");
    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("cannot read the catalogue of the server at {url}")),
        "{message}"
    );

    let tls = swarm::run(&config(Target::Url("https://example.org".into())))
        .await
        .expect_err("this build has no TLS");
    assert!(format!("{tls:#}").contains("needs TLS"), "{tls:#}");
}
