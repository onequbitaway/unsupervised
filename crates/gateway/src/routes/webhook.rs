//! The route that sets the webhook of a resident.

use axum::Json;
use axum::extract::State;
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use unsup_store::{Secret, StoreError, unix_now};

use crate::auth::ResidentAuth;
use crate::error::ApiError;
use crate::extract::Body;
use crate::state::Gateway;
use crate::webhook::url;

/// The body of `PUT /v1/webhook`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebhookBody {
    /// The address that gets the deliveries. `null` removes the webhook.
    /// The field must be there: a body without it removes nothing.
    #[serde(deserialize_with = "present")]
    url: Option<String>,
}

/// Reads a field that can be `null` but cannot be absent.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::deserialize(deserializer)
}

/// `PUT /v1/webhook`: sets, replaces or removes the webhook of the
/// resident. Setting it returns a new signing secret, shown once.
pub(crate) async fn put(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Body(body): Body<WebhookBody>,
) -> Result<Json<Value>, ApiError> {
    let resident = auth.resident;
    let Some(address) = body.url else {
        gateway.inner.webhooks.clear(resident);
        gateway.store(move |store| store.clear_webhook(resident)).await?;
        return Ok(Json(json!({ "url": null })));
    };

    let checked =
        url::check(&address, &gateway.config().webhooks).await.map_err(ApiError::invalid)?;
    let secret = Secret::generate_webhook()?;
    let (text, stored) = (checked.to_string(), secret.clone());
    let saved = gateway
        .store(move |store| match store.set_webhook(resident, &text, &stored, unix_now()) {
            Ok(()) => Ok(true),
            // The resident left the world.
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        })
        .await?;
    if !saved {
        return Err(ApiError::resident_gone());
    }
    gateway.inner.webhooks.set(resident, checked.as_str(), secret.clone()).await;
    // The address can hold a secret path, so the log has the host only.
    tracing::info!(%resident, host = checked.host_str(), "webhook set");
    Ok(Json(json!({ "url": checked.as_str(), "secret": secret.reveal() })))
}
