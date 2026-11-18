//! Limits, the administrator, sign-up rules and the other protections of
//! the gateway, over real HTTP.

mod common;

use std::sync::Arc;

use common::{ADMIN, Harness, code, config, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use unsup_protocol::{AdminCommand, AdminReply, ResidentId};
use unsup_store::{Identity, IdentityError, IdentityFuture, IdentityProvider, LocalSignUp, SignUp};

#[tokio::test]
async fn an_owner_has_at_most_the_configured_number_of_residents() {
    let h = Harness::with_toml("[limits]\nresidents_per_owner = 2").await;
    let (_, token) = h.owner("Ada").await;
    let (first, first_key) = h.resident(&token, "One").await;
    let (second, _) = h.resident(&token, "Two").await;

    let (status, answer) = send(h.post("/v1/residents", &token, &json!({ "name": "Three" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        answer,
        json!({ "error": {
            "code": "resident_limit",
            "message": "An owner has at most 2 residents in the world at a time.",
        } })
    );
    let (_, listed) = send(h.get("/v1/residents", &token)).await;
    assert_eq!(listed["limit"], 2);
    assert_eq!(
        listed["residents"]
            .as_array()
            .expect("residents")
            .iter()
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        [json!(first), json!(second)]
    );
    // The limit is for each owner.
    let (_, other) = h.owner("Bob").await;
    h.resident(&other, "Other").await;

    // A resident that left the world frees its place. Here the world
    // removes it without a word to the gateway, as a death would.
    let gone = h.sim.admin(AdminCommand::Despawn { resident: ResidentId(first) }).await;
    assert_eq!(gone, Ok(AdminReply::Ok));
    let (third, _) = h.resident(&token, "Three").await;
    let (_, listed) = send(h.get("/v1/residents", &token)).await;
    assert_eq!(
        listed["residents"]
            .as_array()
            .expect("residents")
            .iter()
            .map(|r| r["id"].clone())
            .collect::<Vec<_>>(),
        [json!(second), json!(third)]
    );
    // The key of the resident that left does not work any more.
    let (status, answer) = send(h.get("/v1/me", &first_key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
}

#[tokio::test]
async fn a_key_of_a_resident_that_left_the_world_answers_gone() {
    let h = Harness::start().await;
    let (resident, key) = h.agent("Ada").await;
    let gone = h.sim.admin(AdminCommand::Despawn { resident: ResidentId(resident) }).await;
    assert_eq!(gone, Ok(AdminReply::Ok));

    // The first call finds out and says so. After it the key is unknown.
    let (status, answer) = send(h.get("/v1/observe", &key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::GONE, "resident_gone"));
    let (status, answer) = send(h.get("/v1/observe", &key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
}

#[tokio::test]
async fn the_rate_limit_of_a_key_answers_429() {
    // Three requests at once, then one more each minute.
    let h = Harness::with_toml("[limits]\nkey_requests_per_minute = 1\nkey_burst = 3").await;
    let (_, token) = h.owner("Ada").await;
    let (_, key) = h.resident(&token, "One").await; // The owner used 1 of 3.
    let (_, other_key) = h.agent("Two").await;

    for _ in 0..3 {
        let (status, _) = send(h.get("/v1/me", &key)).await;
        assert_eq!(status, StatusCode::OK);
    }
    let response = h.get("/v1/me", &key).send().await.expect("answer");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("60"));
    let body: Value = response.json().await.expect("JSON");
    assert_eq!(
        body,
        json!({ "error": {
            "code": "rate_limited",
            "message": "Too many requests. Try again in 60 seconds.",
        } })
    );
    // Each key has its own bucket.
    let (status, _) = send(h.get("/v1/me", &other_key)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = send(h.get("/v1/residents", &token)).await;
    assert_eq!(status, StatusCode::OK);
    // The administrator sees the count.
    let (_, stats) = send(h.get("/admin/stats", ADMIN)).await;
    assert_eq!(stats["gateway"]["rate_limited"], 1);
}

#[tokio::test]
async fn the_rate_limit_of_an_address_answers_429() {
    let h =
        Harness::with_toml("[limits]\naddress_requests_per_minute = 1\naddress_burst = 2").await;
    for _ in 0..2 {
        let (status, _) = send(h.http.get(h.url("/v1/catalogue"))).await;
        assert_eq!(status, StatusCode::OK);
    }
    // The limit counts before the token is read, so a guess costs too.
    let response = h.get("/v1/me", "unsup_res_guess").send().await.expect("answer");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("60"));
    let (status, answer) = send(h.http.get(h.url("/v1/catalogue"))).await;
    assert_eq!((status, code(&answer)), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    // The health check has no limit.
    for _ in 0..5 {
        let health = h.http.get(h.url("/healthz")).send().await.expect("healthz");
        assert_eq!(health.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn the_client_address_comes_from_the_trusted_proxy() {
    // One trusted proxy: the last entry of X-Forwarded-For is the client.
    let h = Harness::with_toml(
        "forwarded_hops = 1\n[limits]\naddress_requests_per_minute = 1\naddress_burst = 1",
    )
    .await;
    let from = |forwarded: &'static str| {
        let request = h.http.get(h.url("/v1/catalogue")).header("X-Forwarded-For", forwarded);
        async move { send(request).await.0 }
    };
    assert_eq!(from("203.0.113.7").await, StatusCode::OK);
    assert_eq!(from("203.0.113.7").await, StatusCode::TOO_MANY_REQUESTS);
    // Another client behind the same proxy has its own bucket.
    assert_eq!(from("203.0.113.8").await, StatusCode::OK);
    // A client cannot pick its address: only the last entry counts.
    assert_eq!(from("198.51.100.1, 203.0.113.8").await, StatusCode::TOO_MANY_REQUESTS);
    // Two addresses in one IPv6 /64 network are one client.
    assert_eq!(from("2001:db8:1:2::1").await, StatusCode::OK);
    assert_eq!(from("2001:db8:1:2:ffff::9").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(from("2001:db8:1:3::1").await, StatusCode::OK);
}

#[tokio::test]
async fn sign_ups_have_their_own_limit() {
    let h = Harness::with_toml("[limits]\nsignups_per_hour = 1\nsignup_burst = 2").await;
    h.owner("One").await;
    h.owner("Two").await;
    let sign_up = h.http.post(h.url("/v1/owners")).json(&json!({ "name": "Three" }));
    let response = sign_up.send().await.expect("answer");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers().get("retry-after").and_then(|v| v.to_str().ok()), Some("3600"));
    // Other routes are not touched by the sign-up limit.
    let (status, _) = send(h.http.get(h.url("/v1/catalogue"))).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_body_over_the_size_limit_is_refused() {
    let h = Harness::with_toml("[limits]\nmax_body_bytes = 512").await;
    let (_, key) = h.agent("Ada").await;

    let small = json!({ "verb": "tell", "args": { "to": 1, "text": "x".repeat(300) } });
    let (status, _) = send(h.post("/v1/act", &key, &small)).await;
    assert_eq!(status, StatusCode::OK);

    let large = json!({ "verb": "tell", "args": { "to": 1, "text": "x".repeat(600) } });
    let (status, answer) = send(h.post("/v1/act", &key, &large)).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        answer,
        json!({ "error": {
            "code": "payload_too_large",
            "message": "The request body is larger than the server permits.",
        } })
    );
    // The same limit holds for the MCP endpoint and for sign-up.
    let request =
        h.post("/mcp", &key, &json!({ "jsonrpc": "2.0", "id": 1, "method": "x".repeat(600) }));
    let (status, answer) = send(request).await;
    assert_eq!((status, code(&answer)), (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"));
    let sign_up = h.http.post(h.url("/v1/owners")).json(&json!({ "name": "x".repeat(600) }));
    let (status, answer) = send(sign_up).await;
    assert_eq!((status, code(&answer)), (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"));
    // Only the first `tell` reached the world.
    let (_, after) = send(h.get("/v1/observe", &key)).await;
    assert_eq!(after["notices"].as_array().map(Vec::len), Some(1));
}

#[tokio::test]
async fn admin_routes_refuse_a_wrong_token() {
    let h = Harness::start().await;
    let (_, owner_token) = h.owner("Ada").await;
    let (resident, key) = h.resident(&owner_token, "One").await;
    let freeze = format!("/admin/residents/{resident}/freeze");

    // Wrong tokens: a guess, a token that differs in one character, and
    // good tokens of the other kinds.
    let almost = format!("{}x", &ADMIN[..ADMIN.len() - 1]);
    for token in ["guess", &almost, &owner_token, &key] {
        let (status, answer) = send(h.request(Method::POST, &freeze, token)).await;
        assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"), "{token}");
        let (status, answer) = send(h.get("/admin/stats", token)).await;
        assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"), "{token}");
    }
    // Nothing was frozen.
    let work = json!({ "verb": "work", "args": { "ticks": 1 } });
    let (_, answer) = send(h.post("/v1/act", &key, &work)).await;
    assert_eq!(answer["acks"][0]["status"]["state"], "started");
    h.step(1).await;

    // The admin token freezes the resident: every action is refused.
    let (status, answer) = send(h.request(Method::POST, &freeze, ADMIN)).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    let (_, answer) = send(h.post("/v1/act", &key, &work)).await;
    assert_eq!(answer["acks"][0]["status"]["state"], "rejected");
    assert_eq!(answer["acks"][0]["status"]["error"]["code"], "blocked");
    let unfreeze = format!("/admin/residents/{resident}/unfreeze");
    let (status, _) = send(h.request(Method::POST, &unfreeze, ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    let (_, answer) = send(h.post("/v1/act", &key, &work)).await;
    assert_eq!(answer["acks"][0]["status"]["state"], "started");

    // A parameter: the queue takes no planned action any more.
    let param = json!({ "name": "kernel.queue_max", "value": 0 });
    let (status, answer) = send(h.post("/admin/params", ADMIN, &param)).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    h.step(1).await;
    let (_, answer) = send(h.post("/v1/act", &key, &json!({ "plan": [work, work] }))).await;
    let states: Vec<&Value> =
        answer["acks"].as_array().expect("acks").iter().map(|a| &a["status"]["state"]).collect();
    assert_eq!(states, ["started", "rejected"]);
    assert_eq!(answer["acks"][1]["status"]["error"]["code"], "queue_full");

    // The simulation refuses what it cannot do.
    let unknown = json!({ "name": "kernel.no_such_param", "value": 1 });
    let (status, answer) = send(h.post("/admin/params", ADMIN, &unknown)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        answer,
        json!({ "error": { "code": "refused", "message": "No parameter is named `kernel.no_such_param`." } })
    );
    let (status, answer) =
        send(h.request(Method::POST, "/admin/residents/999/freeze", ADMIN)).await;
    assert_eq!((status, code(&answer)), (StatusCode::CONFLICT, "refused"));
    let (status, answer) = send(h.post("/admin/params", ADMIN, &json!({ "name": "x" }))).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));

    // Statistics of the simulation, the store and the gateway.
    let (status, stats) = send(h.get("/admin/stats", ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (stats["sim"]["residents"].clone(), stats["sim"]["frame"].clone()),
        (json!(1), json!(2))
    );
    assert_eq!(stats["store"], json!({ "owners": 1, "residents": 1, "retired": 0, "webhooks": 0 }));
    assert_eq!(stats["gateway"]["ws_connections"], 0);
    assert_eq!(stats["gateway"]["webhooks"]["delivered"], 0);

    // Despawn removes the resident and ends its key.
    let despawn = format!("/admin/residents/{resident}/despawn");
    let (status, _) = send(h.request(Method::POST, &despawn, ADMIN)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = send(h.get("/v1/me", &key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (_, stats) = send(h.get("/admin/stats", ADMIN)).await;
    assert_eq!(stats["sim"]["residents"], 0);
    assert_eq!(stats["store"], json!({ "owners": 1, "residents": 0, "retired": 1, "webhooks": 0 }));
    let (status, answer) = send(h.request(Method::POST, &despawn, ADMIN)).await;
    assert_eq!((status, code(&answer)), (StatusCode::CONFLICT, "refused"));
}

#[tokio::test]
async fn admin_routes_are_closed_without_an_admin_token() {
    let h = Harness::with(unsup_gateway::GatewayConfig::default()).await;
    for token in ["", ADMIN] {
        let request = h.http.get(h.url("/admin/stats"));
        let request = if token.is_empty() { request } else { request.bearer_auth(token) };
        let (status, answer) = send(request).await;
        assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "admin_disabled"), "{token:?}");
    }
}

#[tokio::test]
async fn sign_up_and_resident_creation_check_their_input() {
    // Refused sign-ups count for the sign-up limit, so it is off here.
    let h = Harness::with_toml("[limits]\nsignups_per_hour = 0").await;
    let sign_up = |body: Value| send(h.http.post(h.url("/v1/owners")).json(&body));
    let cases = [
        (json!({}), "`name` is missing or empty."),
        (json!({ "name": "   " }), "`name` is missing or empty."),
        (json!({ "name": "x".repeat(65) }), "`name` has at most 64 bytes."),
        (json!({ "name": "A\nB" }), "`name` must not hold control characters."),
        (
            json!({ "name": "Ada", "credential": "c" }),
            "The provider `local` takes no `credential`.",
        ),
        (
            json!({ "name": "Ada", "provider": "github" }),
            "`provider` is not known. This server has: local.",
        ),
    ];
    for (body, message) in cases {
        let (status, answer) = sign_up(body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer, json!({ "error": { "code": "invalid_request", "message": message } }));
    }
    let (status, answer) = sign_up(json!({ "name": "Ada", "email": "a@b.c" })).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));

    let (_, token) = h.owner("Ada").await;
    let cases = [
        (json!({}), None),
        (json!({ "name": "" }), Some("`name` must not be empty.")),
        (json!({ "name": "x".repeat(65) }), Some("`name` has at most 64 bytes.")),
        (json!({ "name": "A\u{0}B" }), Some("`name` must not hold control characters.")),
        (json!({ "name": "Ada", "model": "m".repeat(65) }), Some("`model` has at most 64 bytes.")),
        (json!({ "name": "Ada", "city": 70000 }), None),
        (json!({ "name": "Ada", "money": 5 }), None),
        (
            json!({ "name": "Ada", "city": 9 }),
            Some("The simulation refused the resident. Check that `city` is the id of a city."),
        ),
    ];
    for (body, message) in cases {
        let (status, answer) = send(h.post("/v1/residents", &token, &body)).await;
        assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"), "{body}");
        if let Some(message) = message {
            assert_eq!(answer["error"]["message"], message, "{body}");
        }
    }
    // Nothing was made.
    let (_, listed) = send(h.get("/v1/residents", &token)).await;
    assert_eq!(listed["residents"], json!([]));
    // A city that exists, and a model that is only spaces.
    let (status, made) =
        send(h.post("/v1/residents", &token, &json!({ "name": "Ada", "city": 0, "model": " " })))
            .await;
    assert_eq!((status, made["resident"]["model"].clone()), (StatusCode::CREATED, Value::Null));
}

/// A provider that knows everyone as the same person, like an OAuth
/// service that returns one account.
struct OneAccount;

impl IdentityProvider for OneAccount {
    fn name(&self) -> &str {
        "one"
    }

    fn identify<'a>(&'a self, sign_up: &'a SignUp) -> IdentityFuture<'a> {
        Box::pin(async move {
            match sign_up.credential.as_deref() {
                Some("good-code") => Ok(Identity {
                    provider: "one".into(),
                    subject: "account-42".into(),
                    name: "The Account".into(),
                }),
                Some("banned") => Err(IdentityError::Denied("This account is banned.".into())),
                Some("down") => {
                    Err(IdentityError::Unavailable("The service does not answer.".into()))
                }
                _ => Err(IdentityError::Invalid("`credential` is not a valid code.".into())),
            }
        })
    }
}

#[tokio::test]
async fn an_identity_that_signs_up_again_gets_the_same_owner() {
    let providers: Vec<Arc<dyn IdentityProvider>> =
        vec![Arc::new(LocalSignUp), Arc::new(OneAccount)];
    let h = Harness::with_providers(config("[limits]\nsignups_per_hour = 0"), providers).await;
    let sign_up = |credential: &'static str| {
        let body = json!({ "provider": "one", "credential": credential });
        send(h.http.post(h.url("/v1/owners")).json(&body))
    };

    let (status, first) = sign_up("good-code").await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(first["owner"]["name"], "The Account");
    let (status, second) = sign_up("good-code").await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_eq!(second["owner"]["id"], first["owner"]["id"]);
    assert_ne!(second["token"], first["token"]);
    // Both tokens open the same account.
    let first_token = first["token"].as_str().expect("a token");
    let second_token = second["token"].as_str().expect("a token");
    let (resident, _) = h.resident(first_token, "One").await;
    let (_, listed) = send(h.get("/v1/residents", second_token)).await;
    assert_eq!(listed["residents"][0]["id"], resident);

    // The default provider is the first of the list.
    let (other, _) = h.owner("Local Person").await;
    assert_ne!(json!(other), first["owner"]["id"]);

    let (status, answer) = sign_up("bad").await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let (status, answer) = sign_up("banned").await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "sign_up_denied"));
    let (status, answer) = sign_up("down").await;
    assert_eq!((status, code(&answer)), (StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"));
}

#[tokio::test]
async fn join_md_is_served_from_the_configured_file() {
    let name = format!("unsup-join-{}.md", unsup_store::random_hex(8).expect("random bytes"));
    let path = std::env::temp_dir().join(name);
    let h = Harness::with_toml(&format!("join_file = {:?}", path.to_string_lossy())).await;

    // The file does not exist yet.
    let (status, answer) = send(h.http.get(h.url("/join.md"))).await;
    assert_eq!((status, code(&answer)), (StatusCode::NOT_FOUND, "not_found"));

    std::fs::write(&path, "# Join\n\nRead this to join.\n").expect("the join file");
    let response = h.http.get(h.url("/join.md")).send().await.expect("answer");
    let _ = std::fs::remove_file(&path);
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/markdown; charset=utf-8")
    );
    assert_eq!(response.text().await.expect("text"), "# Join\n\nRead this to join.\n");

    // Without a path in the configuration the route answers 404.
    let h = Harness::start().await;
    let (status, answer) = send(h.http.get(h.url("/join.md"))).await;
    assert_eq!((status, code(&answer)), (StatusCode::NOT_FOUND, "not_found"));
}

#[tokio::test]
async fn cors_allows_only_the_configured_origins() {
    let h = Harness::with_toml("cors_origins = [\"https://viewer.example\"]").await;
    let preflight = |origin: &'static str| {
        h.http
            .request(Method::OPTIONS, h.url("/v1/act"))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "POST")
            .header("Access-Control-Request-Headers", "authorization,content-type")
            .send()
    };
    let allowed = preflight("https://viewer.example").await.expect("answer");
    assert_eq!(allowed.status(), StatusCode::OK);
    let header = |name: &str| allowed.headers().get(name).and_then(|v| v.to_str().ok());
    assert_eq!(header("access-control-allow-origin"), Some("https://viewer.example"));
    assert_eq!(header("access-control-allow-methods"), Some("GET,POST,PUT,DELETE"));
    assert!(header("access-control-allow-headers").is_some_and(|h| h.contains("authorization")));

    let refused = preflight("https://evil.example").await.expect("answer");
    assert_eq!(refused.headers().get("access-control-allow-origin"), None);

    // `*` allows every origin.
    let h = Harness::with_toml("cors_origins = [\"*\"]").await;
    let response = h
        .http
        .get(h.url("/v1/catalogue"))
        .header("Origin", "https://anywhere.example")
        .send()
        .await
        .expect("answer");
    assert_eq!(
        response.headers().get("access-control-allow-origin").and_then(|v| v.to_str().ok()),
        Some("*")
    );
}
