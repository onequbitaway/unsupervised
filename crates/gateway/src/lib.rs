//! HTTPS, WebSocket and MCP for agents.
//!
//! This is the first slice of the gateway: enough to create a resident, to
//! act and to observe. Authentication, rate limits, WebSocket and MCP come
//! on top of the same [`SimHandle`].

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_protocol::{
    ActionRequest, Detail, ObserveOpts, OwnerId, ResidentId, SimError, SpawnSpec,
};
use unsup_runtime::SimHandle;

/// Returns the routes that agents use.
pub fn router(sim: SimHandle) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/catalogue", get(catalogue))
        .route("/v1/dev/residents", post(create_resident))
        .route("/v1/act", post(act))
        .route("/v1/observe", get(observe))
        .with_state(sim)
}

/// An error answer: `{ "error": { "code", "message" } }` with a status.
struct ApiError(StatusCode, &'static str, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": { "code": self.1, "message": self.2 } }))).into_response()
    }
}

impl From<SimError> for ApiError {
    fn from(error: SimError) -> ApiError {
        let (status, code) = match error {
            SimError::UnknownResident => (StatusCode::NOT_FOUND, "unknown_resident"),
            SimError::Overloaded => (StatusCode::SERVICE_UNAVAILABLE, "overloaded"),
            SimError::Shutdown => (StatusCode::SERVICE_UNAVAILABLE, "shutdown"),
            SimError::NotHosted => (StatusCode::NOT_FOUND, "not_hosted"),
            SimError::Invalid => (StatusCode::BAD_REQUEST, "invalid"),
        };
        ApiError(status, code, error.to_string())
    }
}

/// Reads the resident from the bearer token. In this first slice the token
/// is the resident id.
fn resident(headers: &HeaderMap) -> Result<ResidentId, ApiError> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(ResidentId)
        .ok_or_else(|| {
            ApiError(StatusCode::UNAUTHORIZED, "unauthorized", "send a bearer token".to_string())
        })
}

async fn catalogue(State(sim): State<SimHandle>) -> Json<Value> {
    Json(json!(*sim.catalogue()))
}

#[derive(Deserialize)]
struct NewResident {
    name: String,
    #[serde(default)]
    model: Option<String>,
}

async fn create_resident(
    State(sim): State<SimHandle>,
    Json(body): Json<NewResident>,
) -> Result<Json<Value>, ApiError> {
    let spec = SpawnSpec { owner: OwnerId(0), name: body.name, model: body.model, city: None };
    let id = sim.spawn(spec).await?;
    Ok(Json(json!({ "resident": id })))
}

async fn act(
    State(sim): State<SimHandle>,
    headers: HeaderMap,
    Json(request): Json<ActionRequest>,
) -> Result<Json<Value>, ApiError> {
    let who = resident(&headers)?;
    let ack = sim.act(who, request).await?;
    let observation = sim.observe(who, ObserveOpts::default()).await?;
    Ok(Json(json!({ "acks": [ack], "observation": observation })))
}

#[derive(Deserialize)]
struct ObserveQuery {
    #[serde(default)]
    detail: Option<Detail>,
    #[serde(default)]
    since: Option<u64>,
}

async fn observe(
    State(sim): State<SimHandle>,
    headers: HeaderMap,
    Query(query): Query<ObserveQuery>,
) -> Result<Json<Value>, ApiError> {
    let who = resident(&headers)?;
    let mut opts = ObserveOpts::default();
    if let Some(detail) = query.detail {
        opts.detail = detail;
    }
    if let Some(since) = query.since {
        opts.since_notice = since;
    }
    Ok(Json(json!(sim.observe(who, opts).await?)))
}
