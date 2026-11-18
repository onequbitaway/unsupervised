//! The routes of the gateway.

mod admin;
mod agent;
mod owners;
mod public;
mod residents;
mod webhook;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::{HeaderName, HeaderValue, Method, StatusCode, header};
use axum::middleware;
use axum::routing::{get, post, put};
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::client::address_limit;
use crate::error::ApiError;
use crate::state::Gateway;
use crate::{mcp, ws};

impl Gateway {
    /// Returns the routes of the gateway: everything that owners, agents
    /// and administrators use. Merge it into the application. Serve the
    /// application with the address of the client
    /// (`into_make_service_with_connect_info::<SocketAddr>()`), or the
    /// limit for each address sees all clients as one.
    pub fn router(&self) -> Router {
        let limited = Router::new()
            .route("/join.md", get(public::join))
            .route("/v1/catalogue", get(public::catalogue))
            .route("/v1/owners", post(owners::sign_up))
            .route("/v1/residents", post(residents::create).get(residents::list))
            .route("/v1/residents/{id}/keys", post(residents::rotate_key))
            .route("/v1/me", get(agent::me))
            .route("/v1/act", post(agent::act))
            .route("/v1/observe", get(agent::observe))
            .route("/v1/wait", get(agent::wait))
            .route("/v1/routines", put(agent::routines))
            .route("/v1/webhook", put(webhook::put))
            .route("/v1/ws", get(ws::upgrade))
            .route("/mcp", post(mcp::post).delete(mcp::delete))
            .route("/admin/residents/{id}/freeze", post(admin::freeze))
            .route("/admin/residents/{id}/unfreeze", post(admin::unfreeze))
            .route("/admin/residents/{id}/despawn", post(admin::despawn))
            .route("/admin/params", post(admin::set_param))
            .route("/admin/stats", get(admin::stats))
            .method_not_allowed_fallback(method_not_allowed)
            .layer(DefaultBodyLimit::max(self.config().limits.max_body_bytes))
            .layer(middleware::from_fn_with_state(self.clone(), address_limit));
        // A monitor calls the health check often. It has no limit.
        Router::new()
            .route("/healthz", get(public::healthz))
            .merge(limited)
            .with_state(self.clone())
    }

    /// Returns the CORS rules for the whole application, built from
    /// `cors_origins`. Put it around the gateway routes and the viewer
    /// routes.
    pub fn cors(&self) -> CorsLayer {
        let origins = &self.config().cors_origins;
        let allow = if origins.iter().any(|origin| origin == "*") {
            AllowOrigin::any()
        } else {
            AllowOrigin::list(origins.iter().filter_map(|o| HeaderValue::from_str(o).ok()))
        };
        CorsLayer::new()
            .allow_origin(allow)
            .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
            .allow_headers(
                [header::AUTHORIZATION, header::CONTENT_TYPE, header::ACCEPT]
                    .into_iter()
                    .chain(mcp::REQUEST_HEADERS.map(HeaderName::from_static))
                    .collect::<Vec<_>>(),
            )
            .expose_headers([header::RETRY_AFTER, HeaderName::from_static(mcp::SESSION_HEADER)])
    }
}

/// The answer for a path of the gateway with a method that it does not
/// take.
async fn method_not_allowed() -> ApiError {
    ApiError::new(
        StatusCode::METHOD_NOT_ALLOWED,
        "method_not_allowed",
        "This path does not take this method.",
    )
}

/// The answer for a path that no route takes. The server uses it for
/// paths under `/v1`, `/admin` and `/mcp`.
#[must_use]
pub fn not_found() -> ApiError {
    ApiError::not_found("No route has this path.")
}
