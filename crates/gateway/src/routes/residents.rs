//! Routes of an owner: make residents, list them, rotate a key.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};
use unsup_protocol::{AdminCommand, CityId, ResidentId, SimError, SpawnSpec};
use unsup_store::{NewResident, ResidentRecord, StoreError, Token, TokenKind, unix_now};

use crate::auth::OwnerAuth;
use crate::error::ApiError;
use crate::extract::{Body, PathId};
use crate::state::Gateway;
use crate::validate;

/// The body of `POST /v1/residents`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NewResidentBody {
    /// Display name, 1 to 64 bytes.
    name: String,
    /// Name of the model that controls the resident, at most 64 bytes.
    #[serde(default)]
    model: Option<String>,
    /// The city to start in. The city with the fewest residents when
    /// absent.
    #[serde(default)]
    city: Option<u16>,
}

fn resident_json(record: &ResidentRecord) -> Value {
    json!({
        "id": record.resident,
        "name": record.name,
        "model": record.model,
        "created_at": record.created_at,
        "key_hint": record.key_hint,
    })
}

/// `POST /v1/residents`: makes a resident and returns it with its key.
pub(crate) async fn create(
    State(gateway): State<Gateway>,
    OwnerAuth(owner): OwnerAuth,
    Body(body): Body<NewResidentBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let name = validate::name("name", &body.name)?;
    let model = validate::optional_name("model", body.model.as_deref())?;

    // One creation at a time: the count below stays true until the new
    // record is written.
    let _one_at_a_time = gateway.inner.creation.lock().await;
    let limit = gateway.config().limits.residents_per_owner;
    if limit > 0 && gateway.live_residents(owner).await?.len() >= limit as usize {
        return Err(ApiError::forbidden(
            "resident_limit",
            format!("An owner has at most {limit} residents in the world at a time."),
        ));
    }

    let spec =
        SpawnSpec { owner, name: name.clone(), model: model.clone(), city: body.city.map(CityId) };
    let resident = gateway.sim().spawn(spec).await.map_err(|error| match error {
        SimError::Invalid => ApiError::invalid(
            "The simulation refused the resident. Check that `city` is the id of a city.",
        ),
        other => ApiError::from(other),
    })?;

    let key = Token::generate(TokenKind::Resident)?;
    let (hash, hint) = (key.hash(), key.hint());
    let new = NewResident { resident, owner, name, model };
    let saved =
        gateway.store(move |store| store.create_resident(&new, &hash, &hint, unix_now())).await;
    let record = match saved {
        Ok(record) => record,
        Err(error) => {
            // Without a record nobody could control the resident.
            let _ = gateway.sim().admin(AdminCommand::Despawn { resident }).await;
            return Err(error);
        }
    };
    tracing::info!(%owner, %resident, "resident created");
    Ok((
        StatusCode::CREATED,
        Json(json!({ "resident": resident_json(&record), "key": key.reveal() })),
    ))
}

/// `GET /v1/residents`: lists the residents of the owner that are in the
/// world.
pub(crate) async fn list(
    State(gateway): State<Gateway>,
    OwnerAuth(owner): OwnerAuth,
) -> Result<Json<Value>, ApiError> {
    let residents: Vec<Value> =
        gateway.live_residents(owner).await?.iter().map(resident_json).collect();
    let limit = gateway.config().limits.residents_per_owner;
    Ok(Json(json!({ "residents": residents, "limit": limit })))
}

/// `POST /v1/residents/{id}/keys`: replaces the key of a resident. The
/// old key stops working at once.
pub(crate) async fn rotate_key(
    State(gateway): State<Gateway>,
    OwnerAuth(owner): OwnerAuth,
    PathId(id): PathId,
) -> Result<Json<Value>, ApiError> {
    let resident = ResidentId(id);
    // A resident of another owner looks the same as no resident.
    let missing = || ApiError::not_found("You have no resident with this id.");
    let record = gateway.store(move |store| store.resident(resident)).await?.ok_or_else(missing)?;
    if record.owner != owner {
        return Err(missing());
    }
    let key = Token::generate(TokenKind::Resident)?;
    let (hash, hint) = (key.hash(), key.hint());
    let rotated = gateway
        .store(move |store| match store.rotate_resident_key(resident, &hash, &hint, unix_now()) {
            Ok(()) => Ok(true),
            // The resident left the world between the two store calls.
            Err(StoreError::NotFound(_)) => Ok(false),
            Err(error) => Err(error),
        })
        .await?;
    if !rotated {
        return Err(missing());
    }
    tracing::info!(%owner, %resident, "resident key rotated");
    Ok(Json(json!({ "resident": resident, "key": key.reveal(), "key_hint": key.hint() })))
}
