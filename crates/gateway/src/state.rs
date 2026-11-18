//! The gateway value that every handler shares.

use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::body::Bytes;
use tokio::sync::watch;
use unsup_protocol::{OwnerId, ResidentId, SimError};
use unsup_runtime::SimHandle;
use unsup_store::{IdentityProvider, LocalSignUp, ResidentRecord, Store, StoreError, unix_now};

use crate::config::GatewayConfig;
use crate::error::ApiError;
use crate::gauge::Gauge;
use crate::limit::RateLimiter;
use crate::mcp::session::Sessions;
use crate::push::{Hub, cursor_now};
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
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway").finish_non_exhaustive()
    }
}

impl Gateway {
    /// Starts the gateway with local sign-up as the only identity
    /// provider. Call it inside a Tokio runtime: it starts the tasks that
    /// push notices and deliver webhooks.
    ///
    /// # Errors
    ///
    /// Fails when the store cannot list the webhooks.
    pub async fn start(
        sim: SimHandle,
        store: Arc<dyn Store>,
        config: GatewayConfig,
    ) -> Result<Gateway, StoreError> {
        Gateway::start_with_providers(sim, store, config, vec![Arc::new(LocalSignUp)]).await
    }

    /// Starts the gateway with a list of identity providers. A sign-up
    /// names a provider; the first one is the default.
    ///
    /// # Errors
    ///
    /// Fails when the store cannot list the webhooks.
    pub async fn start_with_providers(
        sim: SimHandle,
        store: Arc<dyn Store>,
        config: GatewayConfig,
        providers: Vec<Arc<dyn IdentityProvider>>,
    ) -> Result<Gateway, StoreError> {
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
                providers,
                sim,
                store,
                config,
            }),
        };

        tokio::spawn(hub.run(gateway.inner.sim.frames()));
        let store = Arc::clone(&gateway.inner.store);
        let saved = tokio::task::spawn_blocking(move || store.webhooks())
            .await
            .map_err(|e| StoreError::Engine(format!("the store task failed: {e}")))??;
        for webhook in saved {
            gateway.inner.webhooks.set(webhook.resident, &webhook.url, webhook.secret).await;
        }
        Ok(gateway)
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
