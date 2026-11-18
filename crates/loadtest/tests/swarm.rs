//! `swarm` through the library, with a few agents.
//!
//! These tests run in real time for a few seconds: a swarm measures a
//! server that steps by the clock. Every server here is the gateway of the
//! project: the sign-up with owners and keys, the WebSocket and the long
//! poll are the real ones.

mod support;

use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Duration;

use support::{TestServer, no_limits};
use unsup_loadtest::local::Shape;
use unsup_loadtest::profile::Profile;
use unsup_loadtest::swarm::{
    self, CredentialFile, HttpConnections, SpawnServer, SwarmConfig, Target, Timing, uses_ws,
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
        },
        // The test machine can be slow and busy. The verdict of these
        // tests is about errors, not about speed.
        p99_target: Duration::from_secs(8),
        max_error_share: 0.0,
        stats_path: "/v1/spectate/stats".into(),
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
    // The tool turned the limits of its own server off through the
    // configuration: no request was refused by a limit.
    assert_eq!(run.throttled_requests, 0, "limits: {:?}", run.throttled);
    assert!(run.throttled_ramp.is_empty(), "limits in the ramp-up: {:?}", run.throttled_ramp);
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
    // Every walk target is a walkable tile from the map of the server: the
    // game rejects no walk because of its target. An agent that does not
    // wait for the end of its walk can fill the queue of its resident.
    for code in ["not_allowed", "not_found", "invalid_args", "unknown_verb"] {
        assert_eq!(run.rejections.get(code), None, "rejections: {:?}", run.rejections);
    }
    assert_eq!(run.actions.sent, run.acks);
    assert_eq!(run.actions.by_verb["move_to"].failed, 0, "{:?}", run.actions.by_verb["move_to"]);
    assert!(run.actions.reports_finished > 0, "no agent read the report of an ended walk");

    // Every agent signed up with the flow of the protocol.
    assert_eq!(run.signup.count, 8, "one sign-up for each agent");
    assert_eq!(run.credentials_reused, 0);

    // Four agents hold a WebSocket and four an HTTP connection.
    let c = &run.connections;
    assert_eq!((c.ws_agents_planned, c.ws_open_peak, c.ws_opened), (4, 4, 4));
    assert_eq!(c.http_agents_peak, 4);
    assert_eq!(c.needs.expect("the needs are computed").held_connections, 8);
    if let (Some(before), Some(peak)) = (c.open_files_before, c.open_files_peak) {
        // The server is in this process: both ends of 8 connections.
        assert!(peak >= before + 16, "{before} open files before, {peak} at the peak");
    }
    assert!(run.ack_ws.count > 10, "{} acknowledgements on WebSockets", run.ack_ws.count);
    assert!(run.ack_http.count > 10, "{} acknowledgements over HTTP", run.ack_http.count);
    // The server pushed reports of ended walks on the WebSockets, and the
    // HTTP agents waited in the long poll.
    assert!(run.pushed_frames > 0);
    assert!(run.waits > 0);
    assert!(run.long_polls > 0, "no long poll was answered");

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
    let summary = run.summary();
    assert!(summary.contains("result: SUSTAINED"), "{summary}");
    assert!(
        summary.contains("sign-up: 8 with POST /v1/owners and POST /v1/residents"),
        "{summary}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_daily_life_profile_sends_plans_over_http_and_over_the_websocket() {
    let spawn = SpawnServer { seed: 2, shape: Shape { cities: 1, districts: 1 }, tick_ms: 10 };
    let mut config = config(Target::Spawn(spawn));
    config.profile = Profile::load("daily_life").expect("a built-in profile");
    config.duration = Duration::from_millis(4_000);
    let run = swarm::run(&config).await.expect("the swarm runs");

    assert_eq!(run.agents_ready, 8);
    assert!(run.agents_failed.is_empty(), "agents gave up: {:?}", run.agents_failed);
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert_eq!(run.throttled_requests, 0, "limits: {:?}", run.throttled);
    assert!(run.profile.skipped.is_empty() && run.profile.warnings.is_empty());

    // An action at a site travels as a plan: `move_to { site }`, then the
    // action, which waits in the queue behind the walk. Both links got
    // such acknowledgements.
    let actions = &run.actions;
    assert!(actions.queued > 0, "no action was queued behind a walk: {:?}", run.ack_states);
    assert_eq!(run.ack.count, run.acks, "each acknowledgement of a plan has a latency");
    assert_eq!(actions.sent, run.acks);
    assert!(run.ack_ws.count > 0 && run.ack_http.count > 0);
    // The server took every request: no argument name and no body was
    // wrong.
    for code in ["invalid_args", "unknown_verb"] {
        assert_eq!(run.rejections.get(code), None, "rejections: {:?}", run.rejections);
    }
    // Verbs of the full game were acknowledged, not only walks.
    let sent = |verb: &str| actions.by_verb.get(verb).map_or(0, |counts| counts.sent);
    assert!(sent("move_to") > 0);
    assert!(sent("work") + sent("gather") > 0, "{:?}", actions.by_verb.keys());
    assert!(sent("say") > 0, "{:?}", actions.by_verb.keys());
    // The rejections by code add up: a refusal of the game is data.
    assert_eq!(run.rejections.values().sum::<u64>(), actions.rejected);
    assert_eq!(run.ack_states.get("rejected").copied().unwrap_or(0), actions.rejected);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_swarm_keeps_its_keys_and_signs_up_again_when_a_server_does_not_know_them() {
    let server = TestServer::start(Duration::from_millis(20), no_limits).await;
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
    assert_eq!(run.signup.count, 8);
    let residents = |sim: &unsup_runtime::SimStats| sim.residents;
    assert_eq!(residents(&server.sim.stats().await.expect("the simulation runs")), 8);

    // The statistics route answered at both ends of the window.
    let frame = |stats: &Option<serde_json::Value>| {
        stats.as_ref().expect("the route answers")["frame"].as_u64().expect("a number")
    };
    let (first, last) = (frame(&run.server_stats.route_start), frame(&run.server_stats.route_end));
    assert!(last > first + 60, "frame {first} at the start and {last} at the end of the window");
    assert_eq!(run.server_stats.sim, None, "the server is not in the control of the tool");

    // The keys are in the file: bearer keys of residents. A second run
    // uses them: nobody signs up again.
    let (stored, warning) = CredentialFile::load(&keys, &server.url()).expect("the file is read");
    assert_eq!(warning, None);
    assert_eq!(stored.len(), 8);
    assert!(stored.iter().all(|c| c.key.starts_with("unsup_res_")), "the keys of residents");
    let mut again = config.clone();
    again.duration = Duration::from_millis(800);
    let second = swarm::run(&again).await.expect("the swarm runs");
    assert_eq!(second.credentials_reused, 8);
    assert_eq!(second.signup.count, 0, "no agent signed up");
    assert_eq!(second.agents_ready, 8);
    assert_eq!(second.failed_requests, 0, "errors: {:?}", second.errors);
    assert_eq!(residents(&server.sim.stats().await.expect("the simulation runs")), 8);
    assert!(second.acks > 0);

    // Keys that the server does not know: it answers 401, the agents sign
    // up again and go on.
    let stale = CredentialFile {
        server: server.url(),
        agents: (0..8u64)
            .map(|n| swarm::Credential {
                resident: 900_000 + n,
                key: format!("unsup_res_{:064x}", 900_000 + n),
            })
            .collect(),
    };
    stale.save(&keys).expect("the file is written");
    let third = swarm::run(&again).await.expect("the swarm runs");
    assert_eq!(third.credentials_reused, 8);
    assert_eq!(third.credentials_renewed, 8, "every agent got a new key");
    assert_eq!(third.agents_ready, 8);
    assert!(third.agents_failed.is_empty(), "agents gave up: {:?}", third.agents_failed);
    assert_eq!(residents(&server.sim.stats().await.expect("the simulation runs")), 16);
    // The refused keys are counted by the code of the server, each agent
    // once, in the ramp-up.
    assert_eq!(third.failed_requests, 0, "window: {:?}", third.errors);
    assert_eq!(third.errors_ramp.get("invalid_token"), Some(&8), "{:?}", third.errors_ramp);
    assert_eq!(third.errors_ramp.len(), 1, "{:?}", third.errors_ramp);
    std::fs::remove_file(&keys).expect("the file is removed");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_limits_of_a_foreign_server_are_a_class_of_their_own() {
    // A server with a small limit for each key: 3 requests at once, then
    // one more each second. An agent of this test wants about 20 for each
    // second. The sign-up limit is off, so every agent gets its resident.
    let server = TestServer::start(Duration::from_millis(20), |config| {
        config.limits.signups_per_hour = 0;
        config.limits.key_requests_per_minute = 60;
        config.limits.key_burst = 3;
    })
    .await;
    let mut config = config(Target::Url(server.url()));
    config.ramp_up = Duration::from_millis(200);
    config.duration = Duration::from_millis(3_000);
    let run = swarm::run(&config).await.expect("the swarm runs to its end");

    // The server refused requests with status 429 and the code
    // `rate_limited`, over HTTP and in `error` frames on the WebSockets.
    assert!(run.throttled_requests > 0, "the limit refused nothing");
    assert_eq!(run.throttled.get("rate_limited"), Some(&run.throttled_requests));
    assert_eq!(run.throttled.len(), 1, "{:?}", run.throttled);
    // They are not failed requests, and no agent gave up because of them:
    // an agent waits as long as the server asks.
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert!(run.error_share.abs() < f64::EPSILON);
    assert_eq!(run.errors.get("rate_limited"), None);
    assert!(run.errors_ramp.is_empty(), "errors in the ramp-up: {:?}", run.errors_ramp);
    assert!(run.agents_failed.is_empty(), "agents gave up: {:?}", run.agents_failed);
    assert_eq!(run.agents_ready, 8);
    // The requests under the limit were served.
    assert!(run.acks > 0);
    // 8 keys with 3 requests at once and one more each second cannot make
    // the 400 actions of the run without a limit.
    assert!(run.acks < 150, "{} acknowledgements under the limit", run.acks);

    // The run is not sustained, and the reason names the limit, not a
    // failure.
    assert!(!run.sustained);
    assert_eq!(run.not_sustained_because.len(), 1, "{:?}", run.not_sustained_because);
    let reason = &run.not_sustained_because[0];
    assert!(reason.contains("a limit of the server refused"), "{reason}");
    assert!(reason.contains("rate_limited"), "{reason}");
    let summary = run.summary();
    assert!(summary.contains("refused by a limit of the server (status 429): "), "{summary}");
    assert!(summary.contains("failed requests: 0 (0.000 %)"), "{summary}");
    assert!(summary.contains("first failure with the code `rate_limited`"), "{summary}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sign_up_limit_of_a_foreign_server_keeps_agents_out_and_is_named() {
    // The limits as the server ships them: 5 sign-ups at once from one
    // address, then 20 for each hour. 8 agents come from one address.
    let server = TestServer::start(Duration::from_millis(20), |_| {}).await;
    let mut config = config(Target::Url(server.url()));
    config.ws_share = 0.0;
    config.ramp_up = Duration::from_millis(200);
    config.duration = Duration::from_millis(1_500);
    let run = swarm::run(&config).await.expect("the swarm runs to its end");

    assert_eq!(run.agents_ready, 5, "the server took five sign-ups");
    assert_eq!(run.signup.count, 5);
    // The three others were refused by the limit, each one time or more.
    // The server asks them to wait minutes, so they are not ready when the
    // run ends.
    let refused = run.throttled_requests + run.throttled_ramp.values().sum::<u64>();
    assert!(refused >= 3, "window: {:?}, ramp-up: {:?}", run.throttled, run.throttled_ramp);
    assert_eq!(run.agents_failed.get("rate_limited"), Some(&3), "{:?}", run.agents_failed);
    assert_eq!(run.failed_requests, 0, "errors: {:?}", run.errors);
    assert!(run.errors_ramp.is_empty(), "errors in the ramp-up: {:?}", run.errors_ramp);
    assert!(!run.sustained);
    let reasons = run.not_sustained_because.join("; ");
    assert!(reasons.contains("only 5 of 8 agents became ready"), "{reasons}");
    assert!(reasons.contains("a limit of the server refused"), "{reasons}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_refuses_actions_is_not_sustained_and_the_errors_are_counted_by_code() {
    let server = TestServer::start(Duration::from_millis(20), no_limits).await;
    server.refuse_acts.store(true, Ordering::Relaxed);
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
    // A server that fails is not a server that limits.
    assert_eq!(run.throttled_requests, 0);
    assert!(run.throttled_ramp.is_empty());
    assert!(!run.sustained);
    let reasons = run.not_sustained_because.join("; ");
    assert!(reasons.contains("8 agents gave up"), "{reasons}");
    assert!(reasons.contains("no action was acknowledged"), "{reasons}");
    assert!(!reasons.contains("a limit of the server"), "{reasons}");
    assert!(run.summary().contains("result: NOT SUSTAINED"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shared_pool_of_connections_serves_all_http_agents() {
    let server = TestServer::start(Duration::from_millis(20), no_limits).await;
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
