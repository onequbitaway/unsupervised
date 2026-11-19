//! A worker process: it hosts a set of zones and does what the
//! coordinator asks.
//!
//! The worker has one world for each of its zones. It reads one message of
//! the coordinator, does the work and sends one answer. The zones of a
//! worker tick in parallel.

use std::net::{Shutdown, TcpStream};
use std::time::{Duration, Instant};

use unsup_kernel::World;
use unsup_protocol::ZoneId;

use super::auth::{self, ClusterSecret};
use super::config::WorkerConfig;
use super::error::ClusterError;
use super::fingerprint::fingerprint;
use super::proto::{FromWorker, PROTOCOL_VERSION, ToWorker};
use super::session::Session;
use super::wire::{WireError, read_frame, write_frame};
use crate::durable::Plugins;
use crate::settings::settings;

/// A worker that the coordinator has accepted. [`Worker::run`] serves the
/// zones until the cluster stops.
pub struct Worker {
    stream: TcpStream,
    zones: Vec<ZoneId>,
    plugins: Plugins,
    config: WorkerConfig,
    max_frame: u32,
}

impl std::fmt::Debug for Worker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Worker")
            .field("slot", &self.config.slot)
            .field("zones", &self.zones)
            .finish_non_exhaustive()
    }
}

/// Cuts the connection of a worker, as the death of its process does.
///
/// The worker then stops with [`ClusterError::CoordinatorLost`] and the
/// coordinator stops the cluster. A supervisor can use it to take a worker
/// out; tests use it to imitate a process that dies.
#[derive(Debug)]
pub struct WorkerAbort {
    stream: TcpStream,
}

impl WorkerAbort {
    /// Cuts the connection now.
    pub fn abort(&self) {
        // The connection may be closed already. That is the same result.
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

/// What a worker gives back after a stop in order.
#[derive(Debug)]
pub struct WorkerExit {
    /// The worlds of the zones of the worker, in zone order. Each world
    /// hosts one zone.
    pub worlds: Vec<World>,
}

/// Maps a failure of the connection to the coordinator.
pub(crate) fn lost(error: &WireError) -> ClusterError {
    ClusterError::CoordinatorLost { reason: error.to_string() }
}

impl Worker {
    /// Connects to the coordinator and does the first steps of the
    /// handshake: both sides prove that they know the shared secret of the
    /// cluster, then the coordinator checks the protocol version, the
    /// world configuration, the set of plugins and the slot.
    ///
    /// The worker tries to connect until `connect_timeout` has passed, so
    /// it can start before the coordinator.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Config`] when the world configuration or the data
    /// of a plugin is not valid, and when the coordinator is not on a
    /// loopback address and the worker has no secret. [`ClusterError::Io`]
    /// when no connection could be made in time. [`ClusterError::Refused`]
    /// when the coordinator refused the worker; the reason says what
    /// differs, or that the secret is not the one of the coordinator.
    /// [`ClusterError::Protocol`] when the coordinator does not prove that
    /// it knows the secret.
    pub fn connect(plugins: Plugins, config: WorkerConfig) -> Result<Worker, ClusterError> {
        if config.secret.is_none() && !config.coordinator.ip().is_loopback() {
            return Err(ClusterError::Config(format!(
                "the coordinator at {} is not on a loopback address, and this worker has no \
                 shared secret; give it the secret of the cluster",
                config.coordinator
            )));
        }
        let secret = ClusterSecret::or_open(config.secret.as_ref());
        let print = fingerprint(&plugins, &config.world)?;
        let s = &settings().cluster;
        let retry = Duration::from_millis(s.retry_ms);
        let deadline = Instant::now() + config.connect_timeout;
        let mut stream = loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match TcpStream::connect_timeout(&config.coordinator, left.max(retry)) {
                Ok(stream) => break stream,
                Err(error) if Instant::now() + retry >= deadline => {
                    return Err(ClusterError::Io(format!(
                        "no connection to the coordinator at {}: {error}",
                        config.coordinator
                    )));
                }
                // The coordinator does not listen yet.
                Err(_) => std::thread::sleep(retry),
            }
        };
        let io = |e: std::io::Error| ClusterError::Io(e.to_string());
        stream.set_nodelay(true).map_err(io)?;
        stream.set_read_timeout(Some(config.reply_timeout)).map_err(io)?;
        // The proof of the shared secret comes before everything else.
        auth::present(&mut stream, &secret)?;
        let hello = FromWorker::Hello {
            protocol: PROTOCOL_VERSION,
            world: print.world,
            plugins: print.plugins,
            slot: config.slot,
        };
        write_frame(&mut stream, &hello, s.max_frame_bytes).map_err(|e| lost(&e))?;
        let zones = match read_frame::<_, ToWorker>(&mut stream, s.max_frame_bytes) {
            Ok((ToWorker::Welcome { zones }, _)) => zones,
            Ok((ToWorker::Refuse { reason }, _)) => return Err(ClusterError::Refused { reason }),
            Ok(_) => {
                return Err(ClusterError::Protocol(
                    "the coordinator did not answer the handshake with a welcome".into(),
                ));
            }
            Err(error) => return Err(lost(&error)),
        };
        // From here on the coordinator decides the pace. A worker waits as
        // long as the coordinator has nothing to ask.
        stream.set_read_timeout(None).map_err(io)?;
        Ok(Worker { stream, zones, plugins, config, max_frame: s.max_frame_bytes })
    }

    /// Returns the zones that the coordinator gave this worker.
    #[must_use]
    pub fn zones(&self) -> &[ZoneId] {
        &self.zones
    }

    /// Returns a value that cuts the connection of this worker.
    ///
    /// # Errors
    ///
    /// Fails when the operating system cannot make a second handle to the
    /// socket.
    pub fn abort_handle(&self) -> Result<WorkerAbort, ClusterError> {
        self.stream
            .try_clone()
            .map(|stream| WorkerAbort { stream })
            .map_err(|e| ClusterError::Io(e.to_string()))
    }

    /// Makes or recovers the worlds of the zones and serves the
    /// coordinator until the cluster stops.
    ///
    /// # Errors
    ///
    /// Every stop that is not in order is an error: the coordinator is
    /// lost ([`ClusterError::CoordinatorLost`]), the coordinator stopped
    /// the cluster after a failure elsewhere ([`ClusterError::Stopped`]),
    /// a zone of this worker failed ([`ClusterError::Failed`]) or its
    /// stored state cannot be used ([`ClusterError::Recovery`]).
    pub fn run(self) -> Result<WorkerExit, ClusterError> {
        let Worker { stream, zones, plugins, config, max_frame } = self;
        let mut session = Session::new(stream, max_frame);
        match session.serve(&zones, &plugins, &config) {
            Ok(worlds) => Ok(WorkerExit { worlds }),
            Err(error) => {
                // The coordinator learns why, when it still listens.
                if let ClusterError::Failed { realm, message } = &error {
                    session
                        .say(&FromWorker::Failed { realm: Some(*realm), message: message.clone() });
                } else if let ClusterError::Recovery { realm, message } = &error {
                    session.say(&FromWorker::Failed { realm: *realm, message: message.clone() });
                }
                Err(error)
            }
        }
    }
}

/// Connects a worker and serves until the cluster stops:
/// [`Worker::connect`], then [`Worker::run`].
///
/// # Errors
///
/// See [`Worker::connect`] and [`Worker::run`].
pub fn run_worker(plugins: Plugins, config: WorkerConfig) -> Result<WorkerExit, ClusterError> {
    Worker::connect(plugins, config)?.run()
}
