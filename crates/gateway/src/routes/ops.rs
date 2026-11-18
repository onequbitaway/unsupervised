//! Routes of the administrator for operations: the dashboard, the alert
//! list, the flag list and the rollback of the world.

use axum::Json;
use axum::extract::State;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::auth::AdminAuth;
use crate::error::ApiError;
use crate::extract::{Body, PathId};
use crate::state::{Counters, Gateway};

/// `GET /admin/dashboard`: everything that an operator looks at first.
pub(crate) async fn dashboard(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
) -> Result<Json<Value>, ApiError> {
    let sim = gateway.sim().stats().await;
    gateway.look_at_sim().await;
    let sim = sim?;
    let counts = gateway.store(|store| store.counts()).await?;
    let inner = &gateway.inner;
    let ops = &inner.ops;
    let mut moderation = inner.moderation.stats();
    moderation["banned_requests"] = json!(Counters::get(&inner.counters.banned));
    moderation["sanctions"] = inner.sanctions.counts();
    let flags = ops.flags();
    let open = flags.iter().filter(|flag| !flag.dismissed).count();
    Ok(Json(json!({
        "sim": sim,
        "journal": {
            "records": sim.journal_records,
            "not_on_disk": sim.journal_lag,
            "errors": sim.journal_errors,
        },
        "snapshots": {
            "written": sim.snapshots,
            "errors": sim.snapshot_errors,
            "last_frame": sim.last_snapshot_frame,
            "last_bytes": sim.last_snapshot_bytes,
            "last_stall_micros": sim.last_snapshot_micros,
            "last_write_micros": sim.last_snapshot_write_micros,
            "rollbacks": sim.rollbacks,
        },
        "economy": ops.economy(),
        "moderation": moderation,
        "abuse": {
            "watched_frame": ops.frame(),
            "frames_lost": ops.frames_lost(),
            "open_flags": open,
            "flags": flags,
        },
        "alerts": ops.alerts.list(),
        "store": {
            "owners": counts.owners,
            "residents": counts.residents,
            "retired": counts.retired,
            "webhooks": counts.webhooks,
        },
        "gateway": {
            "ws_connections": inner.sockets.total(),
            "open_waits": inner.waits.total(),
            "mcp_sessions": inner.sessions.len(),
            "rate_limited": Counters::get(&inner.counters.rate_limited),
        },
        "host": ops.host(),
    })))
}

/// `GET /admin/alerts`: the alert list. The route looks at the simulation
/// first, so the list is current.
pub(crate) async fn alerts(State(gateway): State<Gateway>, _admin: AdminAuth) -> Json<Value> {
    gateway.look_at_sim().await;
    Json(json!({ "alerts": gateway.inner.ops.alerts.list() }))
}

/// `GET /admin/flags`: the findings of the abuse detectors, with the last
/// frame that the detectors saw.
pub(crate) async fn flags(State(gateway): State<Gateway>, _admin: AdminAuth) -> Json<Value> {
    let ops = &gateway.inner.ops;
    Json(json!({ "watched_frame": ops.frame(), "flags": ops.flags() }))
}

/// `POST /admin/flags/{id}/dismiss`: closes a flag.
pub(crate) async fn dismiss(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    if gateway.inner.ops.dismiss(id) {
        tracing::info!(flag = id, "admin: flag dismissed");
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::not_found("No open flag has this id."))
    }
}

/// The body of `POST /admin/rollback`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RollbackBody {
    /// The world goes back to the newest snapshot at or before this
    /// frame.
    frame: u64,
}

/// `POST /admin/rollback`: puts the whole world back to a snapshot.
pub(crate) async fn rollback(
    State(gateway): State<Gateway>,
    _admin: AdminAuth,
    Body(body): Body<RollbackBody>,
) -> Result<Json<Value>, ApiError> {
    tracing::warn!(frame = body.frame, "admin: rollback");
    gateway.rollback(body.frame).await.map(Json)
}
