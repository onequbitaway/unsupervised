//! Sign-in with a GitHub account: the authorization code flow of OAuth 2.0.
//!
//! 1. [`IdentityProvider::begin`] makes a state value and the address of
//!    the authorization page. The person opens it and agrees.
//! 2. GitHub sends the browser back with a `code` and the `state`.
//! 3. [`IdentityProvider::identify`] takes both as the credential
//!    `<state>:<code>`. It accepts a state that it made, one time. It
//!    sends the code, the client secret and the proof key to the token
//!    endpoint in the body of a `POST`, and gets an access token. With
//!    that token in a header it asks the user endpoint for the account.
//!
//! No secret is in an address: the client secret and the proof key go in
//! a request body, the access token goes in a header. No secret is in the
//! log: an error names the step that failed and the status, never a
//! value.

use std::fmt;
use std::time::{Duration, Instant};

use reqwest::header::{ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use sha2::{Digest, Sha256};
use unsup_store::{
    Authorization, Identity, IdentityError, IdentityFuture, IdentityProvider, SignUp,
};

use super::pending::PendingStates;
use crate::config::ConfigError;

/// Greatest length of the display name of an owner, in bytes.
const NAME_MAX: usize = 64;
/// Greatest length of a credential, in bytes.
const CREDENTIAL_MAX: usize = 2_048;
/// What the server calls itself when it asks the service. GitHub refuses
/// a request without this header.
const AGENT: &str = concat!("unsupervised/", env!("CARGO_PKG_VERSION"));

/// The client secret of an OAuth application. `Debug` does not print it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ClientSecret(String);

impl ClientSecret {
    /// Wraps a secret.
    #[must_use]
    pub fn new(secret: impl Into<String>) -> ClientSecret {
        ClientSecret(secret.into())
    }

    /// Returns true when no secret is set.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    fn reveal(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ClientSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() {
            "ClientSecret(<none>)"
        } else {
            "ClientSecret(<hidden>)"
        })
    }
}

impl<'de> Deserialize<'de> for ClientSecret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(ClientSecret)
    }
}

/// Sign-in with GitHub. The values come from the `[github]` table of the
/// gateway configuration. The sign-in is off unless `client_id` and
/// `client_secret` are set.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubConfig {
    /// The client id of the OAuth application.
    pub client_id: String,
    /// The client secret of the OAuth application.
    pub client_secret: ClientSecret,
    /// The page where the person agrees.
    pub authorize_url: String,
    /// The endpoint that exchanges a code for an access token.
    pub token_url: String,
    /// The endpoint that returns the account of an access token.
    pub user_url: String,
    /// The address of `GET /v1/auth/github/callback` as the browser of
    /// the person reaches it. Empty: the service uses the address that
    /// the application registered.
    pub redirect_url: String,
    /// The permissions that the server asks for.
    pub scope: String,
    /// Seconds for which a started sign-in is good.
    pub state_ttl_secs: u64,
    /// Greatest number of started sign-ins that wait.
    pub max_pending: usize,
    /// Time limit of one request to the service, in milliseconds.
    pub timeout_ms: u64,
}

impl GitHubConfig {
    /// Returns true when the configuration has a client id and a client
    /// secret: the sign-in is on.
    #[must_use]
    pub fn is_on(&self) -> bool {
        !self.client_id.is_empty() && !self.client_secret.is_empty()
    }
}

/// Reads an endpoint address of the configuration.
fn endpoint(name: &str, text: &str) -> Result<Url, ConfigError> {
    let url = Url::parse(text)
        .map_err(|error| ConfigError(format!("`github.{name}` is not an address: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ConfigError(format!("`github.{name}` must start with http:// or https://")));
    }
    Ok(url)
}

/// Writes pairs as the body of a form (`application/x-www-form-urlencoded`).
fn form_body(pairs: &[(&str, &str)]) -> String {
    let mut encoder = Url::parse("http://form.invalid/").ok();
    if let Some(url) = &mut encoder {
        url.query_pairs_mut().extend_pairs(pairs);
    }
    // The address above is a constant that always parses.
    encoder.and_then(|url| url.query().map(str::to_string)).unwrap_or_default()
}

/// Encodes bytes as Base64 for addresses, without padding.
fn base64_url(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let group = chunk.iter().fold(0u32, |group, byte| (group << 8) | u32::from(*byte))
            << (8 * (3 - chunk.len()));
        for index in 0..=chunk.len() {
            // The mask keeps six bits, so the index is below 64.
            out.push(char::from(DIGITS[((group >> (18 - 6 * index)) & 0x3f) as usize]));
        }
    }
    out
}

/// Cuts a name to the greatest length, at a character boundary.
fn display_name(name: &str) -> String {
    let mut end = name.len().min(NAME_MAX);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

fn unavailable(step: &str) -> IdentityError {
    IdentityError::Unavailable(format!("GitHub does not answer now ({step}). Try again later."))
}

/// The identity provider `github`.
pub struct GitHubSignIn {
    config: GitHubConfig,
    authorize: Url,
    token: Url,
    user: Url,
    /// `None` when the HTTP client could not be built. Every sign-in
    /// then fails as unavailable.
    client: Option<reqwest::Client>,
    pending: PendingStates,
}

impl fmt::Debug for GitHubSignIn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitHubSignIn").field("pending", &self.pending.len()).finish_non_exhaustive()
    }
}

impl GitHubSignIn {
    /// The name of this provider.
    pub const NAME: &'static str = "github";

    /// Makes the provider from the configuration.
    ///
    /// # Errors
    ///
    /// Fails when the configuration has no client id or no client secret,
    /// or when an endpoint is not an `http` or `https` address.
    pub fn new(config: &GitHubConfig) -> Result<GitHubSignIn, ConfigError> {
        if !config.is_on() {
            return Err(ConfigError(
                "`github.client_id` and `github.client_secret` must both be set".into(),
            ));
        }
        if !config.redirect_url.is_empty() {
            endpoint("redirect_url", &config.redirect_url)?;
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms.max(1)))
            // An answer that points elsewhere is an error, not a new target.
            .redirect(reqwest::redirect::Policy::none())
            .build();
        if let Err(error) = &client {
            tracing::error!(%error, "cannot build the HTTP client for the GitHub sign-in");
        }
        Ok(GitHubSignIn {
            authorize: endpoint("authorize_url", &config.authorize_url)?,
            token: endpoint("token_url", &config.token_url)?,
            user: endpoint("user_url", &config.user_url)?,
            client: client.ok(),
            pending: PendingStates::new(
                Duration::from_secs(config.state_ttl_secs),
                config.max_pending,
            ),
            config: config.clone(),
        })
    }

    /// Exchanges the code for an access token.
    async fn access_token(&self, code: &str, verifier: &str) -> Result<String, IdentityError> {
        let client = self.client.as_ref().ok_or_else(|| unavailable("no HTTP client"))?;
        let mut pairs = vec![
            ("grant_type", "authorization_code"),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.reveal()),
            ("code", code),
            ("code_verifier", verifier),
        ];
        if !self.config.redirect_url.is_empty() {
            pairs.push(("redirect_uri", &self.config.redirect_url));
        }
        let response = client
            .post(self.token.clone())
            .header(ACCEPT, "application/json")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(USER_AGENT, AGENT)
            .body(form_body(&pairs))
            .send()
            .await
            .map_err(|error| {
                // The error can name the address. It holds no secret.
                tracing::warn!(error = %error.without_url(), "the GitHub token request failed");
                unavailable("the token endpoint")
            })?;
        let status = response.status();
        if status.is_server_error() {
            tracing::warn!(%status, "the GitHub token endpoint failed");
            return Err(unavailable("the token endpoint"));
        }
        // GitHub answers a wrong code with the status 200 and an `error`
        // field.
        let body: Value = response.json().await.unwrap_or(Value::Null);
        match body.get("access_token").and_then(Value::as_str) {
            Some(token) if status.is_success() && !token.is_empty() => Ok(token.to_string()),
            _ => {
                let reason = body.get("error").and_then(Value::as_str).unwrap_or("no access token");
                tracing::info!(%status, reason, "GitHub refused an authorization code");
                Err(IdentityError::Denied(
                    "GitHub did not accept the sign-in. Start the sign-in again.".into(),
                ))
            }
        }
    }

    /// Asks for the account of an access token.
    async fn account(&self, access_token: &str) -> Result<Identity, IdentityError> {
        let client = self.client.as_ref().ok_or_else(|| unavailable("no HTTP client"))?;
        let response = client
            .get(self.user.clone())
            .bearer_auth(access_token)
            .header(ACCEPT, "application/json")
            .header(USER_AGENT, AGENT)
            .send()
            .await
            .map_err(|error| {
                tracing::warn!(error = %error.without_url(), "the GitHub user request failed");
                unavailable("the user endpoint")
            })?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(IdentityError::Denied(
                "GitHub did not show the account. Start the sign-in again.".into(),
            ));
        }
        if !status.is_success() {
            tracing::warn!(%status, "the GitHub user endpoint failed");
            return Err(unavailable("the user endpoint"));
        }
        let body: Value = response.json().await.map_err(|_| unavailable("the user endpoint"))?;
        // The id of an account never changes. The login can.
        let subject = match body.get("id") {
            Some(Value::Number(id)) => id.to_string(),
            Some(Value::String(id)) if !id.is_empty() => id.clone(),
            _ => return Err(unavailable("the user endpoint gave no account id")),
        };
        let login = body.get("login").and_then(Value::as_str).unwrap_or_default().trim();
        let name = if login.is_empty() || login.chars().any(char::is_control) {
            format!("github-{subject}")
        } else {
            login.to_string()
        };
        Ok(Identity {
            provider: GitHubSignIn::NAME.to_string(),
            subject,
            name: display_name(&name),
        })
    }

    async fn sign_in(&self, sign_up: &SignUp) -> Result<Identity, IdentityError> {
        let invalid = |message: &str| IdentityError::Invalid(message.to_string());
        let credential = sign_up.credential.as_deref().unwrap_or_default();
        if credential.len() > CREDENTIAL_MAX {
            return Err(invalid("`credential` is too long."));
        }
        let Some((state, code)) =
            credential.split_once(':').filter(|(s, c)| !s.is_empty() && !c.is_empty())
        else {
            return Err(invalid(
                "The provider `github` takes the `credential` `<state>:<code>`. Start the \
                 sign-in at `GET /v1/auth/github`.",
            ));
        };
        let Some(verifier) = self.pending.finish(state, Instant::now()) else {
            return Err(invalid(
                "The `state` is not known, was used already, or is too old. Start the sign-in \
                 again.",
            ));
        };
        let access_token = self.access_token(code, &verifier).await?;
        self.account(&access_token).await
    }
}

impl IdentityProvider for GitHubSignIn {
    fn name(&self) -> &str {
        GitHubSignIn::NAME
    }

    fn identify<'a>(&'a self, sign_up: &'a SignUp) -> IdentityFuture<'a> {
        Box::pin(self.sign_in(sign_up))
    }

    fn begin(&self) -> Result<Option<Authorization>, IdentityError> {
        let started = self
            .pending
            .start(Instant::now())
            .map_err(|error| IdentityError::Unavailable(error.to_string()))?;
        let Some((state, verifier)) = started else {
            return Err(IdentityError::Unavailable(
                "Too many sign-ins wait for GitHub. Try again later.".into(),
            ));
        };
        let challenge = base64_url(&Sha256::digest(verifier.as_bytes()));
        let mut url = self.authorize.clone();
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("response_type", "code")
                .append_pair("client_id", &self.config.client_id)
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            if !self.config.scope.is_empty() {
                query.append_pair("scope", &self.config.scope);
            }
            if !self.config.redirect_url.is_empty() {
                query.append_pair("redirect_uri", &self.config.redirect_url);
            }
        }
        Ok(Some(Authorization { url: url.into(), state, expires_in: self.config.state_ttl_secs }))
    }
}

#[cfg(test)]
mod tests {
    use super::{base64_url, display_name, form_body};

    #[test]
    fn base64_for_addresses_has_no_padding_and_no_plus_or_slash() {
        // The vectors of RFC 4648, without the padding.
        let cases = [("", ""), ("f", "Zg"), ("fo", "Zm8"), ("foo", "Zm9v"), ("foob", "Zm9vYg")];
        for (text, encoded) in cases {
            assert_eq!(base64_url(text.as_bytes()), encoded, "{text}");
        }
        assert_eq!(base64_url(&[0xfb, 0xff, 0xfe]), "-__-");
    }

    #[test]
    fn a_form_body_escapes_what_would_break_it() {
        let body = form_body(&[("code", "a b&c=d"), ("client_secret", "s/+%")]);
        assert_eq!(body, "code=a+b%26c%3Dd&client_secret=s%2F%2B%25");
    }

    #[test]
    fn a_long_name_is_cut_at_a_character() {
        assert_eq!(display_name("ada"), "ada");
        // 63 bytes and then a character of two bytes: it does not fit.
        let long = format!("{}\u{e9}", "x".repeat(63));
        assert_eq!(display_name(&long), "x".repeat(63));
    }
}
