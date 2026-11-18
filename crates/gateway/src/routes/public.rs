//! Routes that need no token.

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use unsup_protocol::SimError;

use crate::error::ApiError;
use crate::state::Gateway;

/// `GET /healthz`: says whether the simulation runs.
pub(crate) async fn healthz(State(gateway): State<Gateway>) -> Response {
    match gateway.sim().stats().await {
        Err(SimError::Shutdown) => {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "shutdown", "The simulation stopped.")
                .into_response()
        }
        // A busy simulation is alive.
        _ => "ok".into_response(),
    }
}

/// `GET /join.md`: the instructions that an agent reads to join.
pub(crate) async fn join(State(gateway): State<Gateway>) -> Result<Response, ApiError> {
    let missing = || ApiError::not_found("This server has no join instructions.");
    let path = gateway.config().join_file.as_ref().ok_or_else(missing)?;
    match tokio::fs::read(path).await {
        Ok(text) => {
            Ok(([(header::CONTENT_TYPE, "text/markdown; charset=utf-8")], text).into_response())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(missing()),
        Err(error) => {
            tracing::error!(%error, "cannot read the join file");
            Err(ApiError::internal())
        }
    }
}

/// `GET /v1/catalogue`: verbs with argument schemas, items, parameters.
pub(crate) async fn catalogue(State(gateway): State<Gateway>) -> Response {
    ([(header::CONTENT_TYPE, "application/json")], gateway.inner.catalogue.clone()).into_response()
}
