//! Starting and stopping a server.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
use unsup_gateway::{AdminToken, Gateway};
use unsup_kernel::WorldConfig;
use unsup_protocol::OwnerId;
use unsup_runtime::{Dilation, DilationConfig, RuntimeConfig, RuntimeJoin, SimHandle};
use unsup_store::{
    MemoryStore, NewOwner, SqliteStore, Store, Token, TokenKind, random_hex, unix_now,
};

use crate::config::{ServerConfig, StoreConfig, WorldStore};
use crate::durable::{self, WorldStart, hash_text};

/// Paths that belong to the API. An unknown path under one of them gets a
/// JSON error, never a file of the viewer.
const API_PREFIXES: [&str; 3] = ["/v1/", "/admin/", "/mcp"];
/// Number of random bytes in the admin token of the development mode.
const DEV_ADMIN_TOKEN_BYTES: usize = 24;

/// What the development mode made ready.
#[derive(Clone)]
pub struct DevAccess {
    /// The owner that the server made.
    pub owner: OwnerId,
    /// The token of that owner.
    pub owner_token: String,
    /// The admin token that the server made. `None` when the
    /// configuration had one already.
    pub admin_token: Option<String>,
}

impl std::fmt::Debug for DevAccess {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The tokens stay out of logs.
        f.debug_struct("DevAccess").field("owner", &self.owner).finish_non_exhaustive()
    }
}

/// What a server left behind when it stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stopped {
    /// Number of the last frame of the world.
    pub frame: u64,
    /// The state hash of the world at the stop, as hexadecimal text. A
    /// start from the same data directory gives a world with this hash.
    pub state_hash: String,
    /// Number of residents of the world.
    pub residents: usize,
}

/// A running server.
pub struct Server {
    /// The address that the server listens on.
    pub addr: SocketAddr,
    /// Handle to the simulation, for tools and tests.
    pub sim: SimHandle,
    /// The owner and the tokens of the development mode.
    pub dev: Option<DevAccess>,
    /// How the world of this run began: recovered or new, and its state
    /// hash at the start.
    pub world: WorldStart,
    gateway: Gateway,
    runtime: RuntimeJoin,
    grace: Duration,
    stop: oneshot::Sender<()>,
    /// True when the grace time of a stop is over: see [`cut_at_stop`].
    cut: watch::Sender<bool>,
    task: JoinHandle<std::io::Result<()>>,
}

/// Opens the store of the configuration.
fn open_store(config: &StoreConfig) -> anyhow::Result<Arc<dyn Store>> {
    match config {
        StoreConfig::Memory => Ok(Arc::new(MemoryStore::new())),
        StoreConfig::Sqlite(path) => {
            if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("cannot make the data directory {}", dir.display()))?;
            }
            let store = SqliteStore::open(path)
                .with_context(|| format!("cannot open the store {}", path.display()))?;
            Ok(Arc::new(store))
        }
    }
}

/// Makes the owner of the development mode, and an admin token when the
/// configuration has none.
fn dev_access(store: &dyn Store, config: &mut ServerConfig) -> anyhow::Result<DevAccess> {
    let token = Token::generate(TokenKind::Owner)?;
    let new = NewOwner {
        name: "Developer".into(),
        provider: "local".into(),
        subject: format!("dev-{}", random_hex(8)?),
    };
    let owner = store.create_owner(&new, &token.hash(), &token.hint(), unix_now())?;
    let admin_token = match config.gateway.admin_token {
        Some(_) => None,
        None => {
            let admin = format!("unsup_adm_{}", random_hex(DEV_ADMIN_TOKEN_BYTES)?);
            config.gateway.admin_token = Some(AdminToken::new(&admin)?);
            Some(admin)
        }
    };
    Ok(DevAccess { owner: owner.id, owner_token: token.reveal().to_string(), admin_token })
}

/// Returns true when an error of the file system means that the viewer
/// has no file with the name that a request asked for.
fn is_missing_file(error: &std::io::Error) -> bool {
    use std::io::ErrorKind::{NotADirectory, NotFound, PermissionDenied};
    matches!(error.kind(), NotFound | PermissionDenied | NotADirectory)
}

/// Answers a path that no route takes: a file of the viewer, or a JSON
/// error for a path of the API.
async fn fallback(State(viewer): State<Option<ServeDir>>, request: Request) -> Response {
    let path = request.uri().path();
    let api = API_PREFIXES.iter().any(|prefix| path.starts_with(prefix));
    match viewer {
        Some(mut files) if !api => match files.try_call(request).await {
            Ok(response) => response.map(axum::body::Body::new),
            // A browser asks for a file. The answer has no JSON body.
            Err(error) if is_missing_file(&error) => StatusCode::NOT_FOUND.into_response(),
            Err(error) => {
                tracing::error!(%error, "cannot read a file of the viewer");
                unsup_gateway::ApiError::internal().into_response()
            }
        },
        _ => unsup_gateway::not_found().into_response(),
    }
}

/// Middleware: ends a request that still runs when the grace time of a
/// stop is over. The request gets the answer `shutdown`, and the work
/// that it started is dropped. After that moment no route runs any more,
/// so nothing uses the simulation or the store behind the back of the
/// stop.
async fn cut_at_stop(
    State(mut cut): State<watch::Receiver<bool>>,
    request: Request,
    next: Next,
) -> Response {
    tokio::select! {
        response = next.run(request) => response,
        // The sender is gone when the server value was dropped. That is
        // a stop too.
        _ = cut.wait_for(|cut| *cut) => unsup_gateway::ApiError::shutdown().into_response(),
    }
}

/// Returns the world configuration for a new world: the shape and the
/// parameter override files.
fn world_config(config: &ServerConfig) -> anyhow::Result<WorldConfig> {
    let mut world = config.world.to_config();
    world.data = durable::read_params(&config.params)?;
    Ok(world)
}

/// Starts the simulation and the HTTP server.
///
/// The world has the durable journal of the runtime: every input goes to
/// the input log, and a snapshot is taken at an interval. With a world
/// directory, the start recovers the world that the directory holds, also
/// after a crash; an empty directory gets a new world. A new world ends
/// the resident records of the store. A recovered world keeps them.
///
/// # Errors
///
/// Fails when the world shape or a parameter file is not valid, when the
/// world directory holds a world that cannot be recovered or is in use by
/// another server, when the store cannot be opened, or when the address
/// cannot be bound.
pub async fn start(mut config: ServerConfig) -> anyhow::Result<Server> {
    let store = open_store(&config.store)?;
    // Accounts in memory do not outlive the world on the disk: after a
    // restart nobody would have a key for its residents.
    let world_store = match (&config.store, &config.world_store) {
        (StoreConfig::Memory, WorldStore::Dir(_)) => WorldStore::Memory,
        (_, world_store) => world_store.clone(),
    };
    let opened = durable::open(
        &world_store,
        world_config(&config)?,
        config.durability.to_runtime(),
        config.durability.audit_every,
    )?;
    let world = opened.start.clone();
    if world.recovered {
        tracing::info!(
            frame = world.frame,
            replayed_inputs = world.replayed_inputs,
            torn_tail = world.torn_tail,
            "world recovered from the data directory"
        );
        if !config.params.is_empty() {
            tracing::warn!(
                "the parameter files were not read: the recovered world keeps its parameters"
            );
        }
    } else {
        // The world is new. The resident ids of an earlier world mean
        // nothing in it, so their records and keys end. Owners and their
        // tokens stay.
        let retired = store.retire_all_residents("world_reset", unix_now())?;
        if retired > 0 {
            tracing::info!(retired, "resident records of an earlier world retired");
        }
        tracing::info!(frame = world.frame, "new world");
    }
    let dev = if config.dev { Some(dev_access(store.as_ref(), &mut config)?) } else { None };

    let mut runtime_config = RuntimeConfig {
        tick_interval: config.tick_interval,
        journal: opened.journal,
        ..RuntimeConfig::default()
    };
    if config.durability.dilation {
        runtime_config.pacer = Box::new(Dilation::new(DilationConfig::default()));
    }
    let (sim, runtime) = unsup_runtime::start(opened.world, runtime_config);
    let gateway = Gateway::start(sim.clone(), store, config.gateway.clone())
        .await
        .context("cannot start the gateway")?;

    // What only the host knows, for the admin dashboard.
    opened.board.attach(gateway.alerts());
    let (board, storage) = (opened.board, opened.storage);
    let place = match &world_store {
        WorldStore::Memory => json!("memory"),
        WorldStore::Dir(dir) => json!(dir),
    };
    let started = world.clone();
    gateway.set_host_info(move || {
        json!({
            "world_store": place,
            "start": started,
            "audit": board.report(),
            "snapshots": durable::snapshots(storage.as_ref()),
        })
    });
    if world.recovered {
        // The store and the world were written apart. A record whose
        // resident the world does not have ends before a request can
        // use it.
        let reconciled = gateway.reconcile().await.map_err(|error| {
            anyhow::anyhow!("cannot compare the store with the world: {}", error.message)
        })?;
        if !reconciled.orphans.is_empty() {
            gateway.alerts().raise(
                "orphan_residents",
                format!(
                    "{} residents of the recovered world have no owner record. Nobody has a \
                     key for them.",
                    reconciled.orphans.len()
                ),
            );
        }
    }

    let viewer = config.web_dir.filter(|dir| dir.is_dir()).map(ServeDir::new);
    let (cut, cut_signal) = watch::channel(false);
    let app: Router = gateway
        .router()
        .merge(unsup_spectator::router(sim.clone()))
        .fallback_service(Router::new().fallback(fallback).with_state(viewer))
        .layer(middleware::from_fn_with_state(cut_signal, cut_at_stop))
        .layer(gateway.cors());

    let listener = TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind))?;
    let addr = listener.local_addr().context("the listener has no address")?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
            .with_graceful_shutdown(async move {
                // Both a signal and a dropped sender stop the server.
                let _ = stopped.await;
            })
            .await
    });
    tracing::info!(%addr, "server started");
    Ok(Server {
        addr,
        sim,
        dev,
        world,
        gateway,
        runtime,
        grace: config.shutdown_grace,
        stop,
        cut,
        task,
    })
}

impl Server {
    /// Stops the HTTP server and the simulation.
    ///
    /// The server takes no new connection. The gateway ends its long
    /// calls and its WebSocket connections. Requests that run get the
    /// grace time of the configuration to end. A request that still runs
    /// after that gets the answer `shutdown`, and its connection gets the
    /// grace time once more to take that answer. Then the simulation
    /// stops, and the feeds of the viewers end with it. The journal
    /// writes a final snapshot, so the next start replays nothing.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP server or the simulation thread failed.
    pub async fn stop(self) -> anyhow::Result<()> {
        self.stop_with_report().await.map(|_| ())
    }

    /// Stops the server like [`Server::stop`] and says what the world was
    /// at the stop.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP server or the simulation thread failed.
    pub async fn stop_with_report(self) -> anyhow::Result<Stopped> {
        self.gateway.shutdown();
        // The receiver is gone only when the server task already ended.
        let _ = self.stop.send(());
        let mut task = self.task;
        let joined = match tokio::time::timeout(self.grace, &mut task).await {
            Ok(joined) => joined,
            Err(_) => {
                // A client that is slow, or one that does not close.
                self.cut.send_replace(true);
                match tokio::time::timeout(self.grace, &mut task).await {
                    Ok(joined) => joined,
                    Err(_) => {
                        task.abort();
                        task.await
                    }
                }
            }
        };
        match joined {
            Ok(result) => result.context("the HTTP server failed")?,
            Err(error) if error.is_cancelled() => {}
            Err(error) => return Err(error).context("the HTTP server task failed"),
        }
        drop(self.sim);
        // The stop of the runtime writes the final snapshot. It waits for
        // the disk, so it runs on a thread that may block.
        let runtime = self.runtime;
        let world = tokio::task::spawn_blocking(move || runtime.shutdown())
            .await
            .context("the stop of the simulation failed")?
            .map_err(|panic| anyhow::anyhow!("the simulation stopped with an error: {panic}"))?;
        Ok(Stopped {
            frame: world.frame(),
            state_hash: hash_text(&world.state_hash()),
            residents: world.residents(),
        })
    }
}
