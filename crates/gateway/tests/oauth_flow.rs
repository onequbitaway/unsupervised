//! The whole sign-in with GitHub against a fake provider, and what the
//! log holds after it.
//!
//! This file has one test, because the test takes the log of the whole
//! process. It never talks to GitHub.

mod common;

use std::io::Write;
use std::sync::{Arc, Mutex};

use common::github::{ACCESS_TOKEN, CLIENT_SECRET, Scene, cookie_pair, query_of};
use common::send;
use reqwest::StatusCode;
use reqwest::header::{CACHE_CONTROL, COOKIE, SET_COOKIE};
use serde_json::{Value, json};

/// A place for the log of the process.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("the lock").extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn the_whole_flow_signs_an_owner_in_and_keeps_every_secret_out_of_addresses_and_the_log() {
    // Every line that any thread of this process logs goes into a
    // buffer, at the finest level.
    let log = Log::default();
    let writer = log.clone();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .init();

    let scene = Scene::start("").await;
    let (authorize_url, set_cookie) = scene.begin().await;

    // The address of the authorize page: the client id, the state, the
    // challenge of the proof key. Not the client secret.
    assert!(authorize_url.starts_with(&format!("{}/authorize?", scene.provider_base)));
    let query = query_of(&authorize_url);
    let state = query["state"].clone();
    assert_eq!(state.len(), 64, "32 random bytes as text");
    let mut names: Vec<&str> = query.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "client_id",
            "code_challenge",
            "code_challenge_method",
            "redirect_uri",
            "response_type",
            "scope",
            "state"
        ]
    );
    assert_eq!(query["redirect_uri"], scene.h.url("/v1/auth/github/callback"));
    assert!(!authorize_url.contains(CLIENT_SECRET));
    // The cookie ties the state to this browser. A script of a page
    // cannot read it.
    assert_eq!(
        set_cookie,
        format!("unsup_signin={state}; Path=/v1/auth/github; Max-Age=600; HttpOnly; SameSite=Lax")
    );

    let callback = scene.agree(&authorize_url).await;
    let code = query_of(&callback)["code"].clone();
    let response = scene
        .browser
        .get(&callback)
        .header(COOKIE, cookie_pair(&set_cookie))
        .send()
        .await
        .expect("the callback");
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    // The cookie did its job and goes.
    let cleared = response.headers()[SET_COOKIE].to_str().expect("text");
    assert!(cleared.starts_with("unsup_signin=; ") && cleared.contains("Max-Age=0"), "{cleared}");
    let signed: Value = response.json().await.expect("JSON");
    assert_eq!(signed["owner"]["name"], "octo-ada");
    let owner_token = signed["token"].as_str().expect("an owner token").to_string();
    assert!(owner_token.starts_with("unsup_own_"));

    // The token is a normal owner token.
    let (resident, _) = scene.h.resident(&owner_token, "Ada").await;
    let (status, listed) = send(scene.h.get("/v1/residents", &owner_token)).await;
    assert_eq!((status, listed["residents"][0]["id"].clone()), (StatusCode::OK, json!(resident)));

    // What the provider saw: the secret, the code and the proof key in
    // the body of a POST, the access token in a header. Nothing in an
    // address.
    {
        let provider = scene.provider.lock().expect("the lock");
        assert_eq!(provider.token_requests.len(), 1);
        let (query, body) = &provider.token_requests[0];
        assert_eq!(query, "", "the address of the token request has no query");
        assert!(body.contains(&format!("client_secret={CLIENT_SECRET}")), "{body}");
        assert!(body.contains(&format!("code={code}")), "{body}");
        assert!(body.contains("code_verifier="), "{body}");
        assert_eq!(provider.user_requests, [format!("Bearer {ACCESS_TOKEN}_4242")]);
    }

    // The same account signs in again: the same owner, a new token. The
    // login name changed at GitHub; the account id decides.
    scene.provider.lock().expect("the lock").account = (4242, "octo-renamed".into());
    let (authorize_url, set_cookie) = scene.begin().await;
    let callback = scene.agree(&authorize_url).await;
    let (status, again) = scene.callback(&callback, &cookie_pair(&set_cookie)).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["owner"]["id"], signed["owner"]["id"]);
    assert_ne!(again["token"], signed["token"]);
    // Another account is another owner.
    scene.provider.lock().expect("the lock").account = (77, "octo-bob".into());
    let (authorize_url, set_cookie) = scene.begin().await;
    let callback = scene.agree(&authorize_url).await;
    let (status, other) = scene.callback(&callback, &cookie_pair(&set_cookie)).await;
    assert_eq!(status, StatusCode::CREATED, "{other}");
    assert_ne!(other["owner"]["id"], signed["owner"]["id"]);
    assert_eq!(other["owner"]["name"], "octo-bob");

    // The log tells that owners signed up, and holds no secret.
    let log = String::from_utf8(log.0.lock().expect("the lock").clone()).expect("text");
    assert!(log.contains("owner signed up"), "{log}");
    let second_token = again["token"].as_str().expect("a token");
    for secret in [CLIENT_SECRET, ACCESS_TOKEN, &state, &code, &owner_token, second_token] {
        assert!(!log.contains(secret), "the log holds a secret:\n{log}");
    }
    assert!(!log.contains(&owner_token["unsup_own_".len()..]), "{log}");
}
