//! Accounts, API keys, webhooks and settings.
//!
//! The gateway uses this crate to know who makes a request. It talks to
//! the [`Store`] trait, so the storage engine can change without a change
//! to the gateway. Two engines exist:
//!
//! - [`SqliteStore`] keeps the data in one SQLite file. The schema lives in
//!   this crate and migrates itself when the file is opened.
//! - [`MemoryStore`] keeps the data in memory, for tests and for the
//!   development mode of the server.
//!
//! # Tokens
//!
//! An owner token and a resident key are random: 32 bytes from the
//! operating system, written as text with a prefix that names the kind.
//! The server shows a token once, when it makes it. The store keeps only
//! the SHA-256 hash, so a copy of the database does not give access. See
//! [`Token`].
//!
//! The signing secret of a webhook is different. The server needs the
//! secret itself to sign a delivery, so the store keeps it as it is.
//!
//! # Identity
//!
//! An [`IdentityProvider`] says who signs up. [`LocalSignUp`] is the only
//! provider in this crate: every sign-up makes a new owner, and the owner
//! token is the only credential. A provider that asks an OAuth service
//! implements the same trait.

mod error;
mod identity;
mod memory;
mod model;
mod sqlite;
mod store;
mod token;

pub use error::StoreError;
pub use identity::{
    Identity, IdentityError, IdentityFuture, IdentityProvider, LocalSignUp, SignUp,
};
pub use memory::MemoryStore;
pub use model::{
    NewOwner, NewResident, Owner, Principal, ResidentRecord, StoreCounts, Webhook, unix_now,
};
pub use sqlite::SqliteStore;
pub use store::Store;
pub use token::{Secret, Token, TokenHash, TokenKind, constant_time_eq, random_hex};
