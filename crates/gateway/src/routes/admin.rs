//! Routes of the administrator.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_protocol::{AdminCommand, AdminReply, CityId, ResidentId};
use unsup_store::{Sanction, StoreError, unix_now};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::extract::{Body, OptionalBody, PathId};
use crate::routes::sanctions::ReasonBody;
use crate::state::{Counters, Gateway};

/// Runs a command in the simulation. A refusal of the simulation is the
/// status 409 with its reason.
async fn run(gateway: &Gateway, command: AdminCommand) -> Result<Json<Value>, ApiError> {
    match gateway.sim().admin(command).await? {
        AdminReply::Ok => Ok(Json(json!({ "ok": true }))),
        AdminReply::Error { message } => {
            Err(ApiError::new(StatusCode::CONFLICT, "refused", message))
        }
    }
}

/// `POST /admin/residents/{id}/freeze`: stops a resident. Its action
/// fails, its queue is cleared and every later request is refused. The
/// store keeps the freeze, so that it comes back after a rollback of the
/// world.
pub(crate) async fn freeze(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
    OptionalBody(body): OptionalBody<ReasonBody>,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    let reason = body.into_reason()?;
    tracing::info!(%resident, %reason, "admin: freeze");
    let answer = run(&gateway, AdminCommand::Freeze { resident }).await?;
    let sanction = Sanction::Freeze(resident);
    let kept = gateway
        .store(move |store| match store.impose(sanction, &reason, unix_now()) {
            Ok(_) => Ok(true),
            // A resident without an owner record: the world holds the
            // freeze, the store cannot.
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        })
        .await?;
    if kept {
        gateway.inner.sanctions.set(sanction, true);
    } else {
        tracing::warn!(%resident, "the freeze is not in the store: the resident has no record");
    }
    Ok(answer)
}

/// `POST /admin/residents/{id}/unfreeze`: undoes `freeze`.
pub(crate) async fn unfreeze(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    tracing::info!(%resident, "admin: unfreeze");
    // The store first: a freeze that the world no longer has must not
    // come back.
    gateway.lift(Sanction::Freeze(resident)).await?;
    run(&gateway, AdminCommand::Unfreeze { resident }).await
}

/// `POST /admin/residents/{id}/despawn`: removes a resident from the
/// world. Its key stops working.
pub(crate) async fn despawn(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    tracing::info!(%resident, "admin: despawn");
    let answer = run(&gateway, AdminCommand::Despawn { resident }).await?;
    gateway.retire(resident, "admin").await;
    Ok(answer)
}

/// The body of `POST /admin/params`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamBody {
    /// Name of the parameter.
    name: String,
    /// New value. The simulation clamps it to the allowed range.
    value: i64,
    /// Change the value for one city only. The national value when
    /// absent.
    #[serde(default)]
    city: Option<u16>,
}

/// `POST /admin/params`: changes a parameter of the simulation.
pub(crate) async fn set_param(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    Body(body): Body<ParamBody>,
) -> Result<Json<Value>, ApiError> {
    tracing::info!(name = %body.name, value = body.value, city = ?body.city, "admin: set param");
    let command =
        AdminCommand::SetParam { name: body.name, value: body.value, city: body.city.map(CityId) };
    run(&gateway, command).await
}

/// `GET /admin/stats`: numbers about the simulation, the store and the
/// gateway.
pub(crate) async fn stats(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
) -> Result<Json<Value>, ApiError> {
    let sim = gateway.sim().stats().await?;
    let counts = gateway.store(|store| store.counts()).await?;
    let inner = &gateway.inner;
    Ok(Json(json!({
        "sim": sim,
        "store": {
            "owners": counts.owners,
            "residents": counts.residents,
            "retired": counts.retired,
            "webhooks": counts.webhooks,
        },
        "gateway": {
            "ws_connections": inner.sockets.total(),
            "ws_dropped": Counters::get(&inner.counters.ws_dropped),
            "open_waits": inner.waits.total(),
            "watchers": inner.hub.watchers(),
            "mcp_sessions": inner.sessions.len(),
            "rate_limited": Counters::get(&inner.counters.rate_limited),
            "webhooks": inner.webhooks.stats(),
        },
    })))
}
