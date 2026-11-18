//! The viewer service in the server: its history survives a restart with
//! a data directory, a new world starts without it, and the `[spectator]`
//! table of the configuration reaches the service.

mod common;

use std::path::Path;
use std::time::Duration;

use common::{Client, SHIFT_PAY, TempDir, advance, durable, earn, send, viewer_shows};
use reqwest::StatusCode;
use serde_json::{Value, json};
use unsup_server::{Overrides, ServerConfig, StoreConfig, WorldShape, WorldStore};

/// Returns the lines of one chapter of a life story.
fn lines(chapter: &Value) -> Vec<&str> {
    chapter
        .as_array()
        .unwrap_or_else(|| panic!("a chapter, not {chapter}"))
        .iter()
        .map(|entry| entry["line"].as_str().expect("a line"))
        .collect()
}

/// Returns the names of the files in a directory, sorted. A directory
/// that does not exist has none.
fn files(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[tokio::test(flavor = "multi_thread")]
async fn a_life_story_survives_a_restart_with_a_data_directory() {
    let dir = TempDir::new();
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (one_id, one) = client.resident(&token, "One").await;
    let (two_id, two) = client.resident(&token, "Two").await;
    // Both work a shift at the depot. One gives a part of its pay to Two.
    earn(&server, &client, &[&one, &two]).await;
    let gift = json!({ "to": two_id, "money": 700 });
    assert_eq!(client.accepted(&one, "give", gift).await, "started");
    advance(&server, 5).await;
    assert_eq!(client.cash(&two).await, SHIFT_PAY + 700);

    // The profile of each has a life story: the arrival and the gift.
    let profile = format!("/v1/residents/{one_id}/profile");
    let before = viewer_shows(&client, &profile, |profile| {
        profile["life"]["deals"].as_array().is_some_and(|deals| !deals.is_empty())
    })
    .await;
    let life = &before["life"];
    assert_eq!(life["born"]["kind"], "kernel.spawned");
    let born = life["born"]["line"].as_str().expect("a line");
    assert!(born.starts_with("One arrived in "), "{born}");
    assert_eq!(lines(&life["deals"]), ["One gave a gift to Two"]);
    let other = format!("/v1/residents/{two_id}/profile");
    let (status, receiver) = send(client.get(&other)).await;
    assert_eq!(status, StatusCode::OK, "{receiver}");
    assert_eq!(lines(&receiver["life"]["deals"]), ["One gave a gift to Two"]);
    assert!(!before["events"].as_array().expect("events").is_empty(), "{before}");
    server.stop().await.expect("the server stops");

    // The history is in the data directory, next to the world.
    let journals = files(&dir.path().join("spectator").join("chronicle"));
    assert_eq!(journals.len(), 1, "{journals:?}");
    assert!(journals[0].starts_with("journal-") && journals[0].ends_with(".jsonl"));
    assert_eq!(
        files(dir.path()),
        ["spectator", "store.sqlite3", "world"],
        "the data directory holds the accounts, the world and the history"
    );

    // A second server on the same directory: the world is recovered, and
    // the life stories are there again. The feed of the new service has
    // no event from before, so the stories come from the directory.
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts again");
    assert!(server.world.recovered);
    let client = Client::of(&server);
    let (status, after) = send(client.get(&profile)).await;
    assert_eq!(status, StatusCode::OK, "{after}");
    assert_eq!(after["life"], before["life"]);
    assert_eq!(after["events"], json!([]));
    assert_eq!(
        (&after["name"], &after["alive"], &after["pos"]),
        (&json!("One"), &json!(true), &before["pos"])
    );
    let (_, receiver_after) = send(client.get(&other)).await;
    assert_eq!(receiver_after["life"], receiver["life"]);

    // The life goes on in the same story.
    let gift = json!({ "to": two_id, "money": 100 });
    assert_eq!(client.accepted(&one, "give", gift).await, "started");
    advance(&server, 5).await;
    assert_eq!(client.cash(&one).await, SHIFT_PAY - 800);
    let later = viewer_shows(&client, &profile, |profile| {
        profile["life"]["deals"].as_array().is_some_and(|deals| deals.len() == 2)
    })
    .await;
    assert_eq!(
        lines(&later["life"]["deals"]),
        ["One gave a gift to Two", "One gave a gift to Two"]
    );
    assert_eq!(later["life"]["born"], before["life"]["born"]);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_world_starts_without_the_history_of_an_earlier_world() {
    let dir = TempDir::new();
    let chronicle = dir.path().join("spectator").join("chronicle");
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (old_id, _) = client.resident(&token, "Old").await;
    advance(&server, 20).await;
    let profile = format!("/v1/residents/{old_id}/profile");
    let old =
        viewer_shows(&client, &profile, |profile| profile["life"].get("born").is_some()).await;
    assert!(old["life"]["born"]["line"].as_str().is_some_and(|line| line.starts_with("Old ")));
    server.stop().await.expect("the server stops");
    assert_eq!(files(&chronicle).len(), 1, "the history of the first world is on the disk");

    // The operator removes the world. The next start makes a new one, and
    // the history of the old world goes with it.
    std::fs::remove_dir_all(dir.path().join("world")).expect("the world is removed");
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    assert!(!server.world.recovered);
    assert_eq!(files(&chronicle), Vec::<String>::new());
    // The new world runs longer than the old one and has nothing to tell.
    advance(&server, 30).await;
    server.stop().await.expect("the server stops");

    // The new world is recovered. Its first resident has the id of the
    // resident of the old world, and a life of its own.
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    assert!(server.world.recovered);
    let client = Client::of(&server);
    let (new_id, _) = client.resident(&token, "New").await;
    assert_eq!(new_id, old_id);
    advance(&server, 1).await;
    let new =
        viewer_shows(&client, &profile, |profile| profile["life"].get("born").is_some()).await;
    let born = new["life"]["born"]["line"].as_str().expect("a line");
    assert!(born.starts_with("New arrived in "), "{born}");
    assert_eq!(new["life"]["born"]["frame"], 31);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_delay_of_the_configuration_delays_the_feed() {
    // The `[spectator]` table of a configuration file, as an operator
    // writes it.
    let dir = TempDir::new();
    let file = dir.write("server.toml", "[spectator.stream]\ndelay_frames = 3\n");
    let loaded = ServerConfig::load(Some(&file), Overrides::default()).expect("a configuration");
    assert_eq!(loaded.spectator.stream.delay_frames, 3);
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("an address"),
        tick_interval: Duration::ZERO,
        world: WorldShape { seed: 7, cities: 1, districts: 1 },
        web_dir: None,
        store: StoreConfig::Memory,
        world_store: WorldStore::Memory,
        ..loaded
    };
    let server = unsup_server::start(config).await.expect("the server starts");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (resident, key) = client.resident(&token, "One").await;
    let profile = format!("/v1/residents/{resident}/profile");
    let arrivals = "/v1/feed?kind=kernel.spawned";

    // The world is at frame 3. Viewers get the world of frame 0: nobody
    // is there. The agent of the resident sees its resident at once.
    server.sim.step(3).await.expect("the world steps");
    let stats = viewer_shows(&client, "/v1/spectate/stats", |_| true).await;
    assert_eq!(
        (&stats["frame"], &stats["delay_frames"], &stats["residents"]),
        (&json!(0), &json!(3), &json!(0))
    );
    let (status, feed) = send(client.get(arrivals)).await;
    assert_eq!((status, &feed["events"]), (StatusCode::OK, &json!([])));
    let (status, unknown) = send(client.get(&profile)).await;
    assert_eq!(
        (status, &unknown["error"]["code"]),
        (StatusCode::NOT_FOUND, &json!("unknown_resident"))
    );
    let (status, me) = send(client.get("/v1/me").bearer_auth(&key)).await;
    assert_eq!((status, &me["resident"]), (StatusCode::OK, &json!(resident)));
    assert_eq!(server.sim.stats().await.expect("stats").frame, 3);

    // One frame later viewers get frame 1, with the arrival.
    server.sim.step(1).await.expect("the world steps");
    let stats = viewer_shows(&client, "/v1/spectate/stats", |stats| stats["frame"] == 1).await;
    assert_eq!((&stats["delay_frames"], &stats["residents"]), (&json!(3), &json!(1)));
    let (_, feed) = send(client.get(arrivals)).await;
    let events = feed["events"].as_array().expect("events");
    assert_eq!(events.len(), 1, "{feed}");
    assert_eq!((&events[0]["frame"], &events[0]["actor"]), (&json!(1), &json!(resident)));
    let (status, shown) = send(client.get(&profile)).await;
    assert_eq!(
        (status, &shown["name"], &shown["frame"]),
        (StatusCode::OK, &json!("One"), &json!(1))
    );
    assert_eq!(server.sim.stats().await.expect("stats").frame, 4);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_dashboard_has_the_newest_snapshot_with_its_state_hash() {
    let dir = TempDir::new();
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    let client = Client::of(&server);
    let first = client.dashboard().await["host"]["newest_snapshot"].clone();
    assert_eq!((&first["frame"], &first["inputs"]), (&json!(0), &json!(0)));
    assert_eq!(first["state_hash"], json!(server.world.state_hash));

    let token = client.sign_up("Ada").await;
    client.resident(&token, "One").await;
    // Frame 50 has a snapshot: one spawn and 50 frames.
    advance(&server, 50).await;
    let host = client.dashboard().await["host"].clone();
    assert_eq!(
        host["snapshots"],
        json!([{ "frame": 0, "inputs": 0 }, { "frame": 50, "inputs": 51 }])
    );
    let newest = &host["newest_snapshot"];
    assert_eq!((&newest["frame"], &newest["inputs"]), (&json!(50), &json!(51)));
    assert_ne!(newest["state_hash"], first["state_hash"]);
    // Nothing changed the world since: the stop finds the same state.
    let stopped = server.stop_with_report().await.expect("the server stops");
    assert_eq!(stopped.frame, 50);
    assert_eq!(newest["state_hash"], json!(stopped.state_hash));
}
