//! Sign-in with GitHub against a fake provider that the test runs: a
//! wrong state, a wrong code, a provider that is down, the start for a
//! program, and the limits. `oauth_flow.rs` has the whole flow.
//!
//! These tests never talk to GitHub. The fake provider checks what a real
//! one checks: the client id and secret, the code, the redirect address
//! and the proof key (PKCE).

mod common;

use common::github::{Scene, cookie_pair, github_toml, query_of};
use common::{Harness, code, config, send};
use reqwest::StatusCode;
use reqwest::header::CACHE_CONTROL;
use serde_json::{Value, json};
use unsup_gateway::{GatewayConfig, GitHubConfig};

#[tokio::test]
async fn a_wrong_state_ends_the_sign_in_before_the_provider_is_asked() {
    let scene = Scene::start("").await;
    let (authorize_url, set_cookie) = scene.begin().await;
    let cookie = cookie_pair(&set_cookie);
    let good = scene.agree(&authorize_url).await;
    let code = query_of(&good)["code"].clone();
    let state = query_of(&good)["state"].clone();
    let callback = |code: &str, state: &str| {
        scene.h.url(&format!("/v1/auth/github/callback?code={code}&state={state}"))
    };
    let not_this_browser = json!({ "error": { "code": "invalid_request",
        "message": "This browser did not start the sign-in, or the sign-in is too old. Start \
                    it again." } });
    let unknown_state = json!({ "error": { "code": "invalid_request",
        "message": "The `state` is not known, was used already, or is too old. Start the \
                    sign-in again." } });

    // A state that the server never made, with the cookie of the true
    // start: the two do not match.
    let forged = "f".repeat(64);
    let (status, answer) = scene.callback(&callback(&code, &forged), &cookie).await;
    assert_eq!((status, &answer), (StatusCode::BAD_REQUEST, &not_this_browser));
    // The same forged state in the cookie too: the server does not know
    // the state.
    let forged_cookie = format!("unsup_signin={forged}");
    let (status, answer) = scene.callback(&callback(&code, &forged), &forged_cookie).await;
    assert_eq!((status, &answer), (StatusCode::BAD_REQUEST, &unknown_state));
    // The true address in a browser that did not make the start: no
    // cookie, or the cookie of another start.
    let (status, answer) = send(scene.browser.get(&good)).await;
    assert_eq!((status, &answer), (StatusCode::BAD_REQUEST, &not_this_browser));
    let (_, other_cookie) = scene.begin().await;
    let (status, answer) = scene.callback(&good, &cookie_pair(&other_cookie)).await;
    assert_eq!((status, &answer), (StatusCode::BAD_REQUEST, &not_this_browser));
    // An address without a code or without a state.
    for query in ["?code=abc", &format!("?state={state}"), ""] {
        let address = scene.h.url(&format!("/v1/auth/github/callback{query}"));
        let (status, answer) = scene.callback(&address, &cookie).await;
        assert_eq!((status, code_of(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    }
    assert_eq!(scene.token_requests(), 0, "the provider was never asked");
    let (_, stats) = send(scene.h.get("/admin/stats", common::ADMIN)).await;
    assert_eq!(stats["store"]["owners"], 0, "nobody signed up");

    // None of this used the state: the browser of the start still ends
    // its sign-in.
    let (status, signed) = scene.callback(&good, &cookie).await;
    assert_eq!(status, StatusCode::CREATED, "{signed}");
    assert_eq!(scene.token_requests(), 1);
    // The same address a second time: the state is used.
    let (status, answer) = scene.callback(&good, &cookie).await;
    assert_eq!((status, &answer), (StatusCode::BAD_REQUEST, &unknown_state));
    assert_eq!(scene.token_requests(), 1);
}

fn code_of(answer: &Value) -> &str {
    code(answer)
}

#[tokio::test]
async fn the_provider_can_refuse_and_can_be_down() {
    let scene = Scene::start("").await;
    let denied =
        |message: &str| json!({ "error": { "code": "sign_up_denied", "message": message } });

    // A code that the provider does not know: GitHub answers 200 with an
    // error field.
    let (authorize_url, set_cookie) = scene.begin().await;
    let state = query_of(&authorize_url)["state"].clone();
    let address = scene.h.url(&format!("/v1/auth/github/callback?code=made-up&state={state}"));
    let (status, answer) = scene.callback(&address, &cookie_pair(&set_cookie)).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::FORBIDDEN,
            denied("GitHub did not accept the sign-in. Start the sign-in again.")
        )
    );
    assert_eq!(scene.token_requests(), 1);

    // The person did not agree: the provider sends an error, no code.
    let (_, set_cookie) = scene.begin().await;
    let address = scene.h.url("/v1/auth/github/callback?error=access_denied&error_description=no");
    let (status, answer) = scene.callback(&address, &cookie_pair(&set_cookie)).await;
    assert_eq!(
        (status, answer),
        (StatusCode::FORBIDDEN, denied("The sign-in did not end at the provider. Start it again."))
    );
    assert_eq!(scene.token_requests(), 1, "no request for a sign-in without a code");

    // A provider that is down: nothing listens at its address.
    let closed = std::net::TcpListener::bind("127.0.0.1:0").expect("a free port");
    let dead = format!("http://{}", closed.local_addr().expect("an address"));
    drop(closed);
    let toml =
        github_toml(&dead, "http://127.0.0.1:1/v1/auth/github/callback", "timeout_ms = 2000");
    let h = Harness::with(config(&toml)).await;
    let (status, started) = send(h.http.get(h.url("/v1/auth/github?format=json"))).await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let state = started["state"].as_str().expect("a state");
    let body = json!({ "provider": "github", "credential": format!("{state}:some-code") });
    let (status, answer) = send(h.http.post(h.url("/v1/owners")).json(&body)).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": { "code": "provider_unavailable",
                "message": "GitHub does not answer now (the token endpoint). Try again later." } })
        )
    );
}

#[tokio::test]
async fn a_program_can_start_with_json_and_finish_with_the_sign_up_route() {
    // Refused sign-ups count for the sign-up limit, so it is off here.
    let scene = Scene::start("state_ttl_secs = 90\n[limits]\nsignups_per_hour = 0").await;
    let response = scene
        .browser
        .get(scene.h.url("/v1/auth/github?format=json"))
        .send()
        .await
        .expect("the start");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    let started: Value = response.json().await.expect("JSON");
    assert_eq!(started["provider"], "github");
    assert_eq!(started["expires_in"], 90);
    let authorize_url = started["authorize_url"].as_str().expect("an address");
    let state = started["state"].as_str().expect("a state");
    assert_eq!(query_of(authorize_url)["state"], state);

    // A front end of the operator gets the redirect and sends the code
    // and the state to the sign-up route.
    let callback = scene.agree(authorize_url).await;
    let code = query_of(&callback)["code"].clone();
    let sign_up = |credential: String| {
        let body = json!({ "provider": "github", "credential": credential });
        send(scene.h.http.post(scene.h.url("/v1/owners")).json(&body))
    };
    let (status, signed) = sign_up(format!("{state}:{code}")).await;
    assert_eq!(status, StatusCode::CREATED, "{signed}");
    assert_eq!(signed["owner"]["name"], "octo-ada");
    // The state is used. A credential of another shape is refused.
    let (status, answer) = sign_up(format!("{state}:{code}")).await;
    assert_eq!((status, code_of(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    for credential in ["", "no-colon", ":code", "state:"] {
        let (status, answer) = sign_up(credential.to_string()).await;
        assert_eq!((status, code_of(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
        let message = answer["error"]["message"].as_str().expect("a message");
        assert!(message.starts_with("The provider `github` takes the `credential`"), "{message}");
    }
    assert_eq!(scene.token_requests(), 1);

    // The other providers of the server: `local` has no sign-in page,
    // and no provider has another name.
    let (status, answer) = send(scene.h.http.get(scene.h.url("/v1/auth/local"))).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::BAD_REQUEST,
            json!({ "error": { "code": "invalid_request",
                "message": "This provider has no sign-in page. Sign up with `POST /v1/owners`." } })
        )
    );
    let (status, answer) = send(scene.h.http.get(scene.h.url("/v1/auth/gitlab"))).await;
    assert_eq!((status, code_of(&answer)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, answer) = send(scene.h.http.get(scene.h.url("/v1/auth/github?format=xml"))).await;
    assert_eq!((status, code_of(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
}

#[tokio::test]
async fn the_sign_in_is_off_without_a_client_id_and_a_secret() {
    // The defaults: nothing is set.
    let h = Harness::start().await;
    let (status, answer) = send(h.http.get(h.url("/v1/auth/github"))).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::NOT_FOUND,
            json!({ "error": { "code": "not_found",
                "message": "This server has no sign-in with this provider." } })
        )
    );
    let (status, answer) = send(h.http.get(h.url("/v1/auth/github/callback?code=a&state=b"))).await;
    assert_eq!((status, code_of(&answer)), (StatusCode::NOT_FOUND, "not_found"));
    let body = json!({ "name": "Ada", "provider": "github" });
    let (status, answer) = send(h.http.post(h.url("/v1/owners")).json(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["error"]["message"], "`provider` is not known. This server has: local.");

    // One of the two alone does not turn it on.
    for toml in
        ["[github]\nclient_id = \"only-the-id\"", "[github]\nclient_secret = \"only-the-secret\""]
    {
        let config = config(toml);
        assert!(!config.github.is_on(), "{toml}");
        let h = Harness::with(config).await;
        let (status, _) = send(h.http.get(h.url("/v1/auth/github"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{toml}");
    }

    // With both, the provider is in the list, behind `local`.
    let on = config("[github]\nclient_id = \"an-id\"\nclient_secret = \"a-secret-of-the-test\"");
    assert!(on.github.is_on());
    // A print of the configuration does not show the secret.
    let shown = format!("{on:?}");
    assert!(shown.contains("an-id") && shown.contains("ClientSecret(<hidden>)"), "{shown}");
    assert!(!shown.contains("a-secret-of-the-test"), "{shown}");
    let h = Harness::with(on).await;
    let body = json!({ "name": "Ada", "provider": "gitlab" });
    let (_, answer) = send(h.http.post(h.url("/v1/owners")).json(&body)).await;
    assert_eq!(
        answer["error"]["message"],
        "`provider` is not known. This server has: local, github."
    );
    // Local sign-up is still the default.
    let (owner, _) = h.owner("Ada").await;
    assert_eq!(owner, 1);
}

#[tokio::test]
async fn a_start_counts_for_the_sign_up_limit_and_the_table_of_starts_has_a_size() {
    let scene =
        Scene::start("max_pending = 3\n[limits]\nsignups_per_hour = 60\nsignup_burst = 2").await;
    scene.begin().await;
    scene.begin().await;
    let response =
        scene.browser.get(scene.h.url("/v1/auth/github")).send().await.expect("the start");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("60"));

    // Without that limit, the table of open sign-ins is the limit.
    let scene = Scene::start("max_pending = 3\n[limits]\nsignups_per_hour = 0").await;
    for _ in 0..3 {
        scene.begin().await;
    }
    let (status, answer) = send(scene.browser.get(scene.h.url("/v1/auth/github"))).await;
    assert_eq!(
        (status, answer),
        (
            StatusCode::SERVICE_UNAVAILABLE,
            json!({ "error": { "code": "provider_unavailable",
                "message": "Too many sign-ins wait for GitHub. Try again later." } })
        )
    );
}

#[tokio::test]
async fn endpoints_that_are_not_addresses_stop_the_configuration() {
    let base = "[github]\nclient_id = \"an-id\"\nclient_secret = \"a-secret\"\n";
    let cases = [
        ("token_url = \"not an address\"", "`github.token_url` is not an address"),
        ("user_url = \"ftp://example.org/user\"", "`github.user_url` must start with http://"),
        ("authorize_url = \"\"", "`github.authorize_url` is not an address"),
        ("redirect_url = \"/v1/auth/github/callback\"", "`github.redirect_url` is not an address"),
    ];
    for (line, reason) in cases {
        let error = GatewayConfig::from_toml(&format!("{base}{line}")).expect_err("a refusal");
        assert!(error.0.contains(reason), "{line}: {}", error.0);
    }
    // The same values are fine while the sign-in is off.
    let off = GatewayConfig::from_toml("[github]\ntoken_url = \"not an address\"");
    assert!(off.is_ok(), "{off:?}");
    let _: GitHubConfig = GatewayConfig::default().github;
}
