//! A gateway for tests that has the routes of `docs/design/protocol.md`
//! which the development server does not have yet: the sign-up with owners
//! and keys, the long poll and the WebSocket for agents.
//!
//! It is a thin layer over a real simulation: the world comes from
//! `unsup_server::build_world` and runs in the runtime of the project. The
//! swarm tests use it to prove the client side of these routes. When the
//! real gateway has them, the test against `--spawn-server` covers them
//! too.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use unsup_loadtest::local::{Shape, build_world};
use unsup_protocol::{
    ActionRequest, ObserveOpts, OwnerId, ResidentId, SimError, SpawnSpec, ViewQuery, WaitFilter,
};
use unsup_runtime::{RuntimeConfig, RuntimeJoin, SimHandle};

/// What the gateway counted.
#[derive(Default)]
pub struct Counters {
    pub owners: AtomicU64,
    pub residents: AtomicU64,
    pub long_polls: AtomicU64,
    pub sockets: AtomicU64,
    /// True: `POST /v1/act` answers `503 overloaded`.
    pub refuse_acts: AtomicBool,
}

#[derive(Clone)]
struct Gate {
    sim: SimHandle,
    counters: Arc<Counters>,
}

/// A running test gateway.
pub struct DocumentedServer {
    pub addr: SocketAddr,
    pub counters: Arc<Counters>,
    _runtime: RuntimeJoin,
    task: JoinHandle<()>,
}

impl DocumentedServer {
    /// Starts a world of one zone that steps one frame every `tick`, and
    /// the gateway on a free port.
    pub async fn start(tick: Duration) -> DocumentedServer {
        let world = build_world(Shape { cities: 1, districts: 1 }.to_config(2), &[])
            .expect("a valid world");
        let (sim, runtime) = unsup_runtime::start(world, RuntimeConfig::real_time(tick));
        let counters = Arc::new(Counters::default());
        let gate = Gate { sim, counters: Arc::clone(&counters) };
        let app = Router::new()
            .route("/v1/catalogue", get(catalogue))
            .route("/v1/world", get(world_layout))
            .route("/v1/owners", post(owners))
            .route("/v1/residents", post(residents))
            .route("/v1/act", post(act))
            .route("/v1/observe", get(observe))
            .route("/v1/wait", get(wait))
            .route("/v1/ws", get(ws))
            .route("/v1/stats", get(stats))
            .with_state(gate);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the gateway serves");
        });
        DocumentedServer { addr, counters, _runtime: runtime, task }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for DocumentedServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct ApiError(StatusCode, &'static str);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": { "code": self.1, "message": self.1 } }))).into_response()
    }
}

impl From<SimError> for ApiError {
    fn from(error: SimError) -> ApiError {
        match error {
            SimError::UnknownResident => ApiError(StatusCode::NOT_FOUND, "unknown_resident"),
            SimError::Overloaded => ApiError(StatusCode::SERVICE_UNAVAILABLE, "overloaded"),
            _ => ApiError(StatusCode::BAD_REQUEST, "invalid"),
        }
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::AUTHORIZATION)?.to_str().ok()?.strip_prefix("Bearer ")
}

/// Reads the resident from its key: `key-<id>`. The development server
/// takes the bare id; this gateway does not.
fn resident(headers: &HeaderMap) -> Result<ResidentId, ApiError> {
    bearer(headers)
        .and_then(|token| token.strip_prefix("key-"))
        .and_then(|id| id.parse().ok())
        .map(ResidentId)
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))
}

async fn catalogue(State(gate): State<Gate>) -> Json<Value> {
    Json(json!(*gate.sim.catalogue()))
}

async fn world_layout(State(gate): State<Gate>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(gate.sim.view(ViewQuery::Layout).await?)))
}

async fn owners(State(gate): State<Gate>) -> Json<Value> {
    let id = gate.counters.owners.fetch_add(1, Ordering::Relaxed) + 1;
    Json(json!({ "owner": { "id": id }, "token": format!("owner-{id}") }))
}

#[derive(Deserialize)]
struct NewResident {
    name: String,
    #[serde(default)]
    model: Option<String>,
}

async fn residents(
    State(gate): State<Gate>,
    headers: HeaderMap,
    Json(body): Json<NewResident>,
) -> Result<Json<Value>, ApiError> {
    let owner: u64 = bearer(&headers)
        .and_then(|token| token.strip_prefix("owner-"))
        .and_then(|id| id.parse().ok())
        .ok_or(ApiError(StatusCode::UNAUTHORIZED, "unauthorized"))?;
    let spec =
        SpawnSpec { owner: OwnerId(owner), name: body.name.clone(), model: body.model, city: None };
    let id = gate.sim.spawn(spec).await?;
    gate.counters.residents.fetch_add(1, Ordering::Relaxed);
    Ok(Json(json!({ "resident": { "id": id, "name": body.name }, "key": format!("key-{id}") })))
}

async fn act(
    State(gate): State<Gate>,
    headers: HeaderMap,
    Json(request): Json<ActionRequest>,
) -> Result<Json<Value>, ApiError> {
    let who = resident(&headers)?;
    if gate.counters.refuse_acts.load(Ordering::Relaxed) {
        return Err(ApiError(StatusCode::SERVICE_UNAVAILABLE, "overloaded"));
    }
    let ack = gate.sim.act(who, request).await?;
    let observation = gate.sim.observe(who, ObserveOpts::default()).await?;
    Ok(Json(json!({ "acks": [ack], "observation": observation })))
}

#[derive(Deserialize)]
struct Since {
    #[serde(default)]
    since: u64,
    #[serde(default)]
    timeout: u64,
}

async fn observe(
    State(gate): State<Gate>,
    headers: HeaderMap,
    Query(query): Query<Since>,
) -> Result<Json<Value>, ApiError> {
    let who = resident(&headers)?;
    let opts = ObserveOpts { since_notice: query.since, ..ObserveOpts::default() };
    Ok(Json(json!(gate.sim.observe(who, opts).await?)))
}

async fn wait(
    State(gate): State<Gate>,
    headers: HeaderMap,
    Query(query): Query<Since>,
) -> Result<Json<Value>, ApiError> {
    let who = resident(&headers)?;
    gate.counters.long_polls.fetch_add(1, Ordering::Relaxed);
    let timeout = Duration::from_secs(query.timeout.min(60));
    let observation =
        gate.sim.wait_since(who, WaitFilter::ActionDone, Some(query.since), timeout).await?;
    Ok(Json(json!(observation)))
}

async fn stats(State(gate): State<Gate>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!(gate.sim.stats().await?)))
}

async fn ws(
    State(gate): State<Gate>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let who = resident(&headers)?;
    // A key of a resident that does not exist is refused in the handshake.
    gate.sim.observe(who, ObserveOpts::default()).await?;
    gate.counters.sockets.fetch_add(1, Ordering::Relaxed);
    Ok(upgrade.on_upgrade(move |socket| session(socket, gate.sim, who)))
}

fn error_frame(error: SimError) -> Value {
    json!({ "op": "error", "error": { "code": error.as_str(), "message": error.to_string() } })
}

/// Answers the frames of one agent in JSON text, and pushes the reports of
/// ended actions as MessagePack binary frames.
async fn session(mut socket: WebSocket, sim: SimHandle, who: ResidentId) {
    let mut cursor = match sim.observe(who, ObserveOpts::default()).await {
        Ok(observation) => observation.next_notice,
        Err(_) => return,
    };
    loop {
        tokio::select! {
            message = socket.recv() => {
                let Some(Ok(message)) = message else { return };
                let Message::Text(text) = message else { continue };
                let frame: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                let reply = match frame["op"].as_str() {
                    Some("act") => match serde_json::from_value::<ActionRequest>(frame["request"].clone()) {
                        Ok(request) => match sim.act(who, request).await {
                            Ok(ack) => json!({ "op": "ack", "ack": ack }),
                            Err(error) => error_frame(error),
                        },
                        Err(_) => error_frame(SimError::Invalid),
                    },
                    Some("observe") => {
                        let opts = serde_json::from_value(frame["opts"].clone()).unwrap_or_default();
                        match sim.observe(who, opts).await {
                            Ok(observation) => json!({ "op": "observation", "observation": observation }),
                            Err(error) => error_frame(error),
                        }
                    }
                    Some("ping") => json!({ "op": "pong" }),
                    _ => error_frame(SimError::Invalid),
                };
                if socket.send(Message::Text(reply.to_string().into())).await.is_err() {
                    return;
                }
            }
            news = sim.wait_since(who, WaitFilter::ActionDone, Some(cursor), Duration::from_secs(30)) => {
                let Ok(observation) = news else { return };
                cursor = observation.next_notice;
                let reports = observation.sections.get("reports").and_then(Value::as_array);
                for report in reports.into_iter().flatten() {
                    let frame = json!({ "op": "report", "report": report });
                    let bytes = unsup_protocol::codec::to_msgpack(&frame).expect("a frame encodes");
                    if socket.send(Message::Binary(bytes.into())).await.is_err() {
                        return;
                    }
                }
            }
        }
    }
}
