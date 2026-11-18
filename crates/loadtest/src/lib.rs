//! Load generation and benchmarks for Unsupervised.
//!
//! The crate gives three commands in one binary (`unsup-loadtest`):
//!
//! - [`bench`]: one process, no network. It measures frame time, the rate
//!   of actions and memory for worlds of several sizes.
//! - [`soak`]: one process, a long run. It proves conservation of money
//!   and goods and determinism for the full game, and fails at the first
//!   violation.
//! - [`swarm`]: many virtual agents over the network against a server. It
//!   measures the acknowledgement latency, the throughput and the errors.
//!
//! All three use the same behaviour [`profile`]s and write the same
//! [`report`] format. They always run the full game: the world comes from
//! `unsup_server::build_world` and the server from `unsup_server::start`.
//!
//! `docs/performance.md` says how to run each command and what each number
//! means.

pub mod bench;
pub mod defaults;
pub mod local;
pub mod machine;
pub mod process;
pub mod profile;
pub mod random;
pub mod report;
pub mod scene;
pub mod soak;
pub mod stats;
pub mod swarm;
pub mod think;
