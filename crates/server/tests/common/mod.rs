//! Helpers for the server tests.

#![allow(dead_code, reason = "each test file uses a part of the helpers")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::{RequestBuilder, StatusCode};
use serde_json::{Value, json};
use unsup_server::{Server, ServerConfig, StoreConfig, WorldShape};
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

/// The configuration of the tests: a free port, a small world that moves
/// only when the test steps it, accounts in memory, no viewer files.
pub fn config() -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1:0".parse().expect("an address"),
        tick_interval: Duration::ZERO,
        world: WorldShape { seed: 7, cities: 1, districts: 1 },
        web_dir: None,
        store: StoreConfig::Memory,
        ..ServerConfig::default()
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
