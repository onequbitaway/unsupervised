//! Sign-up of owners.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_store::{IdentityError, NewOwner, Owner, SignUp, StoreError, Token, TokenKind, unix_now};

use crate::client::ClientAddr;
use crate::error::ApiError;
use crate::extract::Body;
use crate::limit::{limit_address, retry_after_secs};
use crate::state::{Counters, Gateway};

/// The body of `POST /v1/owners`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignUpBody {
    /// Display name of the owner.
    #[serde(default)]
    name: Option<String>,
    /// Name of the identity provider. The first provider when absent.
    #[serde(default)]
    provider: Option<String>,
    /// The proof that the provider needs.
    #[serde(default)]
    credential: Option<String>,
}

fn owner_json(owner: &Owner) -> Value {
    json!({ "id": owner.id, "name": owner.name, "created_at": owner.created_at })
}

/// `POST /v1/owners`: signs up and returns the owner and an owner token.
pub(crate) async fn sign_up(
    State(gateway): State<Gateway>,
    ClientAddr(address): ClientAddr,
    Body(body): Body<SignUpBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let inner = &gateway.inner;
    if let Err(wait) = inner.signup_limiter.check(limit_address(address), Instant::now()) {
        Counters::add(&inner.counters.rate_limited);
        return Err(ApiError::rate_limited(retry_after_secs(wait)));
    }
    let provider = match &body.provider {
        Some(name) => inner.providers.iter().find(|p| p.name() == name),
        None => inner.providers.first(),
    }
    .ok_or_else(|| {
        let names: Vec<&str> = inner.providers.iter().map(|p| p.name()).collect();
        ApiError::invalid(format!(
            "`provider` is not known. This server has: {}.",
            names.join(", ")
        ))
    })?;

    let request = SignUp { name: body.name, credential: body.credential };
    let identity = provider.identify(&request).await.map_err(|error| match error {
        IdentityError::Invalid(message) => ApiError::invalid(message),
        IdentityError::Denied(message) => ApiError::forbidden("sign_up_denied", message),
        IdentityError::Unavailable(message) => {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable", message)
        }
    })?;

    let token = Token::generate(TokenKind::Owner)?;
    let (hash, hint) = (token.hash(), token.hint());
    let (owner, created) = gateway
        .store(move |store| {
            let now = unix_now();
            if let Some(owner) = store.owner_by_identity(&identity.provider, &identity.subject)? {
                store.add_owner_token(owner.id, &hash, &hint, now)?;
                return Ok((owner, false));
            }
            let new = NewOwner {
                name: identity.name,
                provider: identity.provider,
                subject: identity.subject,
            };
            match store.create_owner(&new, &hash, &hint, now) {
                Ok(owner) => Ok((owner, true)),
                // The same identity signed up in another request just now.
                Err(StoreError::Conflict(_)) => {
                    let owner = store
                        .owner_by_identity(&new.provider, &new.subject)?
                        .ok_or_else(|| StoreError::Engine("an owner disappeared".into()))?;
                    store.add_owner_token(owner.id, &hash, &hint, now)?;
                    Ok((owner, false))
                }
                Err(error) => Err(error),
            }
        })
        .await?;

    tracing::info!(owner = %owner.id, created, "owner signed up");
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(json!({ "owner": owner_json(&owner), "token": token.reveal() }))))
}
