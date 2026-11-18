//! Extractors that answer a bad request with the error format of the API.

use axum::body::Bytes;
use axum::extract::{FromRequest, FromRequestParts, Path, Query, Request};
use axum::http::request::Parts;
use axum::http::{HeaderMap, StatusCode, header};
use serde::de::DeserializeOwned;

use crate::error::ApiError;

/// Returns true when the request says that its body is JSON.
pub(crate) fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or_default().trim().to_ascii_lowercase())
        .is_some_and(|mime| mime == "application/json" || mime.ends_with("+json"))
}

/// Reads the whole body of a request, up to the size limit of the router.
pub(crate) async fn body_bytes<S: Send + Sync>(
    request: Request,
    state: &S,
) -> Result<Bytes, ApiError> {
    Bytes::from_request(request, state).await.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
                "The request body is larger than the server permits.",
            )
        } else {
            ApiError::invalid("The request body cannot be read.")
        }
    })
}

/// Reads JSON text into a value, with an error that names the place.
pub(crate) fn parse_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(bytes)
        .map_err(|error| ApiError::invalid(format!("The body is not valid: {error}.")))
}

/// A JSON request body.
#[derive(Debug)]
pub(crate) struct Body<T>(pub(crate) T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        if !is_json(request.headers()) {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Send the body as JSON with the header `Content-Type: application/json`.",
            ));
        }
        let bytes = body_bytes(request, state).await?;
        parse_json(&bytes).map(Body)
    }
}

/// The query string of a request.
#[derive(Debug)]
pub(crate) struct Params<T>(pub(crate) T);

impl<T: DeserializeOwned, S: Send + Sync> FromRequestParts<S> for Params<T> {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, ApiError> {
        Query::<T>::try_from_uri(&parts.uri)
            .map(|Query(value)| Params(value))
            .map_err(|error| ApiError::invalid(format!("The query is not valid: {error}.")))
    }
}

/// An id in the path of a request.
#[derive(Debug)]
pub(crate) struct PathId(pub(crate) u64);

impl<S: Send + Sync> FromRequestParts<S> for PathId {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, ApiError> {
        Path::<u64>::from_request_parts(parts, state)
            .await
            .map(|Path(id)| PathId(id))
            .map_err(|_| ApiError::invalid("The id in the path must be a whole number."))
    }
}

/// A JSON request body that can be absent: a request without a body
/// reads as the default value.
#[derive(Debug)]
pub(crate) struct OptionalBody<T>(pub(crate) T);

impl<T: DeserializeOwned + Default, S: Send + Sync> FromRequest<S> for OptionalBody<T> {
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, ApiError> {
        let json = is_json(request.headers());
        let bytes = body_bytes(request, state).await?;
        if bytes.is_empty() {
            return Ok(OptionalBody(T::default()));
        }
        if !json {
            return Err(ApiError::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media_type",
                "Send the body as JSON with the header `Content-Type: application/json`.",
            ));
        }
        parse_json(&bytes).map(OptionalBody)
    }
}
