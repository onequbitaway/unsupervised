//! Routes of a resident key: check the key, act, observe, wait, set
//! routines.

use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_protocol::{ActionStatus, Detail, ErrorCode, Observation, ObserveOpts};

use crate::actions::{observe_opts, routines_request, run_plan, set_routines, wait_filter};
use crate::auth::ResidentAuth;
use crate::error::ApiError;
use crate::extract::{Body, Params};
use crate::push::cursor_now;
use crate::state::Gateway;
use crate::validate;

/// Characters that a notice kind has in addition to those of a section
/// name: kinds look like `society.speech`.
const KIND_EXTRA: &str = ".";

/// The query of `GET /v1/observe` and of `POST /v1/act`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ObserveQuery {
    /// `brief` or `full`.
    #[serde(default)]
    detail: Option<Detail>,
    /// Only notices and reports with a sequence number at or above this.
    #[serde(default)]
    since: Option<u64>,
    /// Names of the sections to return, with commas between them.
    #[serde(default)]
    sections: Option<String>,
}

impl ObserveQuery {
    fn into_opts(self, gateway: &Gateway) -> Result<ObserveOpts, ApiError> {
        let max = gateway.config().limits.max_names;
        let sections = self
            .sections
            .map(|text| validate::name_list("sections", &text, "", max))
            .transpose()?;
        Ok(observe_opts(self.detail, self.since, sections))
    }
}

/// `GET /v1/me`: checks the key and says which resident it controls.
pub(crate) async fn me(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
) -> Result<Json<Value>, ApiError> {
    let resident = auth.resident;
    if let Err(error) = cursor_now(gateway.sim(), resident).await {
        return Err(gateway.resident_error(resident, error).await);
    }
    let record = gateway
        .store(move |store| store.resident(resident))
        .await?
        .ok_or_else(ApiError::resident_gone)?;
    Ok(Json(json!({
        "resident": record.resident,
        "name": record.name,
        "model": record.model,
        "owner": record.owner,
    })))
}

/// `POST /v1/act`: sends one action request or a plan. Returns the
/// acknowledgements and a fresh observation.
pub(crate) async fn act(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Params(query): Params<ObserveQuery>,
    Body(body): Body<Value>,
) -> Result<Json<Value>, ApiError> {
    let resident = auth.resident;
    let opts = query.into_opts(&gateway)?;
    let (requests, _) =
        validate::act_body(body, gateway.config().limits.max_plan).map_err(ApiError::invalid)?;
    gateway.admit(resident, &requests).await?;
    let acks = match run_plan(gateway.sim(), resident, requests).await {
        Ok(acks) => acks,
        Err(error) => return Err(gateway.resident_error(resident, error).await),
    };
    let observation = observe_as(&gateway, auth, opts).await?;
    Ok(Json(json!({ "acks": acks, "observation": observation })))
}

async fn observe_as(
    gateway: &Gateway,
    auth: ResidentAuth,
    opts: ObserveOpts,
) -> Result<Observation, ApiError> {
    match gateway.sim().observe(auth.resident, opts).await {
        Ok(observation) => Ok(observation),
        Err(error) => Err(gateway.resident_error(auth.resident, error).await),
    }
}

/// `GET /v1/observe`: what the resident can see now.
pub(crate) async fn observe(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Params(query): Params<ObserveQuery>,
) -> Result<Json<Observation>, ApiError> {
    let opts = query.into_opts(&gateway)?;
    observe_as(&gateway, auth, opts).await.map(Json)
}

/// The query of `GET /v1/wait`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WaitQuery {
    /// Greatest time to wait, in seconds. It can have a fraction.
    #[serde(default)]
    timeout: Option<f64>,
    /// The `next_notice` of the last observation. News at or above it
    /// counts, also when it came before the call. Without it, only news
    /// that comes after the call counts.
    #[serde(default)]
    since: Option<u64>,
    /// Notice kinds to wait for, with commas between them. An action
    /// report has the kind `report`. Without it, any news ends the wait.
    #[serde(default)]
    kinds: Option<String>,
}

/// `GET /v1/wait`: a long poll. Returns an observation when the resident
/// gets news, or without news when the time is over.
pub(crate) async fn wait(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Params(query): Params<WaitQuery>,
) -> Result<Json<Observation>, ApiError> {
    let limits = &gateway.config().limits;
    let max = limits.max_wait_secs;
    let seconds = query.timeout.unwrap_or(limits.default_wait_secs.min(max) as f64);
    if !(seconds.is_finite() && (0.0..=max as f64).contains(&seconds)) {
        return Err(ApiError::invalid(format!(
            "`timeout` is a number of seconds from 0 to {max}."
        )));
    }
    let kinds = query
        .kinds
        .map(|text| validate::name_list("kinds", &text, KIND_EXTRA, limits.max_names))
        .transpose()?;

    let resident = auth.resident;
    let _open =
        gateway.inner.waits.acquire(resident, limits.waits_per_resident).ok_or_else(|| {
            ApiError::too_many(format!(
                "A resident has at most {} `wait` calls open at a time.",
                limits.waits_per_resident
            ))
        })?;
    let call = gateway.sim().wait_since(
        resident,
        wait_filter(kinds),
        query.since,
        Duration::from_secs_f64(seconds),
    );
    tokio::select! {
        answer = call => match answer {
            Ok(observation) => Ok(Json(observation)),
            Err(error) => Err(gateway.resident_error(resident, error).await),
        },
        () = gateway.stopped() => Err(ApiError::shutdown()),
        () = gateway.inner.sanctions.banned(auth.owner) => Err(ApiError::banned()),
    }
}

/// The body of `PUT /v1/routines`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RoutinesBody {
    /// The new routines, in priority order. They replace all routines.
    routines: Vec<Value>,
}

/// `PUT /v1/routines`: replaces the routines of the resident.
pub(crate) async fn routines(
    State(gateway): State<Gateway>,
    auth: ResidentAuth,
    Body(body): Body<RoutinesBody>,
) -> Result<Json<Value>, ApiError> {
    let resident = auth.resident;
    let routines =
        validate::routines_from_values("routines", body.routines).map_err(ApiError::invalid)?;
    gateway.admit(resident, &[routines_request(&routines)]).await?;
    let ack = match set_routines(gateway.sim(), resident, &routines).await {
        Ok(ack) => ack,
        Err(error) => return Err(gateway.resident_error(resident, error).await),
    };
    match ack.status {
        ActionStatus::Rejected { error } => {
            let status = match error.code {
                ErrorCode::InvalidArgs | ErrorCode::UnknownVerb => StatusCode::BAD_REQUEST,
                ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
                _ => StatusCode::CONFLICT,
            };
            Err(ApiError::new(status, error.code.as_str(), error.message))
        }
        // The verb is free: it ends at once, and its result is the count.
        ActionStatus::Done { result } => Ok(Json(result)),
        _ => Ok(Json(json!({ "routines": routines.len() }))),
    }
}
