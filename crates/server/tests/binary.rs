//! The `unsupervised` command: `export-schema`, and `run` with a file,
//! the environment, flags and the signals that stop it.

mod common;

use std::process::Stdio;
use std::time::Duration;

use common::{Client, TempDir, send};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStdout, Command};
use tokio::task::JoinHandle;

/// Time after which a command that says nothing counts as broken.
const PATIENCE: Duration = Duration::from_secs(60);

/// The environment variables that `run` reads.
const VARIABLES: [&str; 12] = [
    "UNSUPERVISED_CONFIG",
    "UNSUPERVISED_BIND",
    "UNSUPERVISED_TICK_MS",
    "UNSUPERVISED_SEED",
    "UNSUPERVISED_CITIES",
    "UNSUPERVISED_DISTRICTS",
    "UNSUPERVISED_WEB_DIR",
    "UNSUPERVISED_DATA_DIR",
    "UNSUPERVISED_STORE",
    "UNSUPERVISED_JOIN_FILE",
    "UNSUPERVISED_ADMIN_TOKEN",
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

#[tokio::test]
async fn export_schema_writes_the_schema_and_the_catalogue() {
    let dir = TempDir::new();
    let out = dir.path().join("made").join("schema");
    let output = unsupervised()
        .arg("export-schema")
        .arg("--out")
        .arg(&out)
        .output()
        .await
        .expect("the command runs");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let printed = String::from_utf8_lossy(&output.stdout);
    let schema_file = out.join("protocol.schema.json");
    let catalogue_file = out.join("catalogue.json");
    assert_eq!(
        printed.lines().collect::<Vec<_>>(),
        [format!("wrote {}", schema_file.display()), format!("wrote {}", catalogue_file.display())]
    );

    let read = |path: &std::path::Path| -> Value {
        let text = std::fs::read_to_string(path).expect("the file exists");
        assert!(text.ends_with("}\n"), "the file ends with a line break");
        serde_json::from_str(&text).expect("the file is JSON")
    };
    // The schema has the wire types that a client generator reads.
    let schema = read(&schema_file);
    for name in ["ActionRequest", "ActionAck", "Observation", "Notice", "Routine", "Catalogue"] {
        assert!(schema["$defs"][name].is_object(), "the schema has no type {name}");
    }
    // The catalogue has the verbs of the kernel and of the game modules,
    // each with a schema for its arguments.
    let catalogue = read(&catalogue_file);
    let verbs = catalogue["verbs"].as_array().expect("verbs");
    for (verb, module) in [("wait", "kernel"), ("set_routines", "kernel"), ("move_to", "life")] {
        let entry = verbs
            .iter()
            .find(|entry| entry["verb"] == verb)
            .unwrap_or_else(|| panic!("the catalogue has no verb {verb}"));
        assert_eq!(entry["module"], module);
        assert!(entry["args_schema"].is_object(), "{entry}");
    }
}

/// A running `unsupervised run`.
struct Running {
    child: Child,
    stdout: Lines<BufReader<ChildStdout>>,
    /// Reads the log until the command ends.
    log: JoinHandle<String>,
    /// The lines that the command printed so far.
    printed: Vec<String>,
}

impl Running {
    fn start(mut command: Command) -> Running {
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("the command starts");
        let stdout = BufReader::new(child.stdout.take().expect("the output stream")).lines();
        let mut stderr = child.stderr.take().expect("the error stream");
        // Both streams are read at the same time, so that neither fills.
        let log = tokio::spawn(async move {
            let mut text = String::new();
            stderr.read_to_string(&mut text).await.expect("the log is text");
            text
        });
        Running { child, stdout, log, printed: Vec::new() }
    }

    /// Reads printed lines until one satisfies `last`.
    async fn read_until(&mut self, last: impl Fn(&str) -> bool) {
        loop {
            let line = tokio::time::timeout(PATIENCE, self.stdout.next_line())
                .await
                .expect("the command prints in time")
                .expect("the output is text")
                .unwrap_or_else(|| {
                    panic!("the command ended early; it printed {:?}", self.printed)
                });
            let done = last(&line);
            self.printed.push(line);
            if done {
                return;
            }
        }
    }

    /// Returns the rest of the first printed line that starts with
    /// `prefix`, without the spaces around it.
    fn printed_after(&self, prefix: &str) -> String {
        self.printed
            .iter()
            .find_map(|line| line.trim().strip_prefix(prefix))
            .unwrap_or_else(|| panic!("no line starts with {prefix:?} in {:?}", self.printed))
            .trim()
            .to_string()
    }

    /// Sends a signal, waits for the end and returns the exit code, the
    /// lines that the command printed after the signal, and the log.
    #[cfg(unix)]
    async fn stop_with(mut self, signal: &str) -> (Option<i32>, Vec<String>, String) {
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
        let mut after = Vec::new();
        while let Some(line) = self.stdout.next_line().await.expect("the output is text") {
            after.push(line);
        }
        (status.code(), after, self.log.await.expect("the log was read"))
    }
}

#[cfg(unix)]
#[tokio::test]
async fn the_development_mode_prints_access_that_works_and_ctrl_c_stops_it() {
    let mut command = unsupervised();
    command.args(["run", "--dev", "--bind", "127.0.0.1:0", "--cities", "1", "--districts", "1"]);
    let mut running = Running::start(command);
    // The last line of the announcement is the end of an example command.
    running.read_until(|line| line.ends_with("-d '{\"name\":\"Ada\"}'")).await;
    let base = running.printed_after("Unsupervised listens on");
    let owner_token = running.printed_after("owner token:");
    let admin_token = running.printed_after("admin token:");
    assert!(running.printed.contains(&"Accounts are in memory. They are lost at the stop.".into()));
    let client = Client::at(&base);

    // The printed tokens work.
    let (resident, key) = client.resident(&owner_token, "Ada").await;
    let (status, before) = send(client.get("/v1/observe").bearer_auth(&key)).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let x = before["sections"]["self"]["pos"]["x"].as_i64().expect("x");
    let y = before["sections"]["self"]["pos"]["y"].as_i64().expect("y");
    let walk = json!({ "verb": "move_to", "args": { "x": x + 1, "y": y } });
    let (status, answer) = send(client.post("/v1/act", &walk).bearer_auth(&key)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"][0]["status"]["state"], "started");
    // The ticks of the development mode are fast: the report comes soon.
    let wait = client.get("/v1/wait?timeout=30&since=1&kinds=report").bearer_auth(&key);
    let (status, news) = send(wait).await;
    assert_eq!(status, StatusCode::OK, "{news}");
    assert_eq!(news["sections"]["reports"][0]["result"], json!({ "x": x + 1, "y": y }));
    let (status, stats) = send(client.get("/admin/stats").bearer_auth(&admin_token)).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["sim"]["residents"], 1);

    let (code, after, log) = running.stop_with("-INT").await;
    assert_eq!(code, Some(0), "the command ends without an error; its log:\n{log}");
    assert_eq!(after, ["Stopping."]);
    // The log tells what happened and holds no token and no key.
    assert!(log.contains("server started"), "{log}");
    assert!(log.contains(&format!("resident created owner=1 resident={resident}")), "{log}");
    for secret in [&owner_token, &admin_token, &key] {
        assert!(!log.contains(secret.as_str()), "the log holds a secret:\n{log}");
        // Not even the random part alone.
        assert!(!log.contains(&secret["unsup_xxx_".len()..]), "the log holds a secret:\n{log}");
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_file_the_environment_and_flags_configure_the_server() {
    let dir = TempDir::new();
    let file = dir.write(
        "server.toml",
        "store = \"memory\"\n\n[world]\ncities = 2\n\n[gateway.limits]\nresidents_per_owner = 1\n",
    );
    let data = dir.path().join("data");
    let admin_token = "an-admin-token-from-the-environment";
    let mut command = unsupervised();
    command
        // The file is named in the environment.
        .env("UNSUPERVISED_CONFIG", &file)
        .env("UNSUPERVISED_BIND", "127.0.0.1:0")
        .env("UNSUPERVISED_ADMIN_TOKEN", admin_token)
        .env("UNSUPERVISED_TICK_MS", "0")
        // The environment replaces the file: `sqlite`, not `memory`.
        .env("UNSUPERVISED_STORE", "sqlite")
        .env("UNSUPERVISED_DATA_DIR", &data)
        // A flag replaces the environment and the file: 1 city, not 3 or 2.
        .env("UNSUPERVISED_CITIES", "3")
        .args(["run", "--cities", "1", "--districts", "1"]);
    let mut running = Running::start(command);
    running.read_until(|line| line.starts_with("Accounts are in")).await;
    let base = running.printed_after("Unsupervised listens on");
    let store_file = data.join("store.sqlite3");
    assert_eq!(running.printed_after("Accounts are in"), store_file.display().to_string());
    assert!(store_file.is_file(), "the server made the store file in the data directory");
    let client = Client::at(&base);

    let (status, world) = send(client.get("/v1/world")).await;
    assert_eq!(status, StatusCode::OK, "{world}");
    assert_eq!(world["cities"].as_array().map(Vec::len), Some(1));
    let (status, stats) = send(client.get("/admin/stats").bearer_auth(admin_token)).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    // A tick time of zero: the world does not move by itself.
    assert_eq!(stats["sim"]["frame"], 0);
    let token = client.sign_up("Ada").await;
    let (status, listed) = send(client.get("/v1/residents").bearer_auth(&token)).await;
    assert_eq!((status, listed["limit"].clone()), (StatusCode::OK, json!(1)));

    // Without the development mode the command makes no owner and prints
    // no token: the owner that signed up is the first one.
    let (status, listed) = send(client.get("/admin/stats").bearer_auth(admin_token)).await;
    assert_eq!((status, listed["store"]["owners"].clone()), (StatusCode::OK, json!(1)));
    let printed = running.printed.join("\n");
    assert_eq!(running.printed.len(), 2, "{printed}");

    // The signal that asks a process to end stops the server too.
    let (code, after, log) = running.stop_with("-TERM").await;
    assert_eq!(code, Some(0), "the command ends without an error; its log:\n{log}");
    assert_eq!(after, ["Stopping."]);
    for text in [&printed, &log] {
        assert!(!text.contains("unsup_own_") && !text.contains(admin_token), "{text}");
    }
}

#[tokio::test]
async fn a_configuration_that_is_not_valid_ends_the_command_with_an_error() {
    let dir = TempDir::new();
    let file = dir.write("server.toml", "[gateway.limits]\nmax_plans = 3\n");
    let output = unsupervised()
        .args(["run", "--bind", "127.0.0.1:0", "--config"])
        .arg(&file)
        .output()
        .await
        .expect("the command runs");
    assert_eq!(output.status.code(), Some(1));
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("unknown field `max_plans`"), "{error}");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "");
}
