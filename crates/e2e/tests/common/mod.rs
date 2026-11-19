//! The harness of the end-to-end tests: the real server on a free port.
//!
//! A test steps the world through [`World::step`] and [`World::until`].
//! Everything else goes through the network, as for an outside party.

#![allow(dead_code, reason = "each test file uses a part of the harness")]

pub mod agent;
pub mod child;
pub mod clock;
pub mod game;
pub mod hook;
pub mod joindoc;
pub mod link;
pub mod mcp;
pub mod socket;
pub mod story;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::{Method, RequestBuilder, StatusCode};
use serde_json::{Value, json};
use unsup_gateway::AdminToken;
use unsup_server::{Server, ServerConfig, StoreConfig, WorldShape, WorldStore};

/// The admin token of the test servers.
pub const ADMIN: &str = "admin-token-of-the-end-to-end-tests";

/// Time after which a test gives up on a thing that the server must do.
/// It is long, so that a slow machine does not fail a good server. It
/// only turns a test that would hang into a test that fails.
pub const PATIENCE: Duration = Duration::from_secs(120);

/// Greatest number of frames that [`World::until`] steps for one thing.
/// It is more than 20 game days.
const FRAMES_MAX: u32 = 30_000;

/// Waits for something that the server must do soon.
///
/// # Panics
///
/// Panics when the time is over.
pub async fn soon<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(PATIENCE, future).await.expect("the server does this in time")
}

/// Returns the root directory of the repository.
pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().expect("the repository")
}

/// The configuration of the tests: a free port, one small city that moves
/// only when the test steps it, accounts and the world in memory, no
/// viewer files, an admin token, the join document of the repository,
/// and webhooks to local addresses.
///
/// The rate limits are the defaults of the server. A test that sends
/// more requests than a real agent turns them off with [`no_rate_limits`].
pub fn config() -> ServerConfig {
    let mut config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("an address"),
        tick_interval: Duration::ZERO,
        world: WorldShape { seed: 7, cities: 1, districts: 2 },
        web_dir: None,
        store: StoreConfig::Memory,
        world_store: WorldStore::Memory,
        ..ServerConfig::default()
    };
    config.gateway.admin_token = Some(AdminToken::new(ADMIN).expect("a long token"));
    config.gateway.join_file = Some(repo_root().join("docs/join.md"));
    // The webhook receiver of a test listens on a loopback address.
    config.gateway.webhooks.allow_private = true;
    config
}

/// Turns off the limits of requests for each minute. The clients of some
/// tests are faster than the game time permits a real agent to be: the
/// world of a test runs a game day in seconds.
pub fn no_rate_limits(config: &mut ServerConfig) {
    config.gateway.limits.key_requests_per_minute = 0;
    config.gateway.limits.address_requests_per_minute = 0;
    config.gateway.limits.signups_per_hour = 0;
    config.gateway.moderation.texts_per_minute = 0;
}

/// A running server and an HTTP client for it.
pub struct World {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    /// The address that the server listens on.
    pub addr: SocketAddr,
    /// The HTTP client of the test.
    pub http: reqwest::Client,
    server: Server,
}

impl World {
    /// Starts a server with the configuration of the tests.
    pub async fn start() -> World {
        World::with(config()).await
    }

    /// Starts a server with a configuration.
    pub async fn with(config: ServerConfig) -> World {
        let server = unsup_server::start(config).await.expect("the server starts");
        let addr = server.addr;
        // No call of a test is longer than a `wait` of 60 seconds.
        let http = reqwest::Client::builder().timeout(PATIENCE).build().expect("an HTTP client");
        World { base: format!("http://{addr}"), addr, http, server }
    }

    /// Stops the server.
    pub async fn stop(self) {
        self.server.stop().await.expect("the server stops");
    }

    /// Returns the address of a path.
    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Starts a clock service for the child processes of a test. They
    /// run frames of this world through it.
    pub async fn clock(&self) -> clock::Clock {
        clock::Clock::start(self.server.sim.clone()).await
    }

    /// Runs frames of the world. Returns the number of the last frame.
    pub async fn step(&self, frames: u32) -> u64 {
        self.server.sim.step(frames).await.expect("the world steps")
    }

    /// Returns the number of long polls (`GET /v1/wait`) that are open on
    /// the server now. A test that runs the world only while its clients
    /// wait reads it before each frame.
    pub async fn open_waits(&self) -> u64 {
        let (status, stats) = self.get("/admin/stats", ADMIN).await;
        assert_eq!(status, StatusCode::OK, "{stats}");
        id(&stats["gateway"]["open_waits"], "open_waits")
    }

    /// Runs the world, one frame at a time, until `future` is done.
    /// Returns what the future gave. The future is a call that waits for
    /// news of the world: a long poll, a pushed frame, a delivery.
    ///
    /// The world can be some frames past the news when the call returns:
    /// the answer needs time to travel.
    ///
    /// # Panics
    ///
    /// Panics when the future is not done after [`FRAMES_MAX`] frames or
    /// after [`PATIENCE`].
    pub async fn until<F: Future>(&self, future: F) -> F::Output {
        tokio::pin!(future);
        soon(async {
            for _ in 0..FRAMES_MAX {
                tokio::select! {
                    biased;
                    output = &mut future => return output,
                    _ = self.step(1) => {}
                }
            }
            panic!("the world ran {FRAMES_MAX} frames and the awaited news did not come");
        })
        .await
    }

    /// Builds a request without a token.
    pub fn open(&self, method: Method, path: &str) -> RequestBuilder {
        self.http.request(method, self.url(path))
    }

    /// Builds a request with a bearer token.
    pub fn request(&self, method: Method, path: &str, token: &str) -> RequestBuilder {
        self.open(method, path).bearer_auth(token)
    }

    /// Sends a `GET` without a token: a public read.
    pub async fn read(&self, path: &str) -> (StatusCode, Value) {
        send(self.open(Method::GET, path)).await
    }

    /// Sends a public read that must answer 200. Returns the body.
    pub async fn public(&self, path: &str) -> Value {
        let (status, body) = self.read(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}: {body}");
        body
    }

    /// Sends a `GET` with a token.
    pub async fn get(&self, path: &str, token: &str) -> (StatusCode, Value) {
        send(self.request(Method::GET, path, token)).await
    }

    /// Sends a `POST` with a token and a JSON body.
    pub async fn post(&self, path: &str, token: &str, body: &Value) -> (StatusCode, Value) {
        send(self.request(Method::POST, path, token).json(body)).await
    }

    /// Sends a `PUT` with a token and a JSON body.
    pub async fn put(&self, path: &str, token: &str, body: &Value) -> (StatusCode, Value) {
        send(self.request(Method::PUT, path, token).json(body)).await
    }

    /// Sends a `POST` with the admin token. `body` can be `Value::Null`.
    pub async fn admin(&self, path: &str, body: &Value) -> (StatusCode, Value) {
        let request = self.request(Method::POST, path, ADMIN);
        send(if body.is_null() { request } else { request.json(body) }).await
    }

    /// Signs up an owner. Returns its id and its token.
    pub async fn owner(&self, name: &str) -> (u64, String) {
        let request = self.open(Method::POST, "/v1/owners").json(&json!({ "name": name }));
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::CREATED, "sign-up: {body}");
        let token = body["token"].as_str().expect("an owner token").to_string();
        (body["owner"]["id"].as_u64().expect("an owner id"), token)
    }

    /// Makes a resident. Returns its id and its key.
    pub async fn resident(&self, owner_token: &str, name: &str) -> (u64, String) {
        let body = json!({ "name": name, "model": "e2e-test" });
        let (status, body) = self.post("/v1/residents", owner_token, &body).await;
        assert_eq!(status, StatusCode::CREATED, "create resident: {body}");
        let key = body["key"].as_str().expect("a resident key").to_string();
        (body["resident"]["id"].as_u64().expect("a resident id"), key)
    }

    /// Signs up an owner with one resident. Returns the resident id and
    /// its key.
    pub async fn newcomer(&self, name: &str) -> (u64, String) {
        let (_, token) = self.owner(&format!("Owner of {name}")).await;
        self.resident(&token, name).await
    }
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

/// Returns the `code` of an error answer.
pub fn code(body: &Value) -> &str {
    body["error"]["code"].as_str().unwrap_or_else(|| panic!("an error answer, not {body}"))
}

/// Reads a whole number of a JSON value.
///
/// # Panics
///
/// Panics when the value is not a whole number. `what` names it.
pub fn int(value: &Value, what: &str) -> i64 {
    value.as_i64().unwrap_or_else(|| panic!("{what} is a whole number, not {value}"))
}

/// Reads an id of a JSON value.
///
/// # Panics
///
/// Panics when the value is not an id. `what` names it.
pub fn id(value: &Value, what: &str) -> u64 {
    value.as_u64().unwrap_or_else(|| panic!("{what} is an id, not {value}"))
}
