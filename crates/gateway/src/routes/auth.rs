//! Sign-in through the service of an identity provider: the start and
//! the callback.
//!
//! The start makes a state value and sends the browser to the service.
//! The service sends the browser back to the callback with a code and
//! the state. The callback takes the state only from the browser that
//! made the start: the start puts the state into a cookie, and the
//! callback compares the two. So a link to the callback that another
//! person made does nothing in the browser of a victim.

use std::time::Instant;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header::{CACHE_CONTROL, COOKIE, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;
use unsup_store::{IdentityError, SignUp, constant_time_eq};

use super::owners::{identity_error, sign_up_with};
use crate::client::ClientAddr;
use crate::error::ApiError;
use crate::extract::Params;
use crate::limit::{limit_address, retry_after_secs};
use crate::state::{Counters, Gateway};

/// Name of the cookie that ties a sign-in to the browser that started it.
const STATE_COOKIE: &str = "unsup_signin";
/// Greatest length of `code` and of `state` in the callback, in bytes.
const VALUE_MAX: usize = 1_024;

/// How the start answers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Format {
    /// A redirect to the service, for a browser.
    #[default]
    Redirect,
    /// The address of the service as JSON, for a program.
    Json,
}

/// The query of `GET /v1/auth/{provider}`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StartQuery {
    #[serde(default)]
    format: Format,
}

/// Builds the cookie with the state. An empty state removes the cookie.
fn state_cookie(provider: &str, state: &str, max_age: u64, secure: bool) -> Option<HeaderValue> {
    let secure = if secure { "; Secure" } else { "" };
    let text = format!(
        "{STATE_COOKIE}={state}; Path=/v1/auth/{provider}; Max-Age={max_age}; HttpOnly; \
         SameSite=Lax{secure}"
    );
    HeaderValue::from_str(&text).ok()
}

/// Reads the state cookie of a request.
fn cookie_state(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find_map(|(name, value)| (name == STATE_COOKIE).then_some(value))
}

/// Returns true when the request came through HTTPS, as a trusted proxy
/// says.
fn is_https(gateway: &Gateway, headers: &HeaderMap) -> bool {
    gateway.config().forwarded_hops > 0
        && headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|proto| proto.eq_ignore_ascii_case("https"))
}

/// `GET /v1/auth/{provider}`: starts a sign-in at the service of the
/// provider.
pub(crate) async fn start(
    State(gateway): State<Gateway>,
    ClientAddr(address): ClientAddr,
    Path(name): Path<String>,
    Params(query): Params<StartQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let inner = &gateway.inner;
    let unknown = || ApiError::not_found("This server has no sign-in with this provider.");
    let provider = inner.providers.iter().find(|p| p.name() == name).ok_or_else(unknown)?;
    // A start counts as a sign-up for the limit of the client address.
    if let Err(wait) = inner.signup_limiter.check(limit_address(address), Instant::now()) {
        Counters::add(&inner.counters.rate_limited);
        return Err(ApiError::rate_limited(retry_after_secs(wait)));
    }
    let Some(authorization) = provider.begin().map_err(identity_error)? else {
        return Err(ApiError::invalid(
            "This provider has no sign-in page. Sign up with `POST /v1/owners`.",
        ));
    };
    let secure = is_https(&gateway, &headers);
    let cookie = state_cookie(&name, &authorization.state, authorization.expires_in, secure)
        .ok_or_else(ApiError::internal)?;
    let mut response = match query.format {
        Format::Json => Json(json!({
            "provider": name,
            "authorize_url": authorization.url,
            "state": authorization.state,
            "expires_in": authorization.expires_in,
        }))
        .into_response(),
        Format::Redirect => {
            let target =
                HeaderValue::from_str(&authorization.url).map_err(|_| ApiError::internal())?;
            (StatusCode::SEE_OTHER, [(LOCATION, target)]).into_response()
        }
    };
    response.headers_mut().insert(SET_COOKIE, cookie);
    response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// The query of `GET /v1/auth/{provider}/callback`. The service writes
/// it, so a name that the server does not know is not an error.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    /// The service refused, or the person did not agree.
    #[serde(default)]
    error: Option<String>,
}

/// `GET /v1/auth/{provider}/callback`: ends a sign-in. Returns the owner
/// and a new owner token, like `POST /v1/owners`.
pub(crate) async fn callback(
    State(gateway): State<Gateway>,
    Path(name): Path<String>,
    Params(query): Params<CallbackQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let inner = &gateway.inner;
    let unknown = || ApiError::not_found("This server has no sign-in with this provider.");
    let provider = inner.providers.iter().find(|p| p.name() == name).ok_or_else(unknown)?;
    if query.error.is_some() {
        return Err(identity_error(IdentityError::Denied(
            "The sign-in did not end at the provider. Start it again.".into(),
        )));
    }
    let (Some(code), Some(state)) = (query.code, query.state) else {
        return Err(ApiError::invalid("The callback needs `code` and `state`."));
    };
    if code.is_empty() || code.len() > VALUE_MAX || state.is_empty() || state.len() > VALUE_MAX {
        return Err(ApiError::invalid("`code` and `state` have 1 to 1024 bytes."));
    }
    // Only the browser that made the start has the cookie with the state.
    let same_browser = cookie_state(&headers)
        .is_some_and(|cookie| constant_time_eq(cookie.as_bytes(), state.as_bytes()));
    if !same_browser {
        return Err(ApiError::invalid(
            "This browser did not start the sign-in, or the sign-in is too old. Start it again.",
        ));
    }
    let request = SignUp { name: None, credential: Some(format!("{state}:{code}")) };
    let (status, body) = sign_up_with(&gateway, provider.as_ref(), &request, false).await?;
    let mut response = (status, body).into_response();
    // The answer holds a token: no cache keeps it. The cookie did its job.
    response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(cookie) = state_cookie(&name, "", 0, is_https(&gateway, &headers)) {
        response.headers_mut().insert(SET_COOKIE, cookie);
    }
    Ok(response)
}
