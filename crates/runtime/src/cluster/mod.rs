//! The cluster: one world, its zones spread over several processes.
//!
//! A *coordinator* process hosts the global realm, owns the frame clock
//! and routes every message between realms. *Worker* processes each host
//! a set of zones. They talk over TCP. The cluster runs the frames in
//! lock-step and gives every realm its messages in the order of the single
//! process, so the state of every realm is the state that one process
//! reaches with the same inputs.
//!
//! ```no_run
//! use std::sync::Arc;
//! use unsup_kernel::{Plugin, WorldConfig};
//! use unsup_runtime::cluster::{ClusterConfig, Coordinator, WorkerConfig, run_worker};
//!
//! # fn plugins() -> Arc<Vec<Box<dyn Plugin>>> { Arc::new(Vec::new()) }
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let world = WorldConfig::with_seed(1);
//! let coordinator = Coordinator::bind("127.0.0.1:0")?;
//! let address = coordinator.local_addr()?;
//! // Each worker is a process of its own. Here they are threads.
//! for slot in 0..3 {
//!     let config = WorkerConfig::new(world.clone(), slot, address);
//!     std::thread::spawn(move || run_worker(plugins(), config));
//! }
//! let (handle, join) = coordinator.start(plugins(), ClusterConfig::manual(world, 3))?;
//! // `handle` has the methods of `SimHandle`.
//! # drop(handle);
//! join.shutdown()?;
//! # Ok(())
//! # }
//! ```
//!
//! # Who can join
//!
//! A cluster has a shared secret ([`ClusterSecret`]). A worker proves that
//! it knows the secret before the coordinator reads anything else from it.
//! Give the secret to [`Coordinator::bind_with_secret`] and to
//! [`WorkerConfig::with_secret`]. Without a secret, a coordinator listens
//! on a loopback address only.
//!
//! The connections have no encryption, and the messages after the
//! handshake are not signed. Run a cluster on a private network.
//!
//! `README.md` in the crate directory describes the topology, the
//! handshake, the frame protocol, the failure rules, the recovery and the
//! limits.

mod auth;
pub(crate) mod call;
mod cell;
mod config;
mod coordinator;
mod error;
mod fingerprint;
mod handle;
#[cfg(test)]
mod handshake_tests;
mod link;
mod lookup;
mod plan;
mod proto;
mod scoped;
mod seals;
mod session;
mod stats;
mod totals;
mod wire;
mod worker;

pub use self::auth::ClusterSecret;
pub use self::config::{ClusterConfig, ClusterDurable, WorkerConfig, partition};
pub use self::coordinator::Coordinator;
pub use self::error::ClusterError;
pub use self::handle::{ClusterHandle, ClusterJoin};
pub use self::stats::{ClusterStats, WorkerStats};
pub use self::totals::{ClusterTotals, ValueTotals};
pub use self::worker::{Worker, WorkerAbort, WorkerExit, run_worker};
