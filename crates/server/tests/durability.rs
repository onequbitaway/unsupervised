//! The durable world over real HTTP: a stop and a start, a server that
//! is killed, a rollback, the check of a data directory, and the
//! parameter files.

mod common;

use std::path::Path;

use common::{ADMIN, Client, SHIFT_PAY, TempDir, advance, config, copy_dir, durable, earn, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use unsup_runtime::ReplayFrom;
use unsup_server::{ServerConfig, StoreConfig, WorldStore, audit_world};

/// What a client can read about a world: the residents of the owner, and
/// the place, the cash and the conditions of each.
async fn picture(client: &Client, owner_token: &str, keys: &[&str]) -> Value {
    let (status, listed) = send(client.get("/v1/residents").bearer_auth(owner_token)).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let mut residents = Vec::new();
    for key in keys {
        let (status, me) = send(client.get("/v1/me").bearer_auth(key)).await;
        assert_eq!(status, StatusCode::OK, "{me}");
        let myself = client.myself(key).await;
        residents.push(json!({
            "id": me["resident"], "name": me["name"], "owner": me["owner"],
            "pos": myself["pos"], "cash": myself["cash"], "conditions": myself["conditions"],
            "attrs": myself["attrs"],
        }));
    }
    json!({ "listed": listed["residents"], "residents": residents })
}

/// Makes an owner with two residents. The first one works a shift and
/// gives a part of the pay to the second one.
async fn two_residents_with_cash(
    server: &unsup_server::Server,
    client: &Client,
) -> (String, String, String) {
    let token = client.sign_up("Ada").await;
    let (_, one) = client.resident(&token, "One").await;
    let (two_id, two) = client.resident(&token, "Two").await;
    earn(server, client, &[&one, &two]).await;
    assert_eq!(
        client.accepted(&one, "give", json!({ "to": two_id, "money": 700 })).await,
        "started"
    );
    advance(server, 5).await;
    assert_eq!(
        (client.cash(&one).await, client.cash(&two).await),
        (SHIFT_PAY - 700, SHIFT_PAY + 700)
    );
    (token, one, two)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_stops_and_starts_has_the_same_world() {
    let dir = TempDir::new();
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    assert!(!server.world.recovered, "an empty directory gets a new world");
    assert_eq!(server.world.frame, 0);
    let client = Client::of(&server);
    let (token, one, two) = two_residents_with_cash(&server, &client).await;
    let before = picture(&client, &token, &[&one, &two]).await;
    let stopped = server.stop_with_report().await.expect("the server stops");
    assert_eq!(stopped.residents, 2);
    assert_eq!(stopped.state_hash.len(), 64);

    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts again");
    let world = server.world.clone();
    assert!(world.recovered);
    assert_eq!((world.frame, &world.state_hash), (stopped.frame, &stopped.state_hash));
    // The stop wrote a final snapshot: the start replays nothing.
    assert_eq!((world.snapshot_frame, world.replayed_inputs), (stopped.frame, 0));
    assert!(!world.torn_tail && world.skipped_snapshots.is_empty(), "{world:?}");
    let client = Client::of(&server);
    // The old keys and the old token work. Everything is as it was.
    assert_eq!(picture(&client, &token, &[&one, &two]).await, before);
    // The resident ids go on. No record ended.
    let (third, _) = client.resident(&token, "Three").await;
    assert_eq!(third, 3);
    let (status, stats) = client.admin(Method::GET, "/admin/stats", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(stats["store"], json!({ "owners": 1, "residents": 3, "retired": 0, "webhooks": 0 }));
    assert_eq!(stats["sim"]["frame"], stopped.frame);
    server.stop().await.expect("the server stops");
}

/// Adds bytes to the end of the newest log segment, as a crash in the
/// middle of a write leaves them.
fn tear_the_log(world: &Path) {
    let mut segments: Vec<_> = std::fs::read_dir(world.join("log"))
        .expect("the log directory")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    segments.sort();
    let newest = segments.last().expect("a log segment");
    let mut bytes = std::fs::read(newest).expect("the segment");
    // The start of a record: a length, and then not enough bytes.
    bytes.extend_from_slice(&[200, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7]);
    std::fs::write(newest, bytes).expect("the segment is written");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_server_that_is_killed_recovers_the_world_that_it_had() {
    let dir = TempDir::new();
    let alive = dir.path().join("alive");
    let server = unsup_server::start(durable(&alive)).await.expect("the server starts");
    let client = Client::of(&server);
    let (token, one, two) = two_residents_with_cash(&server, &client).await;
    // More inputs after the last snapshot: they are in the log only.
    advance(&server, 7).await;
    assert_eq!(client.accepted(&one, "say", json!({ "text": "still here" })).await, "done");
    let frame = advance(&server, 1).await;
    assert_eq!(frame, 318, "the newest snapshot is that of frame 300");
    let before = picture(&client, &token, &[&one, &two]).await;

    // A kill leaves the files as they are at one moment: no clean stop,
    // no final snapshot. The copies are those files. One copy also has
    // the start of a record that the kill cut off.
    let killed = dir.path().join("killed");
    let torn = dir.path().join("torn");
    copy_dir(&alive, &killed);
    copy_dir(&alive, &torn);
    tear_the_log(&torn.join("world"));
    // The world that the server had in memory at that moment.
    let live = server.stop_with_report().await.expect("the server stops");
    assert_eq!(live.frame, frame);

    for (place, cut) in [(&killed, false), (&torn, true)] {
        let server = unsup_server::start(durable(place)).await.expect("the server recovers");
        let world = server.world.clone();
        assert!(world.recovered);
        // The same world: the same frame and the same state hash.
        assert_eq!((world.frame, &world.state_hash), (live.frame, &live.state_hash), "{world:?}");
        // The newest snapshot is older than the kill. The log has the
        // rest: 18 frames, the gift of the first resident and the `say`.
        assert_eq!((world.snapshot_frame, world.replayed_frames), (300, 18));
        assert_eq!(world.replayed_inputs, 20);
        assert_eq!(world.torn_tail, cut);
        let client = Client::of(&server);
        // The same residents with the same balances, with the old keys.
        assert_eq!(picture(&client, &token, &[&one, &two]).await, before);
        let dashboard = client.dashboard().await;
        assert_eq!(dashboard["host"]["start"]["state_hash"], json!(live.state_hash));
        assert_eq!(dashboard["host"]["start"]["recovered"], true);
        assert_eq!(dashboard["host"]["audit"]["failures"], 0);
        assert_eq!(dashboard["host"]["audit"]["audits"], 1, "the recovered world was audited");
        assert_eq!(dashboard["alerts"], json!([]));
        // The world goes on from there.
        advance(&server, 3).await;
        let stopped = server.stop_with_report().await.expect("the server stops");
        assert_eq!(stopped.frame, frame + 3);
        assert_ne!(stopped.state_hash, live.state_hash);

        // The check of the directory replays the whole log from the
        // oldest snapshot and ends in the same state.
        let audit = audit_world(&place.join("world"), ReplayFrom::Oldest).expect("the audit");
        assert!(audit.consistent, "{audit:?}");
        assert_eq!((audit.frame, &audit.state_hash), (stopped.frame, &stopped.state_hash));
        assert_eq!(audit.residents, 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_check_of_a_directory_compares_every_snapshot_and_audits_the_money() {
    let dir = TempDir::new();
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    let client = Client::of(&server);
    let (_, one, _) = two_residents_with_cash(&server, &client).await;
    let world = dir.path().join("world");

    // A server that runs holds its directory.
    let error = audit_world(&world, ReplayFrom::Oldest).expect_err("the directory is in use");
    let message = format!("{error:#}");
    assert!(message.contains("the directory is in use"), "{message}");
    assert!(message.contains("stop it, or copy the directory first"), "{message}");
    let second = unsup_server::start(durable(dir.path())).await;
    let message = format!("{:#}", second.err().expect("two servers cannot share a directory"));
    assert!(message.contains("cannot open the world directory"), "{message}");

    // The server still works after both tries.
    assert_eq!(client.cash(&one).await, SHIFT_PAY - 700);
    let stopped = server.stop_with_report().await.expect("the server stops");

    let audit = audit_world(&world, ReplayFrom::Oldest).expect("the audit");
    assert!(audit.consistent, "{audit:?}");
    assert!(audit.snapshot_hash_ok && audit.recovery_matches && !audit.torn_tail);
    assert_eq!((audit.frame, &audit.state_hash), (stopped.frame, &stopped.state_hash));
    // The replay starts at the snapshot of the new world. It passes a
    // snapshot every 50 frames and the one of the stop: each has the
    // state hash that the replay gives.
    assert_eq!((audit.snapshot_frame, stopped.frame), (0, 310));
    let passed: Vec<(u64, bool)> = audit.snapshots.iter().map(|s| (s.frame, s.matches)).collect();
    assert_eq!(passed, [50, 100, 150, 200, 250, 300, 310].map(|frame| (frame, true)), "{audit:?}");
    assert_eq!(audit.frames, 310);
    assert!(audit.inputs > audit.frames);
    // Two shifts were paid from the faucet of the public works. No money
    // was destroyed. The tax went to the city, so all of it is held.
    let money = audit.money.expect("the money adds up");
    assert_eq!((money.minted, money.burned, money.held + money.in_flight), (4_000, 0, 4_000));
    assert_eq!(audit.audit_error, None);

    // From the newest snapshot there is nothing to replay.
    let newest = audit_world(&world, ReplayFrom::Newest).expect("the audit");
    assert_eq!((newest.inputs, newest.snapshot_frame), (0, stopped.frame));
    assert_eq!(newest.state_hash, audit.state_hash);
    // A snapshot whose header changed cannot be compared. The check
    // says so, and the log still gives the world.
    let mut snapshots: Vec<_> = std::fs::read_dir(world.join("snapshots"))
        .expect("the snapshot directory")
        .map(|entry| entry.expect("an entry").path())
        .collect();
    snapshots.sort();
    let last = snapshots.last().expect("a snapshot");
    let mut bytes = std::fs::read(last).expect("the snapshot");
    bytes[40] ^= 0xff;
    std::fs::write(last, bytes).expect("the snapshot is written");
    let damaged = audit_world(&world, ReplayFrom::Oldest).expect("the audit");
    assert_eq!(damaged.snapshots.len(), 6);
    assert_eq!(damaged.skipped.len(), 1, "{damaged:?}");
    assert!(damaged.skipped[0].contains("-00000000000000000310.snap"), "{damaged:?}");
    assert_eq!(damaged.state_hash, audit.state_hash);
    assert!(damaged.consistent && damaged.recovery_matches, "{damaged:?}");

    // A directory without a world.
    let empty = TempDir::new();
    let error = audit_world(empty.path(), ReplayFrom::Oldest).expect_err("no world");
    assert!(format!("{error:#}").ends_with("holds no world"), "{error:#}");
    let error = audit_world(&empty.path().join("missing"), ReplayFrom::Oldest).expect_err("none");
    assert!(format!("{error:#}").contains("does not exist"), "{error:#}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_rollback_is_for_good_and_sanctions_outlive_it_and_a_restart() {
    let dir = TempDir::new();
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (_, one) = client.resident(&token, "One").await;
    let (two_id, two) = client.resident(&token, "Two").await;
    let other_token = client.sign_up("Bob").await;
    let (_, bob) = client.resident(&other_token, "Bob").await;
    // Frame 50 has a snapshot: three residents, nobody has cash.
    advance(&server, 50).await;
    let dashboard = client.dashboard().await;
    let frames: Vec<&Value> = dashboard["host"]["snapshots"]
        .as_array()
        .expect("snapshots")
        .iter()
        .map(|snapshot| &snapshot["frame"])
        .collect();
    assert_eq!(frames, [0, 50], "the dashboard lists where a rollback can go");

    // After the snapshot: a shift is paid, a resident appears, and the
    // administrator mutes One, freezes Two and bans the owner of Bob.
    earn(&server, &client, &[&one]).await;
    let (late_id, late) = client.resident(&token, "Late").await;
    let post = |path: String, body: Option<Value>| {
        let client = &client;
        async move { client.admin(Method::POST, &path, body).await }
    };
    let reason = |text: &str| Some(json!({ "reason": text }));
    let (status, _) = post("/admin/residents/1/mute".into(), reason("spam")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post(format!("/admin/residents/{two_id}/freeze"), reason("stuck")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = post("/admin/owners/2/ban".into(), reason("bots")).await;
    assert_eq!(status, StatusCode::OK);
    let frame = advance(&server, 1).await;
    assert!(frame > 300, "{frame}");
    assert_eq!(client.cash(&one).await, SHIFT_PAY);

    // Back to the newest snapshot at or before frame 99: frame 50.
    let (status, answer) = post("/admin/rollback".into(), Some(json!({ "frame": 99 }))).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        (answer["frame"].clone(), answer["retired"].clone(), answer["orphans"].clone()),
        (json!(50), json!([late_id]), json!([]))
    );
    assert_eq!(answer["refrozen"], 1, "the freeze of Two is from after the snapshot");
    assert_eq!(client.cash(&one).await, 0, "the shift never happened");
    let (status, answer) = send(client.get("/v1/me").bearer_auth(&late)).await;
    assert_eq!(
        (status, answer["error"]["code"].clone()),
        (StatusCode::UNAUTHORIZED, json!("invalid_token"))
    );
    // The snapshots from after frame 50 are gone with the rollback.
    let dashboard = client.dashboard().await;
    assert_eq!(dashboard["snapshots"]["rollbacks"], 1);
    assert_eq!(dashboard["sim"]["frame"], 50);
    // The snapshot of frame 50 holds three spawns and 50 frames.
    assert_eq!(
        dashboard["host"]["snapshots"],
        json!([{ "frame": 0, "inputs": 0 }, { "frame": 50, "inputs": 53 }])
    );
    let sanctions_before = client.admin(Method::GET, "/admin/sanctions", None).await.1;
    advance(&server, 4).await;
    let stopped = server.stop_with_report().await.expect("the server stops");
    assert_eq!(stopped.frame, 54);

    // The rollback is permanent: a start gives the world of the snapshot
    // plus what came after the rollback.
    let server = unsup_server::start(durable(dir.path())).await.expect("the server starts again");
    assert_eq!((server.world.frame, &server.world.state_hash), (54, &stopped.state_hash));
    let client = Client::of(&server);
    assert_eq!(client.cash(&one).await, 0);
    // The three sanctions are in force after the rollback and the restart.
    let (status, sanctions) = client.admin(Method::GET, "/admin/sanctions", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(sanctions, sanctions_before);
    let kinds: Vec<(&Value, &Value)> = sanctions["sanctions"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|sanction| (&sanction["kind"], &sanction["reason"]))
        .collect();
    assert_eq!(
        kinds,
        [
            (&json!("mute"), &json!("spam")),
            (&json!("freeze"), &json!("stuck")),
            (&json!("ban"), &json!("bots"))
        ]
    );
    let say = json!({ "verb": "say", "args": { "text": "hello" } });
    let (status, answer) = send(client.post("/v1/act", &say).bearer_auth(&one)).await;
    assert_eq!((status, answer["error"]["code"].clone()), (StatusCode::FORBIDDEN, json!("muted")));
    let answer = client.act(&two, "wait", json!({ "ticks": 1 })).await;
    assert_eq!(answer["acks"][0]["status"]["error"]["code"], "blocked", "{answer}");
    assert_eq!(client.myself(&two).await["conditions"], json!({ "frozen": null }));
    for token in [&bob, &other_token] {
        let (status, answer) = send(client.get("/v1/residents").bearer_auth(token)).await;
        assert_eq!(
            (status, answer["error"]["code"].clone()),
            (StatusCode::FORBIDDEN, json!("banned"))
        );
    }

    // Each one is lifted, and that holds after one more restart.
    for path in
        ["/admin/residents/1/unmute", "/admin/residents/2/unfreeze", "/admin/owners/2/unban"]
    {
        let (status, answer) = client.admin(Method::POST, path, None).await;
        assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })), "{path}");
    }
    server.stop().await.expect("the server stops");
    let server = unsup_server::start(durable(dir.path())).await.expect("the third start");
    let client = Client::of(&server);
    let (_, sanctions) = client.admin(Method::GET, "/admin/sanctions", None).await;
    assert_eq!(sanctions, json!({ "sanctions": [] }));
    assert_eq!(client.accepted(&one, "say", json!({ "text": "hello" })).await, "done");
    assert_eq!(client.accepted(&two, "wait", json!({ "ticks": 1 })).await, "started");
    let (status, me) = send(client.get("/v1/me").bearer_auth(&bob)).await;
    assert_eq!((status, me["name"].clone()), (StatusCode::OK, json!("Bob")));
    server.stop().await.expect("the server stops");
    let _ = ADMIN;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_world_in_memory_is_new_at_each_start_and_can_still_roll_back() {
    let dir = TempDir::new();
    // Accounts in a file, the world in memory.
    let in_memory = || ServerConfig { world_store: WorldStore::Memory, ..durable(dir.path()) };
    let server = unsup_server::start(in_memory()).await.expect("the server starts");
    assert!(!dir.path().join("world").exists(), "nothing of the world is on the disk");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (_, key) = client.resident(&token, "One").await;
    advance(&server, 50).await;
    earn(&server, &client, &[&key]).await;
    let (status, answer) =
        client.admin(Method::POST, "/admin/rollback", Some(json!({ "frame": 60 }))).await;
    assert_eq!((status, answer["frame"].clone()), (StatusCode::OK, json!(50)), "{answer}");
    assert_eq!(client.cash(&key).await, 0);
    assert_eq!(client.dashboard().await["host"]["world_store"], "memory");
    server.stop().await.expect("the server stops");

    let server = unsup_server::start(in_memory()).await.expect("the server starts again");
    assert!(!server.world.recovered);
    let client = Client::of(&server);
    // The owner is there. Its resident ended with the old world.
    let (status, listed) = send(client.get("/v1/residents").bearer_auth(&token)).await;
    assert_eq!((status, listed["residents"].clone()), (StatusCode::OK, json!([])));
    server.stop().await.expect("the server stops");

    // Accounts in memory: the world is in memory too, whatever the
    // configuration says, and no directory is made.
    let lost = TempDir::new();
    let mixed = ServerConfig {
        store: StoreConfig::Memory,
        world_store: WorldStore::Dir(lost.path().join("world")),
        ..config()
    };
    let server = unsup_server::start(mixed).await.expect("the server starts");
    assert!(!lost.path().join("world").exists());
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn parameter_files_change_a_new_world_and_not_a_recovered_one() {
    let dir = TempDir::new();
    let wage = |cents: i64| {
        format!(
            "[params.\"economy.public_works_wage\"]\nvalue = {cents}\nmin = 0\nmax = 1000000\n\
             policy = true\nsummary = \"Cents that a starter shift pays.\"\n"
        )
    };
    let first = dir.write("first.toml", &wage(3_000));
    // A later file wins.
    let second = dir.write("second.toml", &wage(5_000));
    let data = dir.path().join("data");
    let with = |files: Vec<std::path::PathBuf>| ServerConfig { params: files, ..durable(&data) };

    let server =
        unsup_server::start(with(vec![first.clone(), second])).await.expect("the server starts");
    let client = Client::of(&server);
    let read_wage = |client: &Client| {
        let request = client.get("/v1/catalogue");
        async move {
            let (_, catalogue) = send(request).await;
            let params = catalogue["params"].as_array().expect("params").clone();
            let wage = params.iter().find(|p| p["name"] == "economy.public_works_wage");
            wage.expect("the parameter")["value"].clone()
        }
    };
    assert_eq!(read_wage(&client).await, 5_000);
    server.stop().await.expect("the server stops");

    // The world is recovered: it keeps the wage that it has, also with
    // another file or with none.
    for files in [vec![first.clone()], Vec::new()] {
        let server = unsup_server::start(with(files)).await.expect("the server starts again");
        assert!(server.world.recovered);
        assert_eq!(read_wage(&Client::of(&server)).await, 5_000);
        server.stop().await.expect("the server stops");
    }

    // A file that cannot be read, or that is not a data file, stops the
    // start of a new world.
    let fresh = dir.path().join("fresh");
    let missing = dir.path().join("no-such-file.toml");
    let broken = ServerConfig { params: vec![missing], ..durable(&fresh) };
    let message = format!("{:#}", unsup_server::start(broken).await.err().expect("a refusal"));
    assert!(message.contains("cannot read the parameter file"), "{message}");
    let bad = dir.write("bad.toml", "[params.\"economy.public_works_wage\"]\nvalue = \"much\"\n");
    let broken = ServerConfig { params: vec![bad], ..durable(&fresh) };
    let message = format!("{:#}", unsup_server::start(broken).await.err().expect("a refusal"));
    assert!(message.contains("cannot build the world"), "{message}");
}
