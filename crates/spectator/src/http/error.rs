//! Error answers: `{ "error": { "code", "message" } }` with a status.

use axum::Json;
use axum::extract::rejection::QueryRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::hub::NoWorld;
use crate::wire::ErrorBody;

/// An error answer of the HTTP API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiError {
    status: StatusCode,
    body: ErrorBody,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> ApiError {
        ApiError { status, body: ErrorBody { code: code.to_string(), message: message.into() } }
    }

    /// The thing that the request names is not there.
    pub fn not_found(code: &str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, code, message)
    }

    /// The request is not valid.
    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        #[derive(serde::Serialize)]
        struct Body {
            error: ErrorBody,
        }
        (self.status, Json(Body { error: self.body })).into_response()
    }
}

impl From<NoWorld> for ApiError {
    fn from(_: NoWorld) -> ApiError {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "shutdown", "The simulation has stopped.")
    }
}

impl From<QueryRejection> for ApiError {
    fn from(rejection: QueryRejection) -> ApiError {
        ApiError::invalid(format!("The query is not valid: {}.", rejection.body_text()))
    }
}
