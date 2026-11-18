//! Sign-up of owners.

use std::time::Instant;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_store::{
    IdentityError, IdentityProvider, NewOwner, Owner, SignUp, StoreError, Token, TokenKind,
    unix_now,
};

use crate::client::ClientAddr;
use crate::error::ApiError;
use crate::extract::Body;
use crate::limit::{limit_address, retry_after_secs};
use crate::moderation::{Author, TextUse};
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

/// Converts the refusal of an identity provider into an answer.
pub(crate) fn identity_error(error: IdentityError) -> ApiError {
    match error {
        IdentityError::Invalid(message) => ApiError::invalid(message),
        IdentityError::Denied(message) => ApiError::forbidden("sign_up_denied", message),
        IdentityError::Unavailable(message) => {
            ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable", message)
        }
    }
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

    // A name that the client chose passes the text filters.
    let named = body.name.is_some();
    let request = SignUp { name: body.name, credential: body.credential };
    sign_up_with(&gateway, provider.as_ref(), &request, named).await
}

/// Asks a provider who signs up, finds or makes the owner, and gives it a
/// new owner token. With `check_name`, the display name passes the text
/// filters first.
pub(crate) async fn sign_up_with(
    gateway: &Gateway,
    provider: &dyn IdentityProvider,
    request: &SignUp,
    check_name: bool,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let identity = provider.identify(request).await.map_err(identity_error)?;
    if check_name {
        gateway.admit_name(Author::Visitor, TextUse::OwnerName, "name", &identity.name).await?;
    }

    let token = Token::generate(TokenKind::Owner)?;
    let (hash, hint) = (token.hash(), token.hint());
    let banned = {
        let gateway = gateway.clone();
        move |owner: &Owner| gateway.inner.sanctions.is_banned(owner.id)
    };
    let signed = gateway
        .store(move |store| {
            let now = unix_now();
            if let Some(owner) = store.owner_by_identity(&identity.provider, &identity.subject)? {
                // A banned owner gets no new token.
                if banned(&owner) {
                    return Ok(None);
                }
                store.add_owner_token(owner.id, &hash, &hint, now)?;
                return Ok(Some((owner, false)));
            }
            let new = NewOwner {
                name: identity.name,
                provider: identity.provider,
                subject: identity.subject,
            };
            match store.create_owner(&new, &hash, &hint, now) {
                Ok(owner) => Ok(Some((owner, true))),
                // The same identity signed up in another request just now.
                Err(StoreError::Conflict(_)) => {
                    let owner = store
                        .owner_by_identity(&new.provider, &new.subject)?
                        .ok_or_else(|| StoreError::Engine("an owner disappeared".into()))?;
                    if banned(&owner) {
                        return Ok(None);
                    }
                    store.add_owner_token(owner.id, &hash, &hint, now)?;
                    Ok(Some((owner, false)))
                }
                Err(error) => Err(error),
            }
        })
        .await?;
    let Some((owner, created)) = signed else {
        Counters::add(&gateway.inner.counters.banned);
        return Err(ApiError::banned());
    };

    tracing::info!(owner = %owner.id, created, "owner signed up");
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(json!({ "owner": owner_json(&owner), "token": token.reveal() }))))
}
