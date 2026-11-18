//! Routes that the host adds to the gateway.
//!
//! The gateway knows no game module. A route that needs one, for example
//! the route with which a human sponsor publishes a mission, is code of
//! the host: the server, which puts all parts together. The host gives the
//! gateway an [`OwnerRoute`] for a path
//! ([`GatewayBuilder::owner_route`](crate::GatewayBuilder::owner_route)).
//! The gateway does what it does for each of its own routes, and then
//! calls the route of the host:
//!
//! - the limit for each client address and the limit of the body size;
//! - the owner token: who sends the request, the ban of the owner and the
//!   limit of requests for each token;
//! - no query, and a body that is JSON;
//! - the end of the call when the gateway stops.
//!
//! The route of the host checks the body. It passes every text in it
//! through the text filters with
//! [`Gateway::check_texts`](crate::Gateway::check_texts) before the text
//! goes to the simulation.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::Value;
use unsup_protocol::OwnerId;
use unsup_store::Owner;

use crate::error::ApiError;
use crate::state::Gateway;

/// A request with a good owner token, for a route that the host added.
#[derive(Clone, Debug, PartialEq)]
pub struct OwnerCall {
    /// The owner of the token, as the store knows it.
    pub owner: Owner,
    /// The body of the request, as JSON. The gateway did not check what
    /// is in it.
    pub body: Value,
}

/// The answer of an [`OwnerRoute`]: the status and the JSON body, or an
/// error in the format of the API. It is a boxed future, so that the
/// route can ask the simulation.
pub type OwnerRouteFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(StatusCode, Value), ApiError>> + Send + 'a>>;

/// A route for owners that the host adds to the gateway: `POST` with an
/// owner token and a JSON body.
///
/// See the [module documentation](crate::host) for what the gateway does
/// before it calls the route.
pub trait OwnerRoute: Send + Sync + 'static {
    /// Answers one request. `gateway` gives the text filters
    /// ([`Gateway::check_texts`]) and the configuration.
    fn call<'a>(&'a self, gateway: &'a Gateway, call: OwnerCall) -> OwnerRouteFuture<'a>;
}

/// The query of a route of the host: it takes no query field.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NoQuery {}

/// Runs a route of the host for a request that has a good owner token.
pub(crate) async fn serve(
    gateway: Gateway,
    route: Arc<dyn OwnerRoute>,
    owner: OwnerId,
    body: Value,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    // The token is good, so the store has the owner.
    let owner = gateway
        .store(move |store| store.owner(owner))
        .await?
        .ok_or_else(ApiError::invalid_token)?;
    tokio::select! {
        answer = route.call(&gateway, OwnerCall { owner, body }) => {
            answer.map(|(status, body)| (status, Json(body)))
        }
        // A route can wait for the simulation. A stop does not wait for it.
        () = gateway.stopped() => Err(ApiError::shutdown()),
    }
}
