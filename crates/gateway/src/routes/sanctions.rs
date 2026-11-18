//! Routes of the administrator for sanctions: mute a resident, ban an
//! owner, lift both, and list what is in force. `freeze` and `unfreeze`
//! are in the module of the simulation commands.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_protocol::{OwnerId, ResidentId};
use unsup_store::Sanction;

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::extract::{OptionalBody, PathId};
use crate::state::Gateway;

/// Greatest length of the reason of a sanction, in bytes.
const REASON_MAX: usize = 500;

/// The optional body of a route that imposes a sanction.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReasonBody {
    /// Why. The list of sanctions shows it.
    #[serde(default)]
    reason: Option<String>,
}

impl ReasonBody {
    /// Returns the reason without the spaces around it, or an empty text.
    pub(crate) fn into_reason(self) -> Result<String, ApiError> {
        let reason = self.reason.unwrap_or_default().trim().to_string();
        if reason.len() > REASON_MAX {
            return Err(ApiError::invalid(format!("`reason` has at most {REASON_MAX} bytes.")));
        }
        if reason.chars().any(|c| c.is_control() && c != '\n') {
            return Err(ApiError::invalid("`reason` must not hold control characters."));
        }
        Ok(reason)
    }
}

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

/// `POST /admin/residents/{id}/mute`: the resident cannot send text any
/// more. Routines that speak are removed.
pub(crate) async fn mute(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
    OptionalBody(body): OptionalBody<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    let reason = body.into_reason()?;
    tracing::info!(%resident, %reason, "admin: mute");
    gateway.impose(Sanction::Mute(resident), reason).await?;
    gateway.silence(resident).await;
    Ok(ok())
}

/// `POST /admin/residents/{id}/unmute`: undoes `mute`.
pub(crate) async fn unmute(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    tracing::info!(%resident, "admin: unmute");
    gateway.lift(Sanction::Mute(resident)).await?;
    Ok(ok())
}

/// `POST /admin/owners/{id}/ban`: no token of the owner and no key of its
/// residents works any more. Open connections and waiting calls end.
pub(crate) async fn ban(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
    OptionalBody(body): OptionalBody<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let owner = OwnerId(id);
    let reason = body.into_reason()?;
    tracing::info!(%owner, %reason, "admin: ban");
    gateway.impose(Sanction::Ban(owner), reason).await?;
    let residents = gateway.store(move |store| store.residents_of(owner)).await?;
    let ids: Vec<ResidentId> = residents.iter().map(|record| record.resident).collect();
    // The deliveries stop. The store keeps the webhooks.
    for resident in &ids {
        gateway.inner.webhooks.clear(*resident);
    }
    Ok(Json(json!({ "ok": true, "residents": ids })))
}

/// `POST /admin/owners/{id}/unban`: undoes `ban`.
pub(crate) async fn unban(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    let owner = OwnerId(id);
    tracing::info!(%owner, "admin: unban");
    if gateway.lift(Sanction::Ban(owner)).await? {
        // The webhooks of the residents deliver again.
        let saved = gateway
            .store(move |store| {
                let mut saved = Vec::new();
                for record in store.residents_of(owner)? {
                    saved.extend(store.webhook(record.resident)?);
                }
                Ok(saved)
            })
            .await?;
        for webhook in saved {
            gateway.inner.webhooks.set(webhook.resident, &webhook.url, webhook.secret).await;
        }
    }
    Ok(ok())
}

/// `GET /admin/sanctions`: every sanction in force.
pub(crate) async fn list(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
) -> Result<Json<Value>, ApiError> {
    let records = gateway.store(|store| store.sanctions()).await?;
    let sanctions: Vec<Value> = records
        .iter()
        .map(|record| {
            let subject = match record.sanction {
                Sanction::Ban(_) => "owner",
                Sanction::Mute(_) | Sanction::Freeze(_) => "resident",
            };
            json!({
                "kind": record.sanction.kind(),
                subject: record.sanction.subject(),
                "reason": record.reason,
                "created_at": record.created_at,
            })
        })
        .collect();
    Ok(Json(json!({ "sanctions": sanctions })))
}
