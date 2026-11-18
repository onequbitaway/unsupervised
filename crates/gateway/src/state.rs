//! The gateway value that every handler shares.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use serde_json::Value;
use tokio::sync::watch;
use unsup_protocol::{OwnerId, ResidentId, SimError};
use unsup_runtime::SimHandle;
use unsup_store::{IdentityProvider, LocalSignUp, ResidentRecord, Store, StoreError, unix_now};

use crate::config::{ConfigError, GatewayConfig};
use crate::error::ApiError;
use crate::gauge::Gauge;
use crate::host::OwnerRoute;
use crate::identity::GitHubSignIn;
use crate::limit::RateLimiter;
use crate::mcp::session::Sessions;
use crate::moderation::{Moderation, RuleFilter, TextFilter};
use crate::ops::Ops;
use crate::ops::alerts::Alerts;
use crate::push::{Hub, cursor_now};
use crate::sanctions::Sanctions;
use crate::webhook::Webhooks;

/// The key of a rate limit bucket: one for each owner and each resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LimitKey {
    Owner(OwnerId),
    Resident(ResidentId),
}

/// Counters for the statistics of an administrator.
#[derive(Debug, Default)]
pub(crate) struct Counters {
    /// Requests that a rate limit refused.
    pub(crate) rate_limited: AtomicU64,
    /// WebSocket connections that the server dropped because the client
    /// read too slowly.
    pub(crate) ws_dropped: AtomicU64,
    /// Requests with a token of a banned owner.
    pub(crate) banned: AtomicU64,
}

impl Counters {
    pub(crate) fn add(counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn get(counter: &AtomicU64) -> u64 {
        counter.load(Ordering::Relaxed)
    }
}

/// The gateway: everything that serves agents, owners and administrators.
///
/// Make one with [`Gateway::start`], put [`Gateway::router`] into the
/// application and call [`Gateway::shutdown`] before the server stops.
/// The value is a handle: a clone talks to the same gateway.
#[derive(Clone)]
pub struct Gateway {
    pub(crate) inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub(crate) sim: SimHandle,
    pub(crate) store: Arc<dyn Store>,
    pub(crate) config: GatewayConfig,
    pub(crate) providers: Vec<Arc<dyn IdentityProvider>>,
    pub(crate) key_limiter: RateLimiter<LimitKey>,
    pub(crate) address_limiter: RateLimiter<IpAddr>,
    pub(crate) signup_limiter: RateLimiter<IpAddr>,
    /// Makes resident creation one at a time, so that two requests of one
    /// owner cannot both pass the limit.
    pub(crate) creation: tokio::sync::Mutex<()>,
    pub(crate) hub: Arc<Hub>,
    pub(crate) waits: Arc<Gauge<ResidentId>>,
    pub(crate) sockets: Arc<Gauge<ResidentId>>,
    pub(crate) sessions: Sessions,
    pub(crate) webhooks: Webhooks,
    pub(crate) stopping: watch::Sender<bool>,
    /// The catalogue as JSON. It does not change while the server runs.
    pub(crate) catalogue: Bytes,
    pub(crate) counters: Counters,
    pub(crate) moderation: Moderation,
    pub(crate) sanctions: Sanctions,
    pub(crate) ops: Ops,
    /// Counts the rollbacks of the world. An open connection watches it:
    /// its cursors must go back with the world.
    pub(crate) rollbacks: watch::Sender<u64>,
    /// The routes for owners that the host added, each with its path.
    pub(crate) owner_routes: Vec<(String, Arc<dyn OwnerRoute>)>,
}

/// The gateway could not start.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StartError {
    /// The configuration has a value that the gateway cannot use.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The store could not be read.
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Collects what a gateway needs, then starts it.
///
/// ```no_run
/// # async fn example(sim: unsup_runtime::SimHandle) -> Result<(), unsup_gateway::StartError> {
/// use std::sync::Arc;
/// use unsup_gateway::{Gateway, GatewayConfig};
/// use unsup_store::MemoryStore;
///
/// let store = Arc::new(MemoryStore::new());
/// let gateway = Gateway::builder(sim, store, GatewayConfig::default()).start().await?;
/// # Ok(())
/// # }
/// ```
pub struct GatewayBuilder {
    sim: SimHandle,
    store: Arc<dyn Store>,
    config: GatewayConfig,
    providers: Vec<Arc<dyn IdentityProvider>>,
    filters: Vec<Arc<dyn TextFilter>>,
    owner_routes: Vec<(String, Arc<dyn OwnerRoute>)>,
}

impl std::fmt::Debug for GatewayBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let paths: Vec<&str> = self.owner_routes.iter().map(|(path, _)| path.as_str()).collect();
        f.debug_struct("GatewayBuilder")
            .field("providers", &self.providers.len())
            .field("filters", &self.filters.len())
            .field("owner_routes", &paths)
            .finish_non_exhaustive()
    }
}

impl GatewayBuilder {
    /// Replaces the list of identity providers. A sign-up names a
    /// provider; the first one is the default. Without this call the
    /// list has local sign-up only. The provider `github` joins the list
    /// when the configuration turns it on.
    #[must_use]
    pub fn providers(mut self, providers: Vec<Arc<dyn IdentityProvider>>) -> GatewayBuilder {
        self.providers = providers;
        self
    }

    /// Adds a text filter. The gateway asks the rule filter of the
    /// configuration first, then the filters of these calls in their
    /// order. A filter that asks a model goes here.
    #[must_use]
    pub fn filter(mut self, filter: Arc<dyn TextFilter>) -> GatewayBuilder {
        self.filters.push(filter);
        self
    }

    /// Adds a route for owners that the host answers: `POST path` with an
    /// owner token and a JSON body. The gateway authenticates the owner
    /// and applies its limits, then it calls `route` (see
    /// [`OwnerRoute`]). A game module needs such a route when a human
    /// sends something to it; the gateway itself knows no module.
    ///
    /// `path` is a whole path without a parameter, for example
    /// `/v1/missions`. [`GatewayBuilder::start`] refuses another form of
    /// path and a path that was added before. A path that the gateway
    /// serves itself makes [`Gateway::router`] panic: the host is a
    /// program, and two routes for one path are an error in it.
    #[must_use]
    pub fn owner_route(mut self, path: &str, route: Arc<dyn OwnerRoute>) -> GatewayBuilder {
        self.owner_routes.push((path.to_string(), route));
        self
    }

    /// Starts the gateway. Call it inside a Tokio runtime: it starts the
    /// tasks that push notices, deliver webhooks and watch the frames.
    ///
    /// # Errors
    ///
    /// Fails when the configuration has a rule or an endpoint that cannot
    /// be used, when the path of a route of the host has a wrong form or
    /// is there two times, and when the store cannot be read.
    pub async fn start(self) -> Result<Gateway, StartError> {
        let GatewayBuilder { sim, store, config, mut providers, filters, owner_routes } = self;
        check_owner_paths(&owner_routes)?;
        let rules: Arc<dyn TextFilter> = Arc::new(RuleFilter::new(&config.moderation)?);
        let filters = std::iter::once(rules).chain(filters).collect();
        if config.github.is_on() && providers.iter().all(|p| p.name() != GitHubSignIn::NAME) {
            providers.push(Arc::new(GitHubSignIn::new(&config.github)?));
        }

        // What the store kept from the last run: sanctions and webhooks.
        let reader = Arc::clone(&store);
        let (sanctions, saved) = tokio::task::spawn_blocking(move || {
            let sanctions = reader.sanctions()?;
            let mut saved = Vec::new();
            for webhook in reader.webhooks()? {
                let owner = reader.resident(webhook.resident)?.map(|record| record.owner);
                saved.push((webhook, owner));
            }
            Ok::<_, StoreError>((sanctions, saved))
        })
        .await
        .map_err(|e| StoreError::Engine(format!("the store task failed: {e}")))??;

        let limits = &config.limits;
        let minute = Duration::from_secs(60);
        let hour = Duration::from_secs(3600);
        let hub = Arc::new(Hub::default());
        let (stopping, _) = watch::channel(false);
        // The catalogue is plain data; it always turns into JSON.
        let catalogue = Bytes::from(serde_json::to_vec(&*sim.catalogue()).unwrap_or_default());
        let gateway = Gateway {
            inner: Arc::new(Inner {
                key_limiter: RateLimiter::new(
                    limits.key_requests_per_minute,
                    minute,
                    limits.key_burst,
                ),
                address_limiter: RateLimiter::new(
                    limits.address_requests_per_minute,
                    minute,
                    limits.address_burst,
                ),
                signup_limiter: RateLimiter::new(
                    limits.signups_per_hour,
                    hour,
                    limits.signup_burst,
                ),
                creation: tokio::sync::Mutex::new(()),
                waits: Gauge::new(),
                sockets: Gauge::new(),
                sessions: Sessions::new(&config.mcp),
                webhooks: Webhooks::new(&config.webhooks, sim.clone(), Arc::clone(&hub)),
                hub: Arc::clone(&hub),
                stopping,
                catalogue,
                counters: Counters::default(),
                moderation: Moderation::new(config.moderation.clone(), filters),
                sanctions: Sanctions::load(&sanctions),
                ops: Ops::new(&config.ops, config.abuse.funnel.clone()),
                rollbacks: watch::channel(0).0,
                owner_routes,
                providers,
                sim,
                store,
                config,
            }),
        };

        tokio::spawn(hub.run(gateway.inner.sim.frames()));
        tokio::spawn(gateway.clone().watch(gateway.inner.sim.frames()));
        for (webhook, owner) in saved {
            // A banned owner gets no deliveries. The store keeps the
            // webhook for the day on which the ban is lifted.
            if owner.is_some_and(|owner| gateway.inner.sanctions.is_banned(owner)) {
                continue;
            }
            gateway.inner.webhooks.set(webhook.resident, &webhook.url, webhook.secret).await;
        }
        Ok(gateway)
    }
}

/// Checks the paths of the routes that the host added: each is a plain
/// path (`/` and then parts of letters, digits, `_`, `-` and `.`, with `/`
/// between them), and no path is there two times.
fn check_owner_paths(routes: &[(String, Arc<dyn OwnerRoute>)]) -> Result<(), ConfigError> {
    for (index, (path, _)) in routes.iter().enumerate() {
        let plain = path.strip_prefix('/').is_some_and(|rest| {
            rest.split('/').all(|part| {
                !part.is_empty()
                    && part.chars().all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
            })
        });
        if !plain {
            return Err(ConfigError(format!(
                "the path `{path}` of a route of the host is not a plain path such as \
                 `/v1/missions`"
            )));
        }
        if routes[..index].iter().any(|(other, _)| other == path) {
            return Err(ConfigError(format!("the path `{path}` has two routes of the host")));
        }
    }
    Ok(())
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").finish_non_exhaustive()
    }
}

impl Gateway {
    /// Returns a builder for a gateway with local sign-up as the only
    /// identity provider and the rule filter as the only text filter.
    #[must_use]
    pub fn builder(sim: SimHandle, store: Arc<dyn Store>, config: GatewayConfig) -> GatewayBuilder {
        GatewayBuilder {
            sim,
            store,
            config,
            providers: vec![Arc::new(LocalSignUp)],
            filters: Vec::new(),
            owner_routes: Vec::new(),
        }
    }

    /// Starts the gateway with local sign-up as the first identity
    /// provider. Call it inside a Tokio runtime: it starts the tasks that
    /// push notices, deliver webhooks and watch the frames.
    ///
    /// # Errors
    ///
    /// See [`GatewayBuilder::start`].
    pub async fn start(
        sim: SimHandle,
        store: Arc<dyn Store>,
        config: GatewayConfig,
    ) -> Result<Gateway, StartError> {
        Gateway::builder(sim, store, config).start().await
    }

    /// Starts the gateway with a list of identity providers. A sign-up
    /// names a provider; the first one is the default.
    ///
    /// # Errors
    ///
    /// See [`GatewayBuilder::start`].
    pub async fn start_with_providers(
        sim: SimHandle,
        store: Arc<dyn Store>,
        config: GatewayConfig,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Result<Gateway, StartError> {
        Gateway::builder(sim, store, config).providers(providers).start().await
    }

    /// Returns the alert list. The host raises an alert for a problem
    /// that only it can see, for example an audit of the world that
    /// failed.
    #[must_use]
    pub fn alerts(&self) -> Arc<Alerts> {
        Arc::clone(&self.inner.ops.alerts)
    }

    /// Gives the gateway a function that describes the host: the data
    /// directory, the recovery at the start, the snapshots. The admin
    /// dashboard shows its answer in the field `host`.
    pub fn set_host_info(&self, info: impl Fn() -> Value + Send + Sync + 'static) {
        self.inner.ops.set_host(Arc::new(info));
    }

    /// Returns the configuration.
    #[must_use]
    pub fn config(&self) -> &GatewayConfig {
        &self.inner.config
    }

    pub(crate) fn sim(&self) -> &SimHandle {
        &self.inner.sim
    }

    /// Tells every open connection and every waiting call to end, and
    /// stops the webhook deliveries. Call it before the HTTP server
    /// stops, so that a graceful stop does not wait for them.
    pub fn shutdown(&self) {
        self.inner.stopping.send_replace(true);
        self.inner.webhooks.stop();
    }

    /// Waits until [`Gateway::shutdown`] was called.
    pub(crate) async fn stopped(&self) {
        let mut stopping = self.inner.stopping.subscribe();
        // The sender lives as long as the gateway, so this cannot fail.
        let _ = stopping.wait_for(|stopping| *stopping).await;
    }

    /// Runs a store call on a thread that may block.
    pub(crate) async fn store<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn Store) -> Result<T, StoreError> + Send + 'static,
    ) -> Result<T, ApiError> {
        let store = Arc::clone(&self.inner.store);
        match tokio::task::spawn_blocking(move || work(store.as_ref())).await {
            Ok(result) => result.map_err(ApiError::from),
            Err(error) => {
                tracing::error!(%error, "a store task failed");
                Err(ApiError::internal())
            }
        }
    }

    /// Forgets a resident that left the world: its record ends, its key
    /// stops working, its webhook and its MCP state go.
    pub(crate) async fn retire(&self, resident: ResidentId, reason: &'static str) {
        self.inner.webhooks.clear(resident);
        self.inner.sessions.forget(resident);
        // The store ends the mute and the freeze with the record.
        self.inner.sanctions.forget(resident);
        let retired = self.store(move |s| s.retire_resident(resident, reason, unix_now())).await;
        if matches!(retired, Ok(true)) {
            tracing::info!(%resident, reason, "resident record retired");
        }
    }

    /// Converts an error of the simulation for a call that a resident
    /// key made. A resident that the simulation no longer knows is
    /// retired.
    pub(crate) async fn resident_error(&self, resident: ResidentId, error: SimError) -> ApiError {
        if error == SimError::UnknownResident {
            self.retire(resident, "gone").await;
            return ApiError::resident_gone();
        }
        ApiError::from(error)
    }

    /// Returns the residents of an owner that are in the world. A record
    /// whose resident the simulation no longer knows is retired first.
    pub(crate) async fn live_residents(
        &self,
        owner: OwnerId,
    ) -> Result<Vec<ResidentRecord>, ApiError> {
        let records = self.store(move |s| s.residents_of(owner)).await?;
        let mut live = Vec::with_capacity(records.len());
        for record in records {
            match cursor_now(self.sim(), record.resident).await {
                Ok(_) => live.push(record),
                Err(SimError::UnknownResident) => self.retire(record.resident, "gone").await,
                Err(error) => return Err(ApiError::from(error)),
            }
        }
        Ok(live)
    }
}
