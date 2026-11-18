//! The configuration of the gateway.
//!
//! Every default is in `data/defaults.toml`, which is part of the binary.
//! A caller gives only the values that it wants to change.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Deserializer};
use unsup_store::TokenHash;

/// The defaults of the gateway, as TOML text.
pub const DEFAULTS: &str = include_str!("../data/defaults.toml");

/// Least length of an admin token, in bytes.
const ADMIN_TOKEN_MIN: usize = 16;

/// A configuration that the gateway cannot use.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the gateway configuration is not valid: {0}")]
pub struct ConfigError(pub String);

/// The admin token of the configuration. The value keeps only the hash of
/// the token, so the token cannot leak through a log or a debug print.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdminToken(TokenHash);

impl AdminToken {
    /// Takes the token of the configuration.
    ///
    /// # Errors
    ///
    /// Fails when the token has fewer than 16 bytes.
    pub fn new(token: &str) -> Result<AdminToken, ConfigError> {
        if token.len() < ADMIN_TOKEN_MIN {
            return Err(ConfigError(format!(
                "the admin token must have at least {ADMIN_TOKEN_MIN} bytes"
            )));
        }
        Ok(AdminToken(TokenHash::of(token)))
    }

    /// Returns true when `presented` is the admin token. The comparison
    /// takes the same time for every wrong token.
    #[must_use]
    pub fn matches(&self, presented: &str) -> bool {
        self.0.matches(&TokenHash::of(presented))
    }
}

impl fmt::Debug for AdminToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdminToken(<hidden>)")
    }
}

impl<'de> Deserialize<'de> for AdminToken {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let token = String::deserialize(deserializer)?;
        AdminToken::new(&token).map_err(|e| serde::de::Error::custom(e.0))
    }
}

/// The configuration of the gateway. `GatewayConfig::default()` has the
/// values of `data/defaults.toml`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    /// The token of the administrator. Without it, every admin route
    /// answers `admin_disabled`.
    #[serde(default)]
    pub admin_token: Option<AdminToken>,
    /// The file that `GET /join.md` serves. Without it, or when the file
    /// does not exist, the route answers 404.
    #[serde(default)]
    pub join_file: Option<PathBuf>,
    /// Origins that may call the API from a browser. `*` allows all.
    pub cors_origins: Vec<String>,
    /// Number of trusted reverse proxies in front of the server.
    pub forwarded_hops: u8,
    /// Limits for requests.
    pub limits: Limits,
    /// The WebSocket for agents.
    pub ws: WsConfig,
    /// The MCP endpoint.
    pub mcp: McpConfig,
    /// Webhook deliveries.
    pub webhooks: WebhookConfig,
}

/// Limits for requests. A value of 0 turns off a rate limit, the limit
/// of residents of one owner and the limit of open `wait` calls. It does
/// not turn off a size or the greatest length of a list.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    /// Greatest number of residents of one owner that are in the world.
    pub residents_per_owner: u32,
    /// Requests per minute of one owner token or one resident key.
    pub key_requests_per_minute: u32,
    /// Requests that a key can send at once after a quiet time.
    pub key_burst: u32,
    /// Requests per minute of one client address.
    pub address_requests_per_minute: u32,
    /// Requests that an address can send at once after a quiet time.
    pub address_burst: u32,
    /// Sign-ups per hour of one client address.
    pub signups_per_hour: u32,
    /// Sign-ups that an address can send at once after a quiet time.
    pub signup_burst: u32,
    /// Greatest size of a request body, in bytes.
    pub max_body_bytes: usize,
    /// Greatest number of requests in one plan.
    pub max_plan: usize,
    /// Greatest time of one `wait` call, in seconds.
    pub max_wait_secs: u64,
    /// Time of a `wait` call that names none, in seconds.
    pub default_wait_secs: u64,
    /// Greatest number of open `wait` calls of one resident.
    pub waits_per_resident: u32,
    /// Greatest number of names in `sections` and in `kinds`.
    pub max_names: usize,
}

/// The WebSocket for agents.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsConfig {
    /// Time between two pings of the server, in milliseconds.
    pub ping_interval_ms: u64,
    /// The server closes a connection that sent nothing for this long.
    pub idle_timeout_ms: u64,
    /// Greatest number of frames that wait for a slow client.
    pub queue: usize,
    /// Greatest size of one frame from a client, in bytes.
    pub max_message_bytes: usize,
    /// Greatest number of connections of one resident at a time.
    pub connections_per_resident: u32,
}

impl WsConfig {
    /// Time between two pings of the server.
    #[must_use]
    pub fn ping_interval(&self) -> Duration {
        Duration::from_millis(self.ping_interval_ms.max(1))
    }

    /// Time without a frame after which the server closes a connection.
    #[must_use]
    pub fn idle_timeout(&self) -> Duration {
        Duration::from_millis(self.idle_timeout_ms)
    }
}

/// The MCP endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    /// The server forgets a session that was idle for this many seconds.
    pub session_idle_secs: u64,
    /// Greatest number of sessions.
    pub max_sessions: usize,
    /// Greatest time of the tool `wait`, in seconds.
    pub max_wait_secs: u64,
    /// Time of the tool `wait` when the call names none, in seconds.
    pub default_wait_secs: u64,
    /// Greatest length of a line in a tool result.
    pub line_width: usize,
    /// Time for which a client can keep the tool list, in milliseconds.
    pub list_ttl_ms: u64,
}

/// Webhook deliveries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookConfig {
    /// True: a webhook can point at a private or loopback address.
    pub allow_private: bool,
    /// Greatest length of a webhook address, in bytes.
    pub max_url_bytes: usize,
    /// Time limit of one delivery attempt, in milliseconds.
    pub timeout_ms: u64,
    /// Number of attempts for one delivery.
    pub attempts: u32,
    /// Wait before the second attempt, in milliseconds. It doubles with
    /// each attempt.
    pub backoff_base_ms: u64,
    /// Greatest wait between two attempts, in milliseconds.
    pub backoff_max_ms: u64,
    /// Greatest number of deliveries that wait for a free worker.
    pub queue: usize,
    /// Number of deliveries that are sent at the same time.
    pub workers: usize,
}

impl WebhookConfig {
    /// Time limit of one delivery attempt.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }

    /// The rest of a webhook after the last attempt of a delivery failed.
    #[must_use]
    pub fn rest(&self) -> Duration {
        Duration::from_millis(self.backoff_max_ms)
    }

    /// The wait after the attempt with the given number, from 1.
    #[must_use]
    pub fn backoff(&self, attempt: u32) -> Duration {
        let doubled = self
            .backoff_base_ms
            .saturating_mul(1u64.checked_shl(attempt.saturating_sub(1)).unwrap_or(u64::MAX));
        Duration::from_millis(doubled.min(self.backoff_max_ms))
    }
}

impl Default for GatewayConfig {
    fn default() -> GatewayConfig {
        // The file is part of the binary, and a test reads it.
        GatewayConfig::from_table(toml::Table::new())
            .expect("the defaults in data/defaults.toml are a complete configuration")
    }
}

impl GatewayConfig {
    /// Makes a configuration from the defaults and the values in
    /// `overrides`, which has the shape of `data/defaults.toml`.
    ///
    /// # Errors
    ///
    /// Fails when `overrides` has a key that the gateway does not know or
    /// a value of the wrong type.
    pub fn from_table(overrides: toml::Table) -> Result<GatewayConfig, ConfigError> {
        let mut table: toml::Table =
            toml::from_str(DEFAULTS).map_err(|e| ConfigError(e.to_string()))?;
        merge_tables(&mut table, overrides);
        table.try_into().map_err(|e: toml::de::Error| ConfigError(e.message().to_string()))
    }

    /// Makes a configuration from the defaults and TOML text.
    ///
    /// # Errors
    ///
    /// Fails when the text is not TOML, or as [`GatewayConfig::from_table`].
    pub fn from_toml(overrides: &str) -> Result<GatewayConfig, ConfigError> {
        let overrides: toml::Table =
            toml::from_str(overrides).map_err(|e| ConfigError(e.message().to_string()))?;
        GatewayConfig::from_table(overrides)
    }

    /// Returns true when a browser page of `origin` may call the API.
    #[must_use]
    pub fn allows_origin(&self, origin: &str) -> bool {
        self.cors_origins.iter().any(|allowed| allowed == "*" || allowed == origin)
    }
}

/// Puts every value of `over` into `base`. A table merges with the table
/// of the same name; every other value replaces the old one.
pub fn merge_tables(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(old)), toml::Value::Table(new)) => merge_tables(old, new),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}
