//! The `unsupervised` command with a data directory: a server that is
//! killed and started again, the `replay` command and the `admin`
//! command.

#![cfg(unix)]

mod common;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use common::{Client, DEPOT, SHIFT_PAY, TempDir, send};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

/// Time after which a command that says nothing counts as broken.
const PATIENCE: Duration = Duration::from_secs(120);
const ADMIN_TOKEN: &str = "admin-token-of-the-command-tests";

/// The environment variables that the command reads.
const VARIABLES: [&str; 17] = [
    "UNSUPERVISED_CONFIG",
    "UNSUPERVISED_BIND",
    "UNSUPERVISED_TICK_MS",
    "UNSUPERVISED_SEED",
    "UNSUPERVISED_CITIES",
    "UNSUPERVISED_DISTRICTS",
    "UNSUPERVISED_WEB_DIR",
    "UNSUPERVISED_DATA_DIR",
    "UNSUPERVISED_STORE",
    "UNSUPERVISED_WORLD_STORE",
    "UNSUPERVISED_PARAMS",
    "UNSUPERVISED_JOIN_FILE",
    "UNSUPERVISED_ADMIN_TOKEN",
    "UNSUPERVISED_ADMIN_URL",
    "UNSUPERVISED_GITHUB_CLIENT_ID",
    "UNSUPERVISED_GITHUB_CLIENT_SECRET",
    "RUST_LOG",
];

/// Makes the command, without the variables of the caller.
fn unsupervised() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_unsupervised"));
    for variable in VARIABLES {
        command.env_remove(variable);
    }
    command
}

/// A running `unsupervised run`.
struct Running {
    child: Child,
    base: String,
    /// Reads the output and the log until the command ends.
    streams: JoinHandle<(String, String)>,
}

impl Running {
    /// Starts a server on the data directory, with a small world.
    async fn start(data: &Path, config: &Path, tick_ms: u64) -> Running {
        let mut command = unsupervised();
        command
            .args(["run", "--bind", "127.0.0.1:0", "--cities", "1", "--districts", "1"])
            .arg("--tick-ms")
            .arg(tick_ms.to_string())
            .arg("--data-dir")
            .arg(data)
            .arg("--config")
            .arg(config)
            .env("UNSUPERVISED_ADMIN_TOKEN", ADMIN_TOKEN);
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("the command starts");
        let mut stdout = BufReader::new(child.stdout.take().expect("the output stream")).lines();
        let mut stderr = child.stderr.take().expect("the error stream");
        let log = tokio::spawn(async move {
            let mut text = String::new();
            stderr.read_to_string(&mut text).await.expect("the log is text");
            text
        });
        // The two lines of the announcement.
        let mut printed = Vec::new();
        while printed.len() < 2 {
            let line = tokio::time::timeout(PATIENCE, stdout.next_line())
                .await
                .expect("the command prints in time")
                .expect("the output is text");
            match line {
                Some(line) => printed.push(line),
                None => panic!("the command ended early: {printed:?}\n{}", log.await.expect("log")),
            }
        }
        let base = printed[0].strip_prefix("Unsupervised listens on ").expect("the address");
        let base = base.to_string();
        let streams = tokio::spawn(async move {
            let mut rest = printed.join("\n");
            while let Ok(Some(line)) = stdout.next_line().await {
                rest.push('\n');
                rest.push_str(&line);
            }
            (rest, log.await.expect("the log was read"))
        });
        Running { child, base, streams }
    }

    /// Sends a signal and waits for the end. Returns the exit code, what
    /// the command printed and its log.
    async fn end_with(mut self, signal: &str) -> (Option<i32>, String, String) {
        let id = self.child.id().expect("the command runs").to_string();
        let sent = std::process::Command::new("kill")
            .args([signal, &id])
            .status()
            .expect("the kill command runs");
        assert!(sent.success(), "the signal was sent");
        let status = tokio::time::timeout(PATIENCE, self.child.wait())
            .await
            .expect("the command ends in time")
            .expect("the command ends");
        let (printed, log) = self.streams.await.expect("the streams were read");
        (status.code(), printed, log)
    }
}

/// Runs a command that ends by itself. Returns the exit code, the output
/// and the error output.
async fn run(command: &mut Command) -> (Option<i32>, String, String) {
    let output = tokio::time::timeout(PATIENCE, command.stdin(Stdio::null()).output())
        .await
        .expect("the command ends in time")
        .expect("the command runs");
    let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    (output.status.code(), text(&output.stdout), text(&output.stderr))
}

/// Runs `unsupervised replay --json` on a data directory.
async fn replay(data: &Path) -> Value {
    let (code, printed, error) =
        run(unsupervised().args(["replay", "--json", "--data-dir"]).arg(data)).await;
    assert_eq!(code, Some(0), "{error}");
    serde_json::from_str(&printed).unwrap_or_else(|_| panic!("JSON, not {printed:?}"))
}

/// Runs `unsupervised admin` against a server. Returns the exit code,
/// the output and the error output.
async fn admin(base: &str, token: Option<&str>, args: &[&str]) -> (Option<i32>, String, String) {
    let mut command = unsupervised();
    command.args(["admin", "--url", base]).args(args);
    if let Some(token) = token {
        command.env("UNSUPERVISED_ADMIN_TOKEN", token);
    }
    run(&mut command).await
}

/// Runs an admin command that must work, and returns its JSON answer.
async fn admin_ok(base: &str, args: &[&str]) -> Value {
    let (code, printed, error) = admin(base, Some(ADMIN_TOKEN), args).await;
    assert_eq!(code, Some(0), "{args:?}: {error}");
    serde_json::from_str(&printed).unwrap_or_else(|_| panic!("JSON, not {printed:?}"))
}

/// Waits for the next action report of a resident and returns it.
async fn next_report(client: &Client, key: &str, since: u64) -> Value {
    let path = format!("/v1/wait?timeout=60&kinds=report&since={since}");
    let (status, news) = send(client.get(&path).bearer_auth(key)).await;
    assert_eq!(status, StatusCode::OK, "{news}");
    let reports = news["sections"]["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "{news}");
    reports[0].clone()
}

#[tokio::test]
async fn a_killed_server_starts_again_with_the_same_residents_balances_and_state_hash() {
    let dir = TempDir::new();
    let data = dir.path().join("data");
    // Every snapshot stays, however long the test runs: the check starts
    // at the first one, and the rollback below needs an early one.
    let config = dir.write(
        "server.toml",
        "[durability]\nsnapshot_every_frames = 100\nkeep_snapshots = 100000\n",
    );

    // A world that moves fast: one game minute in 2 milliseconds.
    let running = Running::start(&data, &config, 2).await;
    let client = Client::at(&running.base);
    let token = client.sign_up("Ada").await;
    let (resident, key) = client.resident(&token, "One").await;
    let (idle, idle_key) = client.resident(&token, "Two").await;
    // The first resident walks to the depot and works a starter shift.
    let walk = json!({ "x": DEPOT.0, "y": DEPOT.1 });
    assert_eq!(client.accepted(&key, "move_to", walk).await, "started");
    let walked = next_report(&client, &key, 1).await;
    assert_eq!((walked["verb"].clone(), walked["ok"].clone()), (json!("move_to"), json!(true)));
    assert_eq!(client.accepted(&key, "work", json!({})).await, "started");
    let worked = next_report(&client, &key, 2).await;
    assert_eq!(worked["result"]["net"], SHIFT_PAY, "{worked}");
    let before = (client.myself(&key).await, client.myself(&idle_key).await);
    assert_eq!(before.0["cash"], SHIFT_PAY);
    assert_eq!(before.0["pos"], json!({ "x": DEPOT.0, "y": DEPOT.1 }));

    // The process is killed. It writes nothing more: no final snapshot.
    let (code, printed, _) = running.end_with("-KILL").await;
    assert_eq!(code, None, "a signal ended the process");
    assert!(!printed.contains("Stopping."), "{printed}");

    // The check of the directory: the log gives one world, from the
    // oldest snapshot and from the newest one.
    let checked = replay(&data).await;
    assert_eq!(checked["consistent"], true, "{checked}");
    assert_eq!(checked["snapshot_frame"], 0);
    assert_eq!(checked["residents"], 2);
    let frame = checked["frame"].as_u64().expect("a frame");
    assert!(frame > 250, "the shift alone has 240 frames: {checked}");
    let state_hash = checked["state_hash"].as_str().expect("a state hash").to_string();
    assert!(!checked["snapshots"].as_array().expect("snapshots").is_empty(), "{checked}");
    assert_eq!(checked["money"]["minted"], 2_000, "one shift was paid");

    // The same directory, a world that does not move by itself.
    let running = Running::start(&data, &config, 0).await;
    let client = Client::at(&running.base);
    let dashboard = admin_ok(&running.base, &["dashboard"]).await;
    let start = &dashboard["host"]["start"];
    assert_eq!(start["recovered"], true, "{start}");
    assert_eq!(
        (start["frame"].clone(), start["state_hash"].clone()),
        (json!(frame), json!(state_hash))
    );
    assert!(start["replayed_inputs"].as_u64().is_some_and(|inputs| inputs > 0), "{start}");
    assert_eq!(dashboard["sim"]["frame"], frame);
    assert_eq!(dashboard["sim"]["residents"], 2);
    assert_eq!(dashboard["alerts"], json!([]));
    // The same residents, with the old keys, as they were.
    let (status, listed) = send(client.get("/v1/residents").bearer_auth(&token)).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let ids: Vec<&Value> =
        listed["residents"].as_array().expect("residents").iter().map(|r| &r["id"]).collect();
    assert_eq!(ids, [&json!(resident), &json!(idle)]);
    let mut after = (client.myself(&key).await, client.myself(&idle_key).await);
    // Needs go down with time. The kill came some frames after the read.
    for (now, then) in [(&mut after.0, &before.0), (&mut after.1, &before.1)] {
        assert!(now["attrs"]["need.food"].as_i64() <= then["attrs"]["need.food"].as_i64());
        now["attrs"] = then["attrs"].clone();
    }
    assert_eq!(after, before);

    // The `admin` command: the routes, the refusals, and no token on the
    // screen.
    let answer = admin_ok(&running.base, &["mute", "1", "--reason", "too loud"]).await;
    assert_eq!(answer, json!({ "ok": true }));
    let answer = admin_ok(&running.base, &["freeze", "2"]).await;
    assert_eq!(answer, json!({ "ok": true }));
    let listed = admin_ok(&running.base, &["sanctions"]).await;
    let kinds: Vec<(&Value, &Value)> = listed["sanctions"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|sanction| (&sanction["kind"], &sanction["reason"]))
        .collect();
    assert_eq!(kinds, [(&json!("mute"), &json!("too loud")), (&json!("freeze"), &json!(""))]);
    let say = json!({ "verb": "say", "args": { "text": "hello" } });
    let (status, answer) = send(client.post("/v1/act", &say).bearer_auth(&key)).await;
    assert_eq!((status, answer["error"]["code"].clone()), (StatusCode::FORBIDDEN, json!("muted")));
    for args in [["unmute", "1"], ["unfreeze", "2"]] {
        assert_eq!(admin_ok(&running.base, &args).await, json!({ "ok": true }));
    }
    assert_eq!(admin_ok(&running.base, &["sanctions"]).await, json!({ "sanctions": [] }));
    assert_eq!(admin_ok(&running.base, &["flags"]).await["flags"], json!([]));
    assert_eq!(admin_ok(&running.base, &["alerts"]).await, json!({ "alerts": [] }));
    let answer = admin_ok(&running.base, &["set-param", "kernel.queue_max", "3"]).await;
    assert_eq!(answer, json!({ "ok": true }));
    assert_eq!(admin_ok(&running.base, &["stats"]).await["store"]["residents"], 2);

    // A refusal of the server ends the command with an error that names
    // the code. The token is never printed.
    let wrong = "a-wrong-admin-token-of-the-test";
    let (code, printed, error) = admin(&running.base, Some(wrong), &["dashboard"]).await;
    assert_eq!((code, printed.as_str()), (Some(1), ""));
    assert!(
        error.contains("/admin/dashboard answered 401 Unauthorized (invalid_token)"),
        "{error}"
    );
    assert!(!error.contains(wrong), "{error}");
    let (code, _, error) = admin(&running.base, Some(ADMIN_TOKEN), &["mute", "99"]).await;
    assert_eq!(code, Some(1));
    assert!(error.contains("answered 404 Not Found (not_found)"), "{error}");
    assert!(!error.contains(ADMIN_TOKEN), "{error}");
    let (code, _, error) = admin(&running.base, None, &["dashboard"]).await;
    assert_eq!(code, Some(1));
    assert!(error.contains("the admin token is missing"), "{error}");

    // A rollback through the command: to the newest snapshot at or
    // before frame 150, the one of frame 100. The shift is undone.
    let answer = admin_ok(&running.base, &["rollback", "150"]).await;
    assert_eq!(answer["ok"], true, "{answer}");
    // The journal takes a snapshot every 100 frames, or some frames
    // later when the disk was slow. The shift ended after frame 240.
    let back = answer["frame"].as_u64().expect("a frame");
    assert!((100..=150).contains(&back), "{answer}");
    assert_eq!(answer["retired"], json!([]));
    assert_eq!(client.cash(&key).await, 0);

    // The signal that asks a process to end: a clean stop.
    let (code, printed, log) = running.end_with("-TERM").await;
    assert_eq!(code, Some(0), "{log}");
    assert!(printed.ends_with("Stopping."), "{printed}");
    assert!(log.contains("world recovered from the data directory"), "{log}");
    for secret in [ADMIN_TOKEN, token.as_str(), key.as_str(), idle_key.as_str()] {
        assert!(!log.contains(secret) && !printed.contains(secret), "a secret is shown:\n{log}");
    }

    // After the clean stop the directory holds the world of the
    // rollback.
    let checked = replay(&data).await;
    assert_eq!(
        (checked["consistent"].clone(), checked["frame"].clone()),
        (json!(true), json!(back))
    );
    assert_ne!(checked["state_hash"], json!(state_hash));
    assert_eq!(checked["money"]["minted"], 0, "the shift never happened");
}

#[tokio::test]
async fn the_replay_command_prints_a_report_and_fails_for_a_directory_without_a_world() {
    let dir = TempDir::new();
    let data = dir.path().join("data");
    let config = dir.write("server.toml", "");
    let running = Running::start(&data, &config, 0).await;
    let client = Client::at(&running.base);
    let token = client.sign_up("Ada").await;
    client.resident(&token, "One").await;

    // The server that runs holds the directory.
    let (code, printed, error) =
        run(unsupervised().args(["replay", "--data-dir"]).arg(&data)).await;
    assert_eq!((code, printed.as_str()), (Some(1), ""));
    assert!(error.contains("stop it, or copy the directory first"), "{error}");
    let (code, _, log) = running.end_with("-INT").await;
    assert_eq!(code, Some(0), "{log}");

    let (code, printed, error) =
        run(unsupervised().args(["replay", "--data-dir"]).arg(&data)).await;
    assert_eq!(code, Some(0), "{error}");
    let lines: Vec<&str> = printed.lines().collect();
    assert_eq!(lines[0], format!("World directory: {}", data.join("world").display()));
    assert_eq!(lines[1], "Replay from the snapshot of frame 0: 1 inputs, 0 frames.");
    assert_eq!(lines[2], "The snapshot has the state hash that it names: yes.");
    assert!(lines[3].starts_with("The world is at frame 0. State hash: "), "{printed}");
    assert_eq!(lines[4], "Later snapshots compared: 1. With the same state hash: 1.");
    assert_eq!(lines[5], "A recovery from the newest snapshot gives the same state: yes.");
    assert_eq!(lines[6], "Residents: 1.");
    assert_eq!(
        lines[7],
        "Money in cents: 0 held, 0 created, 0 destroyed, 0 between realms. It adds up."
    );
    assert_eq!(lines[8..], ["Result: consistent."]);
    // The other places to start from.
    for from in ["newest", "0"] {
        let mut command = unsupervised();
        command.args(["replay", "--json", "--from", from, "--data-dir"]).arg(&data);
        let (code, printed, error) = run(&mut command).await;
        assert_eq!(code, Some(0), "{from}: {error}");
        let checked: Value = serde_json::from_str(&printed).expect("JSON");
        assert_eq!(checked["consistent"], true, "{from}");
    }
    let mut command = unsupervised();
    command.args(["replay", "--from", "yesterday", "--data-dir"]).arg(&data);
    let (code, _, error) = run(&mut command).await;
    assert_eq!(code, Some(2), "{error}");
    assert!(error.contains("give `oldest`, `newest` or a frame number"), "{error}");

    // A directory without a world, and a server that keeps no world.
    let empty = dir.path().join("empty");
    let (code, _, error) = run(unsupervised().args(["replay", "--data-dir"]).arg(&empty)).await;
    assert_eq!(code, Some(1));
    assert!(error.contains("does not exist"), "{error}");
}
