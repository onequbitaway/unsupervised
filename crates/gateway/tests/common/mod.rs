//! A gateway on a real port, in front of a small world that the test
//! steps by hand.

#![allow(dead_code, reason = "each test file uses a part of the harness")]

pub mod github;
pub mod market;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{Method, RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::task::JoinHandle;
use unsup_gateway::{Gateway, GatewayConfig};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionError, Begin, Builder, Plugin, ResidentId, VerbSpec, World, WorldConfig};
use unsup_protocol::Untrusted;
use unsup_runtime::{RuntimeConfig, RuntimeJoin, SimHandle};
use unsup_store::{IdentityProvider, LocalSignUp, MemoryStore, Store};

/// The admin token of the test gateway.
pub const ADMIN: &str = "test-admin-token-0123456789";

/// Time after which a test gives up on a thing that the server must do.
/// It is long, so that a slow machine does not fail a good server. It
/// only turns a test that would hang into a test that fails.
pub const PATIENCE: Duration = Duration::from_secs(60);

/// Waits for something that the server must do soon.
///
/// # Panics
///
/// Panics when the time is over.
pub async fn soon<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(PATIENCE, future).await.expect("the server does this in time")
}

/// A limit for a loop that asks the server again and again.
pub struct Deadline(Instant);

impl Deadline {
    pub fn start() -> Deadline {
        Deadline(Instant::now() + PATIENCE)
    }

    /// Lets other tasks run, then returns for the next try.
    ///
    /// # Panics
    ///
    /// Panics when the time is over.
    pub async fn again(&self) {
        assert!(Instant::now() < self.0, "the server did not reach the expected state in time");
        tokio::task::yield_now().await;
    }
}

/// Arguments of `work`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WorkArgs {
    /// Duration in ticks.
    ticks: u32,
}

/// Arguments of `tell`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct TellArgs {
    /// The resident that gets the notice.
    to: ResidentId,
    /// What to tell.
    text: String,
    /// Kind of the notice. `town.told` when absent.
    #[serde(default)]
    kind: Option<String>,
}

/// A small module: `work` keeps the resident busy and reports, `tell`
/// writes a notice with text to another resident.
pub struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        b.verb::<WorkArgs>(
            VerbSpec {
                verb: "work",
                module: "town",
                summary: "Work for a number of ticks.",
                free: false,
                allowed_in: &[],
            },
            |_, _, args| {
                if args.ticks == 0 {
                    return Err(ActionError::invalid_args("`ticks` must be at least 1."));
                }
                Ok(Begin::Busy { ticks: args.ticks })
            },
            |_, _, args| Ok(json!({ "worked": args.ticks })),
        );
        b.instant_verb::<TellArgs>(
            VerbSpec {
                verb: "tell",
                module: "town",
                summary: "Tell a resident something.",
                free: true,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                if realm.idx_of(args.to).is_none() {
                    return Err(ActionError::not_found("Nobody with this id is here."));
                }
                let kind = args.kind.as_deref().unwrap_or("town.told");
                let text = Untrusted::new(ctx.resident, args.text.clone()).to_value();
                realm.notify(args.to, kind, Some(ctx.resident), text);
                Ok(Value::Null)
            },
        );
    }
}

/// Starts the small world. Its frames advance only by `SimHandle::step`.
pub fn world() -> (SimHandle, RuntimeJoin) {
    let world = World::new(&[&Town], WorldConfig::with_seed(3));
    unsup_runtime::start(world, RuntimeConfig::manual())
}

/// A running test gateway.
pub struct Harness {
    /// `http://127.0.0.1:<port>`.
    pub base: String,
    pub addr: SocketAddr,
    pub sim: SimHandle,
    pub gateway: Gateway,
    pub store: Arc<MemoryStore>,
    pub http: reqwest::Client,
    /// Dropping it stops the world. `None` after [`Harness::into_parts`].
    runtime: std::sync::Mutex<Option<RuntimeJoin>>,
    server: JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.gateway.shutdown();
        self.server.abort();
    }
}

/// The configuration of the tests: the defaults, the admin token, and
/// the TOML in `overrides`.
pub fn config(overrides: &str) -> GatewayConfig {
    let text = format!("admin_token = \"{ADMIN}\"\n{overrides}");
    GatewayConfig::from_toml(&text).expect("a valid test configuration")
}

impl Harness {
    /// Starts a gateway with the defaults and the admin token.
    pub async fn start() -> Harness {
        Harness::with(config("")).await
    }

    /// Starts a gateway with the defaults and the TOML in `overrides`.
    pub async fn with_toml(overrides: &str) -> Harness {
        Harness::with(config(overrides)).await
    }

    pub async fn with(config: GatewayConfig) -> Harness {
        Harness::with_providers(config, vec![Arc::new(LocalSignUp)]).await
    }

    pub async fn with_providers(
        config: GatewayConfig,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Harness {
        let (sim, runtime) = world();
        Harness::serve(sim, runtime, Arc::new(MemoryStore::new()), config, providers).await
    }

    /// Starts a gateway in front of a world and a store that exist
    /// already, as after a restart of the server.
    pub async fn serve(
        sim: SimHandle,
        runtime: RuntimeJoin,
        store: Arc<MemoryStore>,
        config: GatewayConfig,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Harness {
        let shared: Arc<dyn Store> = store.clone();
        let gateway = Gateway::start_with_providers(sim.clone(), shared, config, providers)
            .await
            .expect("the gateway starts");
        Harness::around(sim, runtime, store, gateway).await
    }

    /// Starts a gateway with the small world on a port that the test
    /// chose, for a configuration that names the address of the server.
    pub async fn on_port(config: GatewayConfig, port: u16) -> Harness {
        let (sim, runtime) = world();
        let store = Arc::new(MemoryStore::new());
        let shared: Arc<dyn Store> = store.clone();
        let gateway =
            Gateway::start(sim.clone(), shared, config).await.expect("the gateway starts");
        Harness::listen(sim, runtime, store, gateway, port).await
    }

    /// Serves a gateway that the test started itself, on a free port.
    pub async fn around(
        sim: SimHandle,
        runtime: RuntimeJoin,
        store: Arc<MemoryStore>,
        gateway: Gateway,
    ) -> Harness {
        Harness::listen(sim, runtime, store, gateway, 0).await
    }

    async fn listen(
        sim: SimHandle,
        runtime: RuntimeJoin,
        store: Arc<MemoryStore>,
        gateway: Gateway,
        port: u16,
    ) -> Harness {
        let app = gateway.router().layer(gateway.cors());
        let listener =
            tokio::net::TcpListener::bind(("127.0.0.1", port)).await.expect("a free local port");
        let addr = listener.local_addr().expect("the address of the listener");
        let server = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .expect("the test server runs");
        });
        Harness {
            base: format!("http://{addr}"),
            addr,
            sim,
            gateway,
            store,
            // No call of a test is longer than a `wait` of 60 seconds.
            http: reqwest::Client::builder().timeout(PATIENCE * 2).build().expect("an HTTP client"),
            runtime: std::sync::Mutex::new(Some(runtime)),
            server,
        }
    }

    /// Stops the gateway and gives back the world and the store, for a
    /// test that starts a second gateway on them: a restart of the
    /// server with the same data.
    pub fn into_parts(self) -> (SimHandle, RuntimeJoin, Arc<MemoryStore>) {
        let (sim, runtime) = self.take_runtime();
        (sim, runtime, Arc::clone(&self.store))
    }

    /// Takes the world out of a harness that stays in use, for a test
    /// that stops the world while the gateway runs.
    pub fn take_runtime(&self) -> (SimHandle, RuntimeJoin) {
        let runtime = self.runtime.lock().expect("the lock").take();
        (self.sim.clone(), runtime.expect("the harness owns the runtime"))
    }

    pub fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    /// Builds a request with a bearer token.
    pub fn request(&self, method: Method, path: &str, token: &str) -> RequestBuilder {
        self.http.request(method, self.url(path)).bearer_auth(token)
    }

    pub fn get(&self, path: &str, token: &str) -> RequestBuilder {
        self.request(Method::GET, path, token)
    }

    pub fn post(&self, path: &str, token: &str, body: &Value) -> RequestBuilder {
        self.request(Method::POST, path, token).json(body)
    }

    pub fn put(&self, path: &str, token: &str, body: &Value) -> RequestBuilder {
        self.request(Method::PUT, path, token).json(body)
    }

    /// Signs up an owner and returns its id and its token.
    pub async fn owner(&self, name: &str) -> (u64, String) {
        let request = self.http.post(self.url("/v1/owners")).json(&json!({ "name": name }));
        let (status, body) = send(request).await;
        assert_eq!(status, StatusCode::CREATED, "sign-up: {body}");
        let token = body["token"].as_str().expect("an owner token").to_string();
        (body["owner"]["id"].as_u64().expect("an owner id"), token)
    }

    /// Makes a resident and returns its id and its key.
    pub async fn resident(&self, owner_token: &str, name: &str) -> (u64, String) {
        let (status, body) =
            send(self.post("/v1/residents", owner_token, &json!({ "name": name }))).await;
        assert_eq!(status, StatusCode::CREATED, "create resident: {body}");
        let key = body["key"].as_str().expect("a resident key").to_string();
        (body["resident"]["id"].as_u64().expect("a resident id"), key)
    }

    /// Signs up an owner with one resident. Returns the resident id and
    /// its key.
    pub async fn agent(&self, name: &str) -> (u64, String) {
        let (_, token) = self.owner(&format!("Owner of {name}")).await;
        self.resident(&token, name).await
    }

    /// Runs frames of the world.
    pub async fn step(&self, frames: u32) {
        self.sim.step(frames).await.expect("the world steps");
    }

    /// Makes `from` tell `to` a text. The notice arrives with the next
    /// frame.
    pub async fn tell(&self, from_key: &str, to: u64, text: &str) {
        let action = json!({ "verb": "tell", "args": { "to": to, "text": text } });
        let (status, body) = send(self.post("/v1/act", from_key, &action)).await;
        assert_eq!(status, StatusCode::OK, "tell: {body}");
        assert_eq!(body["acks"][0]["status"]["state"], "done", "tell: {body}");
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

/// The agent WebSocket of one resident, as a client.
pub type AgentSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens the agent WebSocket of a resident. `query` is empty or starts
/// with `?`.
pub async fn agent_socket(h: &Harness, key: &str, query: &str) -> AgentSocket {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request =
        format!("ws://{}/v1/ws{query}", h.addr).into_client_request().expect("a request");
    let value = format!("Bearer {key}").parse().expect("a header value");
    request.headers_mut().insert("Authorization", value);
    tokio_tungstenite::connect_async(request).await.expect("the socket opens").0
}

/// Reads the next text frame of a socket as JSON. Pings are skipped.
///
/// # Panics
///
/// Panics when no text frame comes in time. The pings of the server do
/// not count: one limit covers the whole wait.
pub async fn next_frame(socket: &mut AgentSocket) -> Value {
    use futures::StreamExt;
    use tokio_tungstenite::tungstenite::Message;
    soon(async {
        loop {
            match socket.next().await.expect("the socket is open").expect("a frame") {
                Message::Text(text) => return serde_json::from_str(&text).expect("JSON"),
                Message::Ping(_) | Message::Pong(_) => {}
                other => panic!("a text frame, not {other:?}"),
            }
        }
    })
    .await
}
