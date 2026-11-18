//! Identity providers of the gateway.
//!
//! [`GitHubSignIn`] signs an owner in with a GitHub account. It speaks the
//! authorization code flow of OAuth 2.0 with a state value and a proof key
//! (PKCE). It is off unless the configuration has a client id and a client
//! secret.

mod github;
mod pending;

pub use self::github::{ClientSecret, GitHubConfig, GitHubSignIn};
