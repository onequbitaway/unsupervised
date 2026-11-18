//! Helpers for the server tests.

#![allow(dead_code, reason = "each test file uses a part of the helpers")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode};
use serde_json::{Value, json};
use unsup_server::{Server, ServerConfig, StoreConfig, WorldShape, WorldStore};
use unsup_store::random_hex;

/// A directory in the temporary directory of the system. It is removed
/// when the value is dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> TempDir {
        let name = format!("unsup-server-test-{}", random_hex(8).expect("random bytes"));
        let path = std::env::temp_dir().join(name);
        std::fs::create_dir(&path).expect("a new directory");
        TempDir(path)
    }

    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes a file in the directory and returns its path.
    pub fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).expect("the file is written");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Frames between two snapshots in the servers of the tests.
pub const SNAPSHOT_EVERY: u64 = 50;

/// The configuration of the tests: a free port, a small world that moves
/// only when the test steps it, accounts and the world in memory, no
/// viewer files, and a snapshot every [`SNAPSHOT_EVERY`] frames. It
/// keeps 20 snapshots, more than a test makes.
pub fn config() -> ServerConfig {
    let mut config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("an address"),
        tick_interval: Duration::ZERO,
        world: WorldShape { seed: 7, cities: 1, districts: 1 },
        web_dir: None,
        store: StoreConfig::Memory,
        world_store: WorldStore::Memory,
        ..ServerConfig::default()
    };
    config.durability.snapshot_every = Some(SNAPSHOT_EVERY);
    config.durability.keep_snapshots = Some(20);
    config
}

/// Sends a request and returns the status and the JSON body. A body that
/// is empty reads as `null`.
pub async fn send(request: RequestBuilder) -> (StatusCode, Value) {
    let response = request.send().await.expect("the server answers");
    let status = response.status();
    let bytes = response.bytes().await.expect("the body arrives");
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| panic!("a JSON body, not {:?}", String::from_utf8_lossy(&bytes)))
    };
    (status, body)
}

/// A client of one server.
pub struct Client {
    pub base: String,
    pub http: reqwest::Client,
}

impl Client {
    pub fn of(server: &Server) -> Client {
        Client::at(&format!("http://{}", server.addr))
    }

    pub fn at(base: &str) -> Client {
        Client { base: base.to_string(), http: reqwest::Client::new() }
    }

    pub fn get(&self, path: &str) -> RequestBuilder {
        self.http.get(format!("{}{path}", self.base))
    }

    pub fn post(&self, path: &str, body: &Value) -> RequestBuilder {
        self.http.post(format!("{}{path}", self.base)).json(body)
    }

    /// Signs up an owner and returns its token.
    pub async fn sign_up(&self, name: &str) -> String {
        let (status, body) = send(self.post("/v1/owners", &json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED, "sign-up: {body}");
        body["token"].as_str().expect("an owner token").to_string()
    }

    /// Makes a resident and returns its id and its key.
    pub async fn resident(&self, owner_token: &str, name: &str) -> (u64, String) {
        let request = self.post("/v1/residents", &json!({ "name": name })).bearer_auth(owner_token);
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::CREATED, "create resident: {body}");
        let key = body["key"].as_str().expect("a resident key").to_string();
        (body["resident"]["id"].as_u64().expect("a resident id"), key)
    }
}

/// The admin token of the tests that need one.
pub const ADMIN: &str = "admin-token-of-the-server-tests";

/// A tile inside the depot of the test world. A resident without a job
/// can work a starter shift there.
pub const DEPOT: (i64, i64) = (24, 4);
/// What one starter shift pays after the income tax, in cents.
pub const SHIFT_PAY: i64 = 2_520;
/// The wage of one starter shift before the income tax, in cents. The
/// world makes this money when it pays the shift.
pub const SHIFT_WAGE: i64 = 2_800;
/// Frames that are enough to walk to the depot from the place where a
/// resident appears, and to work one shift there.
const WALK_FRAMES: u32 = 60;
const SHIFT_FRAMES: u32 = 245;

/// The configuration of a server that keeps accounts and the world in
/// `dir`, with an admin token.
pub fn durable(dir: &Path) -> ServerConfig {
    let mut config = ServerConfig {
        store: StoreConfig::Sqlite(dir.join("store.sqlite3")),
        world_store: WorldStore::Dir(dir.join("world")),
        ..config()
    };
    config.gateway.admin_token = Some(unsup_gateway::AdminToken::new(ADMIN).expect("a long token"));
    config
}

/// Copies a directory with everything in it.
pub fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("the target directory");
    for entry in std::fs::read_dir(from).expect("the source directory") {
        let entry = entry.expect("an entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("the file is copied");
        }
    }
}

impl Client {
    /// Sends one action of a resident and returns the answer.
    pub async fn act(&self, key: &str, verb: &str, args: Value) -> Value {
        let request = self.post("/v1/act", &json!({ "verb": verb, "args": args })).bearer_auth(key);
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::OK, "{verb}: {body}");
        body
    }

    /// Sends one action that the simulation must accept. Returns the
    /// state of its acknowledgement.
    pub async fn accepted(&self, key: &str, verb: &str, args: Value) -> String {
        let body = self.act(key, verb, args).await;
        let state = body["acks"][0]["status"]["state"].as_str().expect("a state").to_string();
        assert_ne!(state, "rejected", "{verb}: {body}");
        state
    }

    /// Returns the `self` section of a resident.
    pub async fn myself(&self, key: &str) -> Value {
        let (status, seen) = send(self.get("/v1/observe?sections=self").bearer_auth(key)).await;
        assert_eq!(status, StatusCode::OK, "{seen}");
        seen["sections"]["self"].clone()
    }

    /// Returns the cash of a resident, in cents.
    pub async fn cash(&self, key: &str) -> i64 {
        self.myself(key).await["cash"].as_i64().expect("cash")
    }

    /// Sends a request with the admin token of the tests.
    pub async fn admin(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let request = self.http.request(method, format!("{}{path}", self.base)).bearer_auth(ADMIN);
        send(match body {
            Some(body) => request.json(&body),
            None => request,
        })
        .await
    }

    /// Returns the admin dashboard.
    pub async fn dashboard(&self) -> Value {
        let (status, dashboard) = self.admin(reqwest::Method::GET, "/admin/dashboard", None).await;
        assert_eq!(status, StatusCode::OK, "{dashboard}");
        dashboard
    }
}

/// Waits until the snapshot of `frame` is on the disk. Another thread
/// writes it.
pub async fn snapshot_written(server: &Server, frame: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while server.sim.stats().await.expect("stats").last_snapshot_frame != frame {
        assert!(std::time::Instant::now() < deadline, "the snapshot of frame {frame} is late");
        tokio::task::yield_now().await;
    }
}

/// Runs frames of the world. At each frame that has a snapshot it waits
/// until the snapshot is written: the journal takes no snapshot while
/// the one before is on its way, and the tests name the frames of the
/// snapshots. Returns the number of the last frame.
pub async fn advance(server: &Server, frames: u32) -> u64 {
    let mut frame = server.sim.stats().await.expect("stats").frame;
    let mut left = u64::from(frames);
    while left > 0 {
        let chunk = left.min(SNAPSHOT_EVERY - frame % SNAPSHOT_EVERY);
        let chunk_frames = u32::try_from(chunk).expect("a small number");
        frame = server.sim.step(chunk_frames).await.expect("the world steps");
        left -= chunk;
        if frame.is_multiple_of(SNAPSHOT_EVERY) {
            snapshot_written(server, frame).await;
        }
    }
    frame
}

/// Makes residents walk to the depot and work one starter shift there.
/// The test steps the world. Each resident has [`SHIFT_PAY`] more cents
/// at the end.
pub async fn earn(server: &Server, client: &Client, keys: &[&str]) {
    for key in keys {
        let walk = json!({ "x": DEPOT.0, "y": DEPOT.1 });
        assert_eq!(client.accepted(key, "move_to", walk).await, "started");
    }
    advance(server, WALK_FRAMES).await;
    let mut before = Vec::new();
    for key in keys {
        before.push(client.cash(key).await);
        assert_eq!(client.accepted(key, "work", json!({})).await, "started");
    }
    advance(server, SHIFT_FRAMES).await;
    for (key, before) in keys.iter().zip(before) {
        assert_eq!(client.cash(key).await, before + SHIFT_PAY, "the shift paid");
    }
}
