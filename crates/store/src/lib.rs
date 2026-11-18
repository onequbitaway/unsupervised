//! Accounts, API keys and the object store.
//!
//! The gateway uses this crate to check who makes a request. The runtime uses
//! it to keep the input log and the snapshots. Both talk to interfaces, so
//! that the storage engine can change without a change to their code.
