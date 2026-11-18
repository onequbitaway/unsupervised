//! Authentication: who sent a request.
//!
//! There are three kinds of token. An owner token and a resident key are
//! in the store, as hashes. The admin token is in the configuration. All
//! travel as `Authorization: Bearer <token>`.

use std::time::Instant;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, header};
use unsup_protocol::{OwnerId, ResidentId};
use unsup_store::{Principal, TokenHash, TokenKind};

use crate::error::ApiError;
use crate::limit::retry_after_secs;
use crate::state::{Counters, Gateway, LimitKey};

/// Reads the bearer token of a request.
pub(crate) fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    let missing =
        || ApiError::unauthorized("Send the token in the header `Authorization: Bearer <token>`.");
    let value =
        headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).ok_or_else(missing)?;
    let (scheme, token) = value.split_once(' ').ok_or_else(missing)?;
    let token = token.trim();
    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() || token.contains(' ') {
        return Err(missing());
    }
    Ok(token)
}

impl Gateway {
    /// Finds who a request comes from, and takes one request from the
    /// rate limit of that owner or resident.
    pub(crate) async fn authenticate(&self, headers: &HeaderMap) -> Result<Principal, ApiError> {
        let token = bearer(headers)?;
        // A text that cannot be a token of this server needs no lookup.
        if TokenKind::of(token).is_none() {
            return Err(ApiError::invalid_token());
        }
        let hash = TokenHash::of(token);
        let principal = self
            .store(move |store| store.authenticate(&hash))
            .await?
            .ok_or_else(ApiError::invalid_token)?;
        let key = match principal {
            Principal::Owner(owner) => LimitKey::Owner(owner),
            Principal::Resident { resident, .. } => LimitKey::Resident(resident),
        };
        self.take_request(key)?;
        Ok(principal)
    }

    /// Takes one request from the rate limit of a key.
    pub(crate) fn take_request(&self, key: LimitKey) -> Result<(), ApiError> {
        self.inner.key_limiter.check(key, Instant::now()).map_err(|wait| {
            Counters::add(&self.inner.counters.rate_limited);
            ApiError::rate_limited(retry_after_secs(wait))
        })
    }
}

/// A request with a good owner token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OwnerAuth(pub(crate) OwnerId);

impl FromRequestParts<Gateway> for OwnerAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, gateway: &Gateway) -> Result<Self, ApiError> {
        match gateway.authenticate(&parts.headers).await? {
            Principal::Owner(owner) => Ok(OwnerAuth(owner)),
            Principal::Resident { .. } => Err(ApiError::forbidden(
                "wrong_token_kind",
                "This route needs an owner token. You sent a resident key.",
            )),
        }
    }
}

/// A request with a good resident key.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ResidentAuth {
    pub(crate) resident: ResidentId,
}

impl FromRequestParts<Gateway> for ResidentAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, gateway: &Gateway) -> Result<Self, ApiError> {
        match gateway.authenticate(&parts.headers).await? {
            Principal::Resident { resident, .. } => Ok(ResidentAuth { resident }),
            Principal::Owner(_) => Err(ApiError::forbidden(
                "wrong_token_kind",
                "This route needs a resident key. You sent an owner token.",
            )),
        }
    }
}

/// A request with the admin token.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AdminAuth;

impl FromRequestParts<Gateway> for AdminAuth {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, gateway: &Gateway) -> Result<Self, ApiError> {
        let Some(admin) = &gateway.config().admin_token else {
            return Err(ApiError::forbidden(
                "admin_disabled",
                "This server has no admin token. Set one in the configuration.",
            ));
        };
        if admin.matches(bearer(&parts.headers)?) {
            Ok(AdminAuth)
        } else {
            Err(ApiError::invalid_token())
        }
    }
}
