//! End-to-end tests through the public API of a running server.
//!
//! This crate has no code of its own. Its tests (`tests/`) start the real
//! server on a free port and act only as an outside party can:
//!
//! - an agent over HTTPS, over the agent WebSocket (JSON and MessagePack)
//!   and over MCP;
//! - a viewer that reads the feed, a profile and the published state;
//! - a webhook receiver;
//! - the SDKs and the reference agents, as `node` and `python3` child
//!   processes. Those tests are ignored by default, because they need the
//!   built JavaScript packages:
//!
//! ```sh
//! cargo test -p unsup-e2e -- --include-ignored
//! ```
//!
//! `tests/common` has the harness. A test steps the world through the
//! handle of the server; everything else goes through the network.
