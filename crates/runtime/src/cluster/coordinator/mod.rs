//! The coordinator process: it hosts the global realm, owns the frame
//! clock and the directory, and routes every message between realms.

mod begin;
mod core;
mod frame;
mod inputs;
mod numbers;
mod reads;
mod serve;
mod waits;

use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use std::time::{Duration, Instant};

use self::begin::open_global;
use self::core::{Core, Routed, unexpected};
use self::serve::{Server, stop_all};
use super::config::ClusterConfig;
use super::error::ClusterError;
use super::fingerprint::{Fingerprint, fingerprint};
use super::handle::{ClusterHandle, ClusterJoin};
use super::link::{Alarm, Link};
use super::lookup::Lookup;
use super::proto::{FromWorker, PROTOCOL_VERSION, ToWorker, ZoneState};
use crate::durable::Plugins;
use crate::settings::settings;

/// A coordinator that listens for its workers.
///
/// [`Coordinator::bind`] opens the socket, so that the address is known
/// before a worker starts. [`Coordinator::start`] waits for the workers
/// and starts the cluster.
#[derive(Debug)]
pub struct Coordinator {
    listener: TcpListener,
}

/// A worker that passed the first step of the handshake.
struct Joined {
    link: Link,
    states: Vec<ZoneState>,
}

/// Why a worker is refused, or `None`.
fn refusal(hello: &FromWorker, own: &Fingerprint, taken: &[Option<Joined>]) -> Result<u32, String> {
    let FromWorker::Hello { protocol, world, plugins, slot } = hello else {
        return Err(format!("the first message must be `Hello`, not `{}`", hello.name()));
    };
    if *protocol != PROTOCOL_VERSION {
        return Err(format!(
            "the worker speaks version {protocol} of the cluster protocol, the coordinator version {PROTOCOL_VERSION}"
        ));
    }
    if *world != own.world {
        return Err("the world configuration of the worker is not the one of the coordinator \
                    (seed, cities, zone size, days per year, data or audit interval)"
            .into());
    }
    if *plugins != own.plugins {
        return Err("the set of plugins of the worker is not the one of the coordinator \
                    (names, order, verbs, parameters, items, attributes or conditions)"
            .into());
    }
    match taken.get(*slot as usize) {
        None => {
            Err(format!("the cluster has {} worker slots, there is no slot {slot}", taken.len()))
        }
        Some(Some(_)) => Err(format!("another worker has slot {slot}")),
        Some(None) => Ok(*slot),
    }
}

impl Coordinator {
    /// Opens the socket on which the workers connect. Use port 0 to let
    /// the operating system choose a free port.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Io`] when the address cannot be used.
    pub fn bind(address: impl ToSocketAddrs) -> Result<Coordinator, ClusterError> {
        let io = |e: std::io::Error| ClusterError::Io(e.to_string());
        let listener = TcpListener::bind(address).map_err(io)?;
        // `start` looks for new connections and for the time limit in
        // turns.
        listener.set_nonblocking(true).map_err(io)?;
        Ok(Coordinator { listener })
    }

    /// Returns the address on which the workers connect.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Io`] when the operating system does not tell.
    pub fn local_addr(&self) -> Result<SocketAddr, ClusterError> {
        self.listener.local_addr().map_err(|e| ClusterError::Io(e.to_string()))
    }

    /// Waits until a worker has joined for every slot. A connection that
    /// fails the handshake is refused with the reason, and its slot stays
    /// free.
    fn assemble(
        &self,
        own: &Fingerprint,
        config: &ClusterConfig,
        alarm: &Alarm,
    ) -> Result<Vec<Joined>, ClusterError> {
        let s = &settings().cluster;
        let deadline = Instant::now() + config.join_timeout;
        let mut joined: Vec<Option<Joined>> = config.workers.iter().map(|_| None).collect();
        while joined.iter().any(Option::is_none) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                let missing = joined
                    .iter()
                    .enumerate()
                    .filter(|(_, worker)| worker.is_none())
                    .map(|(slot, _)| slot as u32)
                    .collect();
                let error = ClusterError::JoinTimeout { missing };
                for worker in joined.iter_mut().flatten() {
                    stop_all(std::slice::from_mut(&mut worker.link), &error);
                }
                return Err(error);
            }
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(s.retry_ms).min(left));
                    continue;
                }
                Err(error) => return Err(ClusterError::Io(error.to_string())),
            };
            let mut link = match Link::start(stream, s.max_frame_bytes, alarm.clone()) {
                Ok(link) => link,
                Err(error) => {
                    tracing::warn!(%error, "a connection could not be used");
                    continue;
                }
            };
            // A connection that says nothing must not hold the others up
            // for longer than the time that is left.
            let hello = match link.recv(left) {
                Ok(hello) => hello,
                Err(error) => {
                    tracing::warn!(%error, "a connection ended before the handshake");
                    continue;
                }
            };
            let slot = match refusal(&hello, own, &joined) {
                Ok(slot) => slot,
                Err(reason) => {
                    tracing::warn!(%reason, "a worker was refused");
                    let _ = link.send(&ToWorker::Refuse { reason });
                    continue;
                }
            };
            link.slot = slot;
            let zones = config.workers[slot as usize].clone();
            let states = link.send(&ToWorker::Welcome { zones: zones.clone() }).and_then(|()| {
                match link.recv(left)? {
                    FromWorker::State { zones: states }
                        if states.iter().map(|state| state.zone).eq(zones.iter().copied()) =>
                    {
                        Ok(states)
                    }
                    other => Err(unexpected(slot as usize, "State", &other)),
                }
            });
            // A worker that fails after its welcome, for example on its
            // storage, ends the start: the error says why.
            let states = states?;
            joined[slot as usize] = Some(Joined { link, states });
        }
        Ok(joined.into_iter().flatten().collect())
    }

    /// Waits for the workers, makes or recovers the worlds and starts the
    /// cluster.
    ///
    /// The call returns when every worker has joined and every realm is
    /// ready: after world generation and the first exchange of a new
    /// world, or after the recovery of a stored one to the newest seal
    /// that all realms have.
    ///
    /// # Errors
    ///
    /// [`ClusterError::Config`] for a configuration that is not valid.
    /// [`ClusterError::JoinTimeout`] when a slot stays free.
    /// [`ClusterError::Recovery`] when the stored state of the processes
    /// has no common seal or cannot be read. The other errors mean that a
    /// worker failed while the cluster started.
    pub fn start(
        self,
        plugins: Plugins,
        config: ClusterConfig,
    ) -> Result<(ClusterHandle, ClusterJoin), ClusterError> {
        let slot_of = config.zone_slots()?;
        let own = fingerprint(&plugins, &config.world)?;
        let alarm = Alarm::default();
        let joined = self.assemble(&own, &config, &alarm)?;
        // No worker joins a cluster that runs.
        drop(self.listener);
        let (mut links, states): (Vec<Link>, Vec<Vec<ZoneState>>) =
            joined.into_iter().map(|worker| (worker.link, worker.states)).unzip();
        for link in &links {
            link.limit_writes(config.reply_timeout).map_err(|e| ClusterError::Io(e.to_string()))?;
        }

        // A failure from here on ends the workers that joined. They learn
        // the reason.
        let (global, recover) = match open_global(&plugins, &config, &states) {
            Ok(opened) => opened,
            Err(error) => {
                stop_all(&mut links, &error);
                return Err(error);
            }
        };
        let workers = links.len();
        let mut core = Core {
            global,
            links,
            zones_of: config.workers.clone(),
            residents: vec![0; slot_of.len()],
            slot_of,
            spawn_zone: config.spawn_zone,
            lookup: Lookup::default(),
            reply_timeout: config.reply_timeout,
            next_epoch: recover.map_or(0, |epoch| epoch + 1),
            dirty: false,
            dropped: vec![0; workers],
            journals: vec![Default::default(); workers],
            routed: Routed::default(),
        };
        // A recovery replays the log. It can take longer than one answer.
        let patience = config.join_timeout.max(config.reply_timeout);
        if let Err(error) = core.begin(recover, patience) {
            stop_all(&mut core.links, &error);
            return Err(error);
        }
        Server::spawn(core, config, alarm)
    }
}
