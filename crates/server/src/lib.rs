//! The composition root of Unsupervised.
//!
//! This crate puts the parts together: the game modules, the world, the
//! runtime, the gateway for agents and the feed for viewers. The
//! `unsupervised` binary is a thin command-line front for it.

pub mod plugins;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tower_http::services::ServeDir;
use unsup_kernel::{CityConfig, Pos, World, WorldConfig};
use unsup_runtime::{RuntimeConfig, RuntimeJoin, SimHandle};

pub use plugins::plugins;

/// Shape of the world that a new server creates.
#[derive(Clone, Debug)]
pub struct WorldShape {
    /// Seed of all randomness.
    pub seed: u64,
    /// Number of cities.
    pub cities: u32,
    /// Districts on one side of a city. A city has `districts * districts` zones.
    pub districts: u32,
}

impl Default for WorldShape {
    fn default() -> WorldShape {
        WorldShape { seed: 1, cities: 2, districts: 3 }
    }
}

impl WorldShape {
    /// Returns the kernel configuration for this shape. Cities stand in a
    /// row with a gap between them.
    #[must_use]
    pub fn to_config(&self) -> WorldConfig {
        let mut config = WorldConfig::with_seed(self.seed);
        let side = i64::from(self.districts) * i64::from(config.zone_size);
        let pitch = side + i64::from(config.zone_size);
        config.cities = (0..self.cities)
            .map(|i| {
                let x = i32::try_from(i64::from(i) * pitch).unwrap_or(i32::MAX);
                CityConfig::new(city_name(i), Pos::new(x, 0), self.districts)
            })
            .collect();
        config
    }
}

/// Returns the name of a city by number. The names are fixed so that the
/// same seed always gives the same world.
fn city_name(index: u32) -> String {
    const NAMES: [&str; 12] = [
        "Latent City",
        "Tokentown",
        "Gradient Falls",
        "Port Epoch",
        "New Tensor",
        "Softmax Springs",
        "Fort Inference",
        "Attention Heights",
        "Overfit Hollow",
        "Kernel Bay",
        "Mount Entropy",
        "Bias Harbor",
    ];
    let base = NAMES[index as usize % NAMES.len()];
    let round = index as usize / NAMES.len();
    if round == 0 { base.to_string() } else { format!("{base} {}", round + 1) }
}

/// Builds a new world with all game modules.
#[must_use]
pub fn build_world(config: WorldConfig) -> World {
    let plugins = plugins();
    let refs: Vec<&dyn unsup_kernel::Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    World::new(&refs, config)
}

/// Configuration of a server.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Address to listen on. Port 0 picks a free port.
    pub bind: SocketAddr,
    /// Real time between two frames. Zero means manual stepping.
    pub tick_interval: Duration,
    /// Shape of the world.
    pub world: WorldShape,
    /// Directory with the built viewer. It is served at `/` when it exists.
    pub web_dir: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> ServerConfig {
        ServerConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], 8080)),
            tick_interval: Duration::from_secs(1),
            world: WorldShape::default(),
            web_dir: Some(PathBuf::from("web/dist")),
        }
    }
}

/// A running server.
pub struct Server {
    /// The address that the server listens on.
    pub addr: SocketAddr,
    /// Handle to the simulation, for tools and tests.
    pub sim: SimHandle,
    runtime: RuntimeJoin,
    stop: oneshot::Sender<()>,
    task: JoinHandle<std::io::Result<()>>,
}

/// Starts the simulation and the HTTP server.
///
/// # Errors
///
/// Fails when the address cannot be bound.
pub async fn start(config: ServerConfig) -> anyhow::Result<Server> {
    let world = build_world(config.world.to_config());
    let runtime_config =
        RuntimeConfig { tick_interval: config.tick_interval, ..RuntimeConfig::default() };
    let (sim, runtime) = unsup_runtime::start(world, runtime_config);

    let mut app: Router =
        unsup_gateway::router(sim.clone()).merge(unsup_spectator::router(sim.clone()));
    if let Some(dir) = config.web_dir.filter(|dir| dir.is_dir()) {
        app = app.fallback_service(ServeDir::new(dir));
    }

    let listener = TcpListener::bind(config.bind)
        .await
        .with_context(|| format!("cannot listen on {}", config.bind))?;
    let addr = listener.local_addr().context("the listener has no address")?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                // Both a signal and a dropped sender stop the server.
                let _ = stopped.await;
            })
            .await
    });
    tracing::info!(%addr, "server started");
    Ok(Server { addr, sim, runtime, stop, task })
}

impl Server {
    /// Stops the HTTP server and the simulation.
    ///
    /// # Errors
    ///
    /// Fails when the HTTP server or the simulation thread failed.
    pub async fn stop(self) -> anyhow::Result<()> {
        // The receiver is gone only when the server task already ended.
        let _ = self.stop.send(());
        drop(self.sim);
        // Open viewer connections would keep a graceful shutdown waiting.
        self.task.abort();
        match self.task.await {
            Ok(result) => result.context("the HTTP server failed")?,
            Err(error) if error.is_cancelled() => {}
            Err(error) => return Err(error).context("the HTTP server task failed"),
        }
        self.runtime
            .shutdown()
            .map(|_world| ())
            .map_err(|panic| anyhow::anyhow!("the simulation stopped with an error: {panic}"))
    }
}
