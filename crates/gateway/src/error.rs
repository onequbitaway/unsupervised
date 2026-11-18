//! The error answer of the gateway.

use axum::Json;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use unsup_protocol::SimError;
use unsup_store::StoreError;

/// An error answer: `{ "error": { "code", "message" } }` with a status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiError {
    /// The HTTP status.
    pub status: StatusCode,
    /// A short name of the error that a program can test.
    pub code: &'static str,
    /// One or two sentences for the caller.
    pub message: String,
    /// Seconds after which the caller can try again.
    pub retry_after: Option<u64>,
}

impl ApiError {
    /// Makes an error.
    #[must_use]
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError { status, code, message: message.into(), retry_after: None }
    }

    /// 400: the request has a wrong shape or a wrong value.
    #[must_use]
    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    /// 401: the request has no usable bearer token.
    #[must_use]
    pub fn unauthorized(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }

    /// 401: the server does not know the token.
    #[must_use]
    pub fn invalid_token() -> ApiError {
        ApiError::new(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "The server does not know this token. It is wrong or it was replaced.",
        )
    }

    /// 403: the token is good but it does not permit the request.
    #[must_use]
    pub fn forbidden(code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, code, message)
    }

    /// 404: the thing that the request names does not exist.
    #[must_use]
    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not_found", message)
    }

    /// 410: the resident of the key is no longer in the world.
    #[must_use]
    pub fn resident_gone() -> ApiError {
        ApiError::new(
            StatusCode::GONE,
            "resident_gone",
            "The resident is no longer in the world. Its key does not work any more.",
        )
    }

    /// 403: an administrator muted the resident.
    #[must_use]
    pub fn muted() -> ApiError {
        ApiError::forbidden(
            "muted",
            "An administrator muted this resident. It cannot send text. Every other action \
             works.",
        )
    }

    /// 403: an administrator banned the owner of the token.
    #[must_use]
    pub fn banned() -> ApiError {
        ApiError::forbidden(
            "banned",
            "An administrator banned the owner of this token. No token of the owner and no \
             key of its residents works.",
        )
    }

    /// 422: a filter refused a text of the request. Nothing reached the
    /// simulation.
    #[must_use]
    pub fn text_refused(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "text_refused", message)
    }

    /// 429: too many requests. `wait_secs` goes into `Retry-After`.
    #[must_use]
    pub fn rate_limited(wait_secs: u64) -> ApiError {
        ApiError {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "rate_limited",
            message: format!("Too many requests. Try again in {wait_secs} seconds."),
            retry_after: Some(wait_secs),
        }
    }

    /// 429: too many connections or waiting calls at a time.
    #[must_use]
    pub fn too_many(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too_many_connections", message)
    }

    /// 500: the server failed. The message for the caller says nothing
    /// about the cause; the log has it.
    #[must_use]
    pub fn internal() -> ApiError {
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "The server failed. Try again later.",
        )
    }

    /// 503: the server stops.
    #[must_use]
    pub fn shutdown() -> ApiError {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "shutdown", "The server stops.")
    }

    /// Returns the body of the answer.
    #[must_use]
    pub fn body(&self) -> serde_json::Value {
        json!({ "error": { "code": self.code, "message": self.message } })
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(self.body())).into_response();
        if let Some(seconds) = self.retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(seconds));
        }
        if self.status == StatusCode::UNAUTHORIZED {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        response
    }
}

impl From<SimError> for ApiError {
    fn from(error: SimError) -> ApiError {
        let (status, code) = match error {
            SimError::UnknownResident => (StatusCode::NOT_FOUND, "unknown_resident"),
            SimError::NotHosted => (StatusCode::SERVICE_UNAVAILABLE, "not_hosted"),
            SimError::Overloaded => (StatusCode::SERVICE_UNAVAILABLE, "overloaded"),
            SimError::Shutdown => (StatusCode::SERVICE_UNAVAILABLE, "shutdown"),
            SimError::Invalid => (StatusCode::BAD_REQUEST, "invalid_request"),
        };
        let mut message = error.to_string();
        if let Some(first) = message.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        ApiError::new(status, code, format!("{message}."))
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> ApiError {
        // The cause can name a file or a table. It goes to the log only.
        tracing::error!(%error, "a store call failed");
        ApiError::internal()
    }
}
