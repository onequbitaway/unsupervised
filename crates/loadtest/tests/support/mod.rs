//! A server for the swarm tests that the test controls.
//!
//! It is the gateway and the viewer routes of the project over a real
//! simulation, like `unsup_server::start` makes them. Two things differ:
//! the test sets the limits of the gateway, and the test can make the
//! server refuse every action with `503 overloaded`, to prove that the
//! swarm counts a failing server.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use axum::Json;
use axum::extract::{Request, State};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use unsup_gateway::{Gateway, GatewayConfig};
use unsup_loadtest::local::{Shape, build_world};
use unsup_runtime::{RuntimeConfig, RuntimeJoin, SimHandle};
use unsup_store::MemoryStore;

/// A running test server.
pub struct TestServer {
    pub addr: SocketAddr,
    /// The simulation behind the server.
    pub sim: SimHandle,
    /// True: `POST /v1/act` answers `503 overloaded`.
    pub refuse_acts: Arc<AtomicBool>,
    gateway: Gateway,
    _runtime: RuntimeJoin,
    task: JoinHandle<()>,
}

/// Turns off every limit of the gateway that would stop a swarm from one
/// address, as the tool does for a server that it starts.
pub fn no_limits(config: &mut GatewayConfig) {
    let limits = &mut config.limits;
    limits.residents_per_owner = 0;
    limits.key_requests_per_minute = 0;
    limits.address_requests_per_minute = 0;
    limits.signups_per_hour = 0;
    limits.waits_per_resident = 0;
    config.ws.connections_per_resident = 0;
}

/// Refuses every action while the switch is on.
async fn refuse(State(on): State<Arc<AtomicBool>>, request: Request, next: Next) -> Response {
    let is_act = request.method() == Method::POST && request.uri().path() == "/v1/act";
    if is_act && on.load(Ordering::Relaxed) {
        let body =
            json!({ "error": { "code": "overloaded", "message": "The test refuses actions." } });
        return (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response();
    }
    next.run(request).await
}

impl TestServer {
    /// Starts a world of one zone that steps one frame every `tick`, and
    /// the gateway of the project on a free port. `configure` changes the
    /// default limits of the gateway.
    pub async fn start(tick: Duration, configure: impl FnOnce(&mut GatewayConfig)) -> TestServer {
        let world = build_world(Shape { cities: 1, districts: 1 }.to_config(2), &[])
            .expect("a valid world");
        let (sim, runtime) = unsup_runtime::start(world, RuntimeConfig::real_time(tick));
        let mut config = GatewayConfig::default();
        configure(&mut config);
        let gateway = Gateway::start(sim.clone(), Arc::new(MemoryStore::new()), config)
            .await
            .expect("the gateway starts");
        let refuse_acts = Arc::new(AtomicBool::new(false));
        let app = gateway
            .router()
            .merge(unsup_spectator::router(sim.clone()))
            .layer(middleware::from_fn_with_state(Arc::clone(&refuse_acts), refuse));
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free port");
        let addr = listener.local_addr().expect("an address");
        let task = tokio::spawn(async move {
            axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
                .await
                .expect("the server serves");
        });
        TestServer { addr, sim, refuse_acts, gateway, _runtime: runtime, task }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // End the long polls and the sockets, then the server task.
        self.gateway.shutdown();
        self.task.abort();
    }
}
