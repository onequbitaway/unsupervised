//! A clock for child processes: a small HTTP service that runs frames of
//! the world of the test. A script in `node` or `python3` cannot reach
//! the handle of the server, so it asks this service. The world of such
//! a test still moves only when somebody asks, and a run is the same
//! each time.

use std::collections::HashMap;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use unsup_runtime::SimHandle;

/// Greatest number of frames of one request.
const FRAMES_MAX: u32 = 20_000;

/// A running clock service.
pub struct Clock {
    /// The address for `E2E_CLOCK_URL`: `POST <url>/step?frames=N`.
    pub url: String,
    task: JoinHandle<()>,
}

impl Drop for Clock {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn step(
    State(sim): State<SimHandle>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<Value>, StatusCode> {
    let frames: u32 = query
        .get("frames")
        .and_then(|frames| frames.parse().ok())
        .filter(|frames| (1..=FRAMES_MAX).contains(frames))
        .ok_or(StatusCode::BAD_REQUEST)?;
    let frame = sim.step(frames).await.map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    Ok(Json(json!({ "frame": frame })))
}

impl Clock {
    /// Starts the clock of a simulation on a free local port.
    pub async fn start(sim: SimHandle) -> Clock {
        let app = Router::new().route("/step", post(step)).with_state(sim);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a free local port");
        let addr = listener.local_addr().expect("the address of the listener");
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the clock runs");
        });
        Clock { url: format!("http://{addr}"), task }
    }
}
