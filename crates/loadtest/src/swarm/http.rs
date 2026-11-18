//! The HTTP side of an agent: one request for each call.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde::de::DeserializeOwned;
use unsup_protocol::{ActionRequest, Observation};

use super::error::{RequestError, from_reqwest};
use super::wire::ActAnswer;

/// The addresses of the routes that the swarm uses. See
/// `docs/design/protocol.md`, section 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Urls {
    /// The address of the server, without a path.
    pub base: String,
    /// `POST /v1/act`.
    pub act: String,
    /// `GET /v1/observe`.
    pub observe: String,
    /// `GET /v1/wait`.
    pub wait: String,
    /// `GET /v1/ws`, with the scheme `ws` or `wss`.
    pub ws: String,
    /// `GET /v1/catalogue`.
    pub catalogue: String,
    /// `GET /v1/world`.
    pub world: String,
    /// `POST /v1/owners`.
    pub owners: String,
    /// `POST /v1/residents`.
    pub residents: String,
    /// `POST /v1/dev/residents`, the sign-up of the development server.
    pub dev_residents: String,
    /// The route with the statistics of the server.
    pub stats: String,
}

impl Urls {
    /// Makes the addresses for a server at `base`, for example
    /// `http://127.0.0.1:8080`.
    ///
    /// # Errors
    ///
    /// Fails when `base` is not an `http://` address. This build has no
    /// TLS.
    pub fn new(base: &str, stats_path: &str) -> Result<Urls, String> {
        let base = base.trim().trim_end_matches('/').to_string();
        let Some(host) = base.strip_prefix("http://") else {
            return Err(if base.starts_with("https://") {
                format!(
                    "`{base}` needs TLS, and this build has none. Use an http:// address. \
                     For a server behind TLS, run the swarm next to the server or through a \
                     local proxy."
                )
            } else {
                format!("`{base}` is not a server address; write it like http://127.0.0.1:8080")
            });
        };
        if host.is_empty() || host.contains('/') {
            return Err(format!("`{base}` must be an address without a path"));
        }
        let at = |path: &str| format!("{base}{path}");
        Ok(Urls {
            act: at("/v1/act"),
            observe: at("/v1/observe"),
            wait: at("/v1/wait"),
            ws: format!("ws://{host}/v1/ws"),
            catalogue: at("/v1/catalogue"),
            world: at("/v1/world"),
            owners: at("/v1/owners"),
            residents: at("/v1/residents"),
            dev_residents: at("/v1/dev/residents"),
            stats: at(&format!("/{}", stats_path.trim_start_matches('/'))),
            base,
        })
    }
}

/// Makes an HTTP client. With `own` it serves one agent and keeps one
/// connection open, like a real agent. Without, many agents share its pool
/// of connections.
///
/// # Errors
///
/// Fails when the client cannot be made.
pub fn client(timeout: Duration, own: bool) -> Result<reqwest::Client, RequestError> {
    reqwest::Client::builder()
        .no_proxy()
        .tcp_nodelay(true)
        // An agent thinks for a long time between two requests. Its
        // connection must stay open meanwhile.
        .pool_idle_timeout(None)
        .pool_max_idle_per_host(if own { 1 } else { usize::MAX })
        .timeout(timeout)
        .build()
        .map_err(|error| from_reqwest(&error))
}

/// Sends a request and reads the whole answer. Returns the body of a
/// successful answer and the time from the send to the last byte.
async fn exchange(request: reqwest::RequestBuilder) -> Result<(Vec<u8>, Duration), RequestError> {
    let start = Instant::now();
    let response = request.send().await.map_err(|error| from_reqwest(&error))?;
    let status = response.status();
    let body = response.bytes().await.map_err(|error| from_reqwest(&error))?;
    let latency = start.elapsed();
    if status.is_success() {
        Ok((body.to_vec(), latency))
    } else {
        Err(RequestError::from_status(status.as_u16(), &body))
    }
}

fn decode<T: DeserializeOwned>(body: &[u8], what: &str) -> Result<T, RequestError> {
    serde_json::from_slice(body).map_err(|error| {
        RequestError::local("decode", format!("{what} is not as expected: {error}"))
    })
}

/// Sends `GET url` and reads a JSON answer.
///
/// # Errors
///
/// Fails when no answer comes, when the status is not a success, or when
/// the body is not the expected JSON.
pub async fn get_json<T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
) -> Result<(T, Duration), RequestError> {
    let mut request = client.get(url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let (body, latency) = exchange(request).await?;
    Ok((decode(&body, "the answer")?, latency))
}

/// Sends `POST url` with a JSON body and reads a JSON answer.
///
/// # Errors
///
/// Fails like [`get_json`].
pub async fn post_json<B: Serialize + ?Sized, T: DeserializeOwned>(
    client: &reqwest::Client,
    url: &str,
    token: Option<&str>,
    body: &B,
) -> Result<(T, Duration), RequestError> {
    let mut request = client.post(url).json(body);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let (body, latency) = exchange(request).await?;
    Ok((decode(&body, "the answer")?, latency))
}

/// The HTTP link of one agent.
#[derive(Clone, Debug)]
pub struct HttpLink {
    client: reqwest::Client,
    urls: Arc<Urls>,
    key: String,
}

impl HttpLink {
    /// Makes a link that sends the key of a resident with each request.
    #[must_use]
    pub fn new(client: reqwest::Client, urls: Arc<Urls>, key: String) -> HttpLink {
        HttpLink { client, urls, key }
    }

    /// Sends one action. The latency is the acknowledgement latency.
    ///
    /// # Errors
    ///
    /// Fails when the server gives no acknowledgement.
    pub async fn act(
        &self,
        request: &ActionRequest,
    ) -> Result<(ActAnswer, Duration), RequestError> {
        post_json(&self.client, &self.urls.act, Some(&self.key), request).await
    }

    /// Asks for an observation with the notices since `since`.
    ///
    /// # Errors
    ///
    /// Fails when the server gives no observation.
    pub async fn observe(&self, since: u64) -> Result<(Observation, Duration), RequestError> {
        let url = format!("{}?since={since}", self.urls.observe);
        get_json(&self.client, &url, Some(&self.key)).await
    }

    /// Waits in a long poll until an action report comes or `timeout`
    /// passes. `patience` is the extra time that the client gives the
    /// server to answer after the timeout.
    ///
    /// # Errors
    ///
    /// Fails when the server gives no observation. An error for which
    /// [`RequestError::route_absent`] is true means that the server has no
    /// long poll.
    pub async fn wait(
        &self,
        since: u64,
        timeout: Duration,
        patience: Duration,
    ) -> Result<Observation, RequestError> {
        let url =
            format!("{}?timeout={}&since={since}&kinds=report", self.urls.wait, timeout.as_secs());
        let request =
            self.client.get(url).bearer_auth(&self.key).timeout(timeout.saturating_add(patience));
        let (body, _) = exchange(request).await?;
        decode(&body, "the answer of the long poll")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_follow_the_protocol_document() {
        let urls = Urls::new("http://127.0.0.1:8080/", "v1/stats").expect("a valid address");
        assert_eq!(urls.base, "http://127.0.0.1:8080");
        assert_eq!(urls.act, "http://127.0.0.1:8080/v1/act");
        assert_eq!(urls.ws, "ws://127.0.0.1:8080/v1/ws");
        assert_eq!(urls.dev_residents, "http://127.0.0.1:8080/v1/dev/residents");
        assert_eq!(urls.stats, "http://127.0.0.1:8080/v1/stats");
    }

    #[test]
    fn an_address_that_this_build_cannot_reach_is_refused_with_a_reason() {
        let tls = Urls::new("https://example.org", "/v1/stats").expect_err("TLS is not built in");
        assert!(tls.contains("TLS"), "{tls}");
        let bare = Urls::new("localhost:8080", "/v1/stats").expect_err("no scheme");
        assert!(bare.contains("http://127.0.0.1:8080"), "{bare}");
        let path = Urls::new("http://host/api", "/v1/stats").expect_err("a path");
        assert!(path.contains("without a path"), "{path}");
    }
}
