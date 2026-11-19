//! The SDKs against the real server, and their fake servers against the
//! real server. The tests run `node` and `python3` as child processes.
//!
//! They are ignored by default, because they need the built JavaScript
//! packages (`npm ci && npm run build` in `sdk/ts`):
//!
//! ```sh
//! cargo test -p unsup-e2e -- --include-ignored
//! ```

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::child::{Finished, run};
use common::{ADMIN, World, config, id, int, no_rate_limits, repo_root};
use reqwest::StatusCode;

/// Time limit of one child process.
const LIMIT: Duration = Duration::from_secs(240);

/// Starts a world for a child process: no limit of requests, because a
/// script sends its requests faster than a real agent.
async fn world_for_scripts() -> World {
    let mut config = config();
    no_rate_limits(&mut config);
    World::with(config).await
}

/// Runs a script of an SDK against a world, with the clock of the test.
async fn script(world: &World, program: &str, dir: &str, file: &str) -> Finished {
    let clock = world.clock().await;
    let dir: PathBuf = repo_root().join(dir);
    let env = [
        ("UNSUPERVISED_URL", world.base.clone()),
        ("E2E_CLOCK_URL", clock.url.clone()),
        ("E2E_ADMIN_TOKEN", ADMIN.to_string()),
    ];
    let finished = run(program, [file], &dir, &env, LIMIT).await;
    assert!(finished.success, "`{program} {file}` in {} works:\n{finished}", dir.display());
    finished
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs node and the built sdk/ts: run with --include-ignored after `npm ci && npm run build`"]
async fn the_typescript_fake_answers_like_the_real_server() {
    let world = world_for_scripts().await;
    let finished = script(&world, "node", "sdk/ts", "e2e/conformance.mjs").await;
    let result = finished.fact("conformance");
    assert_eq!(result["mismatches"], 0, "{finished}");
    assert!(result["probes"].as_u64().is_some_and(|probes| probes >= 100), "{finished}");
    world.stop().await;
}

/// Reads the wealth of a resident from the public leaderboard. Only the
/// script knows the key of its resident, so the test reads what viewers
/// see.
async fn wealth_of(world: &World, resident: u64) -> i64 {
    let board = world.public("/v1/leaderboards/wealth").await;
    let top = board["data"]["top"].as_array().expect("the best residents");
    let entry = top
        .iter()
        .find(|entry| entry["resident"] == resident)
        .unwrap_or_else(|| panic!("the resident {resident} is on the board: {board}"));
    int(&entry["score"], "score")
}

/// Checks the mission that the script published as a sponsor against
/// what viewers see: the world published three missions of this owner,
/// and the first one has the id and the request number of the answer
/// that the SDK got. The name of the sponsor is the name of the owner.
async fn assert_the_mission_is_public(world: &World, facts: &serde_json::Value, owner: &str) {
    let feed = world.public("/v1/feed?limit=500&kind=missions.mission_posted").await;
    let posted = feed["events"].as_array().expect("events");
    let sponsors: Vec<&serde_json::Value> =
        posted.iter().map(|event| &event["data"]["sponsor"]["text"]).collect();
    assert_eq!(sponsors, [owner, owner, owner], "three missions of one sponsor: {feed}");
    let first = &posted[0]["data"];
    assert_eq!(first["mission"], facts["mission"], "{feed}");
    assert_eq!(first["request"], facts["request"], "{feed}");
    assert_eq!(id(&first["reward"], "reward"), 5_000, "{feed}");
    // The module refused one mission of the script in public: its reward was too small.
    let refused = world.public("/v1/feed?limit=500&kind=missions.sponsor_refused").await;
    let codes: Vec<&serde_json::Value> = refused["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| &event["data"]["code"])
        .collect();
    assert_eq!(codes, ["invalid_args"], "{refused}");
}

/// Checks what a script said about its resident against the server: the
/// resident is in the world with its name and its model, the feed has
/// its shift, the missions that it published as a sponsor are public,
/// and it has the money that the script saw.
async fn assert_the_server_agrees(
    world: &World,
    facts: &serde_json::Value,
    name: &str,
    model: &str,
    owner: &str,
) {
    let resident = id(&facts["resident"], "resident");
    assert!(int(&facts["earned"], "earned") > 0, "{facts}");
    world.step(1).await;
    let (status, profile) = world.read(&format!("/v1/residents/{resident}/profile")).await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["name"], name);
    assert_eq!(profile["model"], model);
    let kinds: Vec<&str> = profile["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| event["kind"].as_str().expect("a kind"))
        .collect();
    assert!(kinds.contains(&"economy.shift_completed"), "{kinds:?}");
    assert_the_mission_is_public(world, facts, owner).await;
    // The leaderboard of wealth is published at an interval.
    world.step(60).await;
    assert_eq!(wealth_of(world, resident).await, int(&facts["cash"], "cash"));
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs node and the built sdk/ts: run with --include-ignored after `npm ci && npm run build`"]
async fn the_typescript_sdk_works_against_the_real_server() {
    let world = world_for_scripts().await;
    let finished = script(&world, "node", "sdk/ts", "e2e/real-server.mjs").await;
    let facts = finished.fact("typescript");
    assert_eq!(facts["pushed"], 4, "{finished}");
    assert_the_server_agrees(&world, &facts, "Tess", "sdk-typescript", "TypeScript SDK").await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs python3: run with --include-ignored"]
async fn the_python_sdk_works_against_the_real_server() {
    let world = world_for_scripts().await;
    let finished = script(&world, "python3", "sdk/python", "e2e/real_server.py").await;
    let facts = finished.fact("python");
    assert_the_server_agrees(&world, &facts, "Pia", "sdk-python", "Python SDK").await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs python3: run with --include-ignored"]
async fn the_python_fake_answers_like_the_real_server() {
    let world = world_for_scripts().await;
    let finished = script(&world, "python3", "sdk/python", "e2e/conformance.py").await;
    let result = finished.fact("conformance");
    assert_eq!(result["mismatches"], 0, "{finished}");
    assert!(result["probes"].as_u64().is_some_and(|probes| probes >= 100), "{finished}");
    world.stop().await;
}
