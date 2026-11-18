//! A fake OAuth provider with the endpoints of GitHub, and a gateway
//! that is configured for it. The tests of the sign-in run it; they never
//! talk to GitHub.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode as Status};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reqwest::StatusCode;
use reqwest::header::{COOKIE, LOCATION, SET_COOKIE};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{Harness, config, send};

pub const CLIENT_ID: &str = "client-id-of-the-test";
pub const CLIENT_SECRET: &str = "client-secret-0123456789abcdef";
pub const ACCESS_TOKEN: &str = "gho_access_token_of_the_fake";

/// One authorization that the fake provider gave.
#[derive(Clone, Debug)]
pub struct Grant {
    challenge: String,
    redirect_uri: String,
    /// The account that agreed: its id and its login name.
    account: (u64, String),
}

/// What the fake provider remembers.
#[derive(Debug, Default)]
pub struct Provider {
    /// The grants, by their code.
    pub grants: HashMap<String, Grant>,
    /// The account of each access token.
    pub tokens: HashMap<String, (u64, String)>,
    pub last_code: u64,
    /// The account that agrees at the next visit of the authorize page.
    pub account: (u64, String),
    /// Every request to the token endpoint: the query of its address and
    /// its body.
    pub token_requests: Vec<(String, String)>,
    /// Every request to the user endpoint: its `Authorization` header.
    pub user_requests: Vec<String>,
}

pub type Shared = Arc<Mutex<Provider>>;

/// Encodes bytes as Base64 for addresses, without padding.
fn base64_url(bytes: &[u8]) -> String {
    const DIGITS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let mut group = 0u32;
        for index in 0..3 {
            group = (group << 8) | u32::from(*chunk.get(index).unwrap_or(&0));
        }
        for index in 0..=chunk.len() {
            out.push(char::from(DIGITS[((group >> (18 - 6 * index)) & 0x3f) as usize]));
        }
    }
    out
}

/// The authorize page: the person agrees at once. The fake sends the
/// browser back with a code and the state.
async fn authorize(
    State(provider): State<Shared>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let wanted = [
        ("response_type", "code"),
        ("client_id", CLIENT_ID),
        ("code_challenge_method", "S256"),
        ("scope", "read:user"),
    ];
    for (name, value) in wanted {
        if query.get(name).map(String::as_str) != Some(value) {
            return (Status::BAD_REQUEST, format!("wrong {name}")).into_response();
        }
    }
    let (Some(state), Some(challenge), Some(redirect_uri)) =
        (query.get("state"), query.get("code_challenge"), query.get("redirect_uri"))
    else {
        return (Status::BAD_REQUEST, "state, code_challenge or redirect_uri is missing")
            .into_response();
    };
    let mut provider = provider.lock().expect("the lock");
    provider.last_code += 1;
    let code = format!("code-{}-of-the-fake", provider.last_code);
    let grant = Grant {
        challenge: challenge.clone(),
        redirect_uri: redirect_uri.clone(),
        account: provider.account.clone(),
    };
    provider.grants.insert(code.clone(), grant);
    Redirect::to(&format!("{redirect_uri}?code={code}&state={state}")).into_response()
}

/// The token endpoint: exchanges a code for an access token, like GitHub
/// with the status 200 also for a refusal.
async fn token(
    State(provider): State<Shared>,
    uri: axum::http::Uri,
    headers: HeaderMap,
    body: String,
) -> Response {
    let form: HashMap<String, String> =
        reqwest::Url::parse(&format!("http://form.invalid/?{body}"))
            .expect("a form body")
            .query_pairs()
            .into_owned()
            .collect();
    let mut provider = provider.lock().expect("the lock");
    provider.token_requests.push((uri.query().unwrap_or_default().to_string(), body.clone()));
    let refuse = |error: &str| Json(json!({ "error": error })).into_response();
    let field = |name: &str| form.get(name).map(String::as_str).unwrap_or_default();
    if headers.get("accept").and_then(|v| v.to_str().ok()) != Some("application/json") {
        return refuse("not_acceptable");
    }
    if field("client_id") != CLIENT_ID || field("client_secret") != CLIENT_SECRET {
        return refuse("incorrect_client_credentials");
    }
    // A code is good one time.
    let Some(grant) = provider.grants.remove(field("code")) else {
        return refuse("bad_verification_code");
    };
    if field("redirect_uri") != grant.redirect_uri {
        return refuse("redirect_uri_mismatch");
    }
    if base64_url(&Sha256::digest(field("code_verifier").as_bytes())) != grant.challenge {
        return refuse("bad_code_verifier");
    }
    let access_token = format!("{ACCESS_TOKEN}_{}", grant.account.0);
    provider.tokens.insert(access_token.clone(), grant.account);
    Json(json!({ "access_token": access_token, "token_type": "bearer", "scope": "read:user" }))
        .into_response()
}

/// The user endpoint: the account of an access token.
async fn user(State(provider): State<Shared>, headers: HeaderMap) -> Response {
    let authorization =
        headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let mut provider = provider.lock().expect("the lock");
    provider.user_requests.push(authorization.clone());
    if headers.get("user-agent").is_none() {
        return (Status::FORBIDDEN, "a user agent is required").into_response();
    }
    match authorization.strip_prefix("Bearer ").and_then(|token| provider.tokens.get(token)) {
        Some((id, login)) => {
            Json(json!({ "id": id, "login": login, "name": null })).into_response()
        }
        None => Status::UNAUTHORIZED.into_response(),
    }
}

/// A running fake provider and a gateway that is configured for it.
pub struct Scene {
    pub h: Harness,
    pub provider: Shared,
    pub provider_base: String,
    /// A client that follows no redirect and keeps no cookie: the test
    /// plays the browser by hand.
    pub browser: reqwest::Client,
}

pub async fn fake_provider() -> (Shared, SocketAddr) {
    let provider: Shared = Arc::default();
    provider.lock().expect("the lock").account = (4242, "octo-ada".into());
    let app = Router::new()
        .route("/authorize", get(authorize))
        .route("/token", post(token))
        .route("/user", get(user))
        .with_state(Arc::clone(&provider));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a free port");
    let addr = listener.local_addr().expect("an address");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("the fake provider runs") });
    (provider, addr)
}

/// The configuration of a gateway whose GitHub sign-in points at `base`.
/// The callback address is filled in when the gateway has its port.
pub fn github_toml(base: &str, callback: &str, more: &str) -> String {
    format!(
        "[github]\nclient_id = \"{CLIENT_ID}\"\nclient_secret = \"{CLIENT_SECRET}\"\n\
         authorize_url = \"{base}/authorize\"\ntoken_url = \"{base}/token\"\n\
         user_url = \"{base}/user\"\nredirect_url = \"{callback}\"\n{more}"
    )
}

impl Scene {
    pub async fn start(more: &str) -> Scene {
        let (provider, addr) = fake_provider().await;
        let provider_base = format!("http://{addr}");
        // The gateway must know its own callback address, so the test
        // picks the port first.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().expect("an address").port();
        drop(listener);
        let callback = format!("http://127.0.0.1:{port}/v1/auth/github/callback");
        let config = config(&github_toml(&provider_base, &callback, more));
        let h = Harness::on_port(config, port).await;
        let browser = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("an HTTP client");
        Scene { h, provider, provider_base, browser }
    }

    /// Starts a sign-in as a browser does. Returns the address of the
    /// authorize page and the cookie that the server set.
    pub async fn begin(&self) -> (String, String) {
        let response =
            self.browser.get(self.h.url("/v1/auth/github")).send().await.expect("the start");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        let location = response.headers()[LOCATION].to_str().expect("text").to_string();
        let cookie = response.headers()[SET_COOKIE].to_str().expect("text").to_string();
        (location, cookie)
    }

    /// Visits the authorize page of the fake provider. Returns the
    /// address of the callback that the provider sends the browser to.
    pub async fn agree(&self, authorize_url: &str) -> String {
        let response = self.browser.get(authorize_url).send().await.expect("the authorize page");
        assert_eq!(response.status(), StatusCode::SEE_OTHER, "the provider agrees");
        response.headers()[LOCATION].to_str().expect("text").to_string()
    }

    /// Opens a callback address with a cookie.
    pub async fn callback(&self, address: &str, cookie: &str) -> (StatusCode, Value) {
        send(self.browser.get(address).header(COOKIE, cookie)).await
    }

    pub fn token_requests(&self) -> usize {
        self.provider.lock().expect("the lock").token_requests.len()
    }
}

/// Returns `name=value` of a `Set-Cookie` header.
pub fn cookie_pair(set_cookie: &str) -> String {
    set_cookie.split(';').next().expect("a pair").to_string()
}

pub fn query_of(address: &str) -> HashMap<String, String> {
    reqwest::Url::parse(address).expect("an address").query_pairs().into_owned().collect()
}
