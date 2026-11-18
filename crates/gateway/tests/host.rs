//! A route that the host adds for owners, over real HTTP: the gateway
//! authenticates and limits, the route of the host answers.

mod common;

use std::sync::Arc;
use std::sync::Mutex;

use common::{ADMIN, Harness, code, config, send, soon, world};
use reqwest::{Method, StatusCode};
use serde_json::json;
use unsup_gateway::{
    ApiError, Author, Gateway, GatewayConfig, OwnerCall, OwnerRoute, OwnerRouteFuture, StartError,
    TextUse,
};
use unsup_protocol::OwnerId;
use unsup_store::MemoryStore;

/// A route of a host: it keeps each call, checks the texts of the body as
/// arguments of the verb `post`, and answers `201` with the name of the
/// owner and the body.
#[derive(Default)]
struct Board {
    calls: Mutex<Vec<OwnerCall>>,
}

impl OwnerRoute for Board {
    fn call<'a>(&'a self, gateway: &'a Gateway, call: OwnerCall) -> OwnerRouteFuture<'a> {
        Box::pin(async move {
            self.calls.lock().expect("the lock").push(call.clone());
            let author = Author::Owner(call.owner.id);
            gateway.check_texts(author, "post", &call.body).await?;
            if call.body["full"] == true {
                return Err(ApiError::new(
                    StatusCode::CONFLICT,
                    "board_full",
                    "The board is full.",
                ));
            }
            Ok((StatusCode::CREATED, json!({ "by": call.owner.name, "posted": call.body })))
        })
    }
}

/// A route of a host that checks one text of its own: a line that it
/// makes from the name of the owner.
struct Greeting;

impl OwnerRoute for Greeting {
    fn call<'a>(&'a self, gateway: &'a Gateway, call: OwnerCall) -> OwnerRouteFuture<'a> {
        Box::pin(async move {
            let line = format!("{} says {}", call.owner.name, call.body["word"]);
            let usage = TextUse::Speech { verb: "post" };
            gateway.check_text(Author::Owner(call.owner.id), usage, "The greeting", &line).await?;
            Ok((StatusCode::OK, json!({ "line": line })))
        })
    }
}

/// A route of a host that never answers, as one that waits for the
/// simulation.
struct Stuck;

impl OwnerRoute for Stuck {
    fn call<'a>(&'a self, _gateway: &'a Gateway, _call: OwnerCall) -> OwnerRouteFuture<'a> {
        Box::pin(std::future::pending())
    }
}

/// Starts a gateway with the routes of a host in front of the small world.
async fn with_routes(
    config: GatewayConfig,
    routes: Vec<(&str, Arc<dyn OwnerRoute>)>,
) -> Result<Harness, StartError> {
    let (sim, runtime) = world();
    let store = Arc::new(MemoryStore::new());
    let mut builder = Gateway::builder(sim.clone(), store.clone(), config);
    for (path, route) in routes {
        builder = builder.owner_route(path, route);
    }
    let gateway = builder.start().await?;
    Ok(Harness::around(sim, runtime, store, gateway).await)
}

const RULES: &str = "[moderation]\nwords = [\"badword\"]\ntext_verbs = [\"post\"]";

#[tokio::test]
async fn the_host_answers_an_owner_and_the_gateway_guards_the_door() {
    let board = Arc::new(Board::default());
    let h = with_routes(config(RULES), vec![("/v1/board", board.clone())])
        .await
        .expect("the gateway starts");
    let (owner, token) = h.owner("Ada").await;
    let (_, key) = h.resident(&token, "Ada's resident").await;

    // The route of the host gets the owner of the token and the body.
    let body = json!({ "title": "Bread for the fair", "tags": ["food", "fair"], "count": 3 });
    let (status, answer) = send(h.post("/v1/board", &token, &body)).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(answer, json!({ "by": "Ada", "posted": body }));
    {
        let calls = board.calls.lock().expect("the lock");
        assert_eq!(calls.len(), 1);
        assert_eq!((calls[0].owner.id, calls[0].owner.name.as_str()), (OwnerId(owner), "Ada"));
        assert_eq!(calls[0].body, body);
    }
    // An error of the route has the format of the API.
    let (status, answer) = send(h.post("/v1/board", &token, &json!({ "full": true }))).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(
        answer,
        json!({ "error": { "code": "board_full", "message": "The board is full." } })
    );
    assert_eq!(board.calls.lock().expect("the lock").len(), 2);

    // The gateway refuses each of these, and the route is not called.
    let no_token = h.http.post(h.url("/v1/board")).json(&body);
    let (status, answer) = send(no_token).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "unauthorized"));
    let (status, answer) = send(h.post("/v1/board", "unsup_own_wrong", &body)).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (status, answer) = send(h.post("/v1/board", &key, &body)).await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "wrong_token_kind"));
    let (status, answer) = send(h.post("/v1/board", ADMIN, &body)).await;
    assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (status, answer) = send(h.post("/v1/board?draft=1", &token, &body)).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let raw = h.request(Method::POST, "/v1/board", &token);
    let (status, answer) = send(raw.header("Content-Type", "text/plain").body("{}")).await;
    assert_eq!(
        (status, code(&answer)),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
    );
    let raw = h.request(Method::POST, "/v1/board", &token);
    let (status, answer) =
        send(raw.header("Content-Type", "application/json").body("{ not json")).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let (status, answer) = send(h.get("/v1/board", &token)).await;
    assert_eq!((status, code(&answer)), (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"));
    assert_eq!(board.calls.lock().expect("the lock").len(), 2, "the route was not called");

    // A banned owner does not reach the route.
    let ban = h.request(Method::POST, &format!("/admin/owners/{owner}/ban"), ADMIN);
    assert_eq!(send(ban).await.0, StatusCode::OK);
    let (status, answer) = send(h.post("/v1/board", &token, &body)).await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "banned"));
    assert_eq!(board.calls.lock().expect("the lock").len(), 2, "the route was not called");
}

#[tokio::test]
async fn the_text_of_a_host_route_passes_the_text_filters() {
    let board = Arc::new(Board::default());
    let routes: Vec<(&str, Arc<dyn OwnerRoute>)> =
        vec![("/v1/board", board.clone()), ("/v1/greeting", Arc::new(Greeting))];
    let h = with_routes(config(RULES), routes).await.expect("the gateway starts");
    let (_, token) = h.owner("Ada").await;

    // Each string of the body is a text, also deep inside it, and so is
    // the name of a field inside a value. The answer names the place.
    let cases = [
        (json!({ "title": "a Badword here" }), "`title`"),
        (json!({ "tags": ["fine", "badword"] }), "`tags[1]`"),
        (
            json!({ "goal": { "event": "x.y", "fields": { "item": "badword" } } }),
            "`goal.fields.item`",
        ),
        (json!({ "goal": { "badword": 1 } }), "`a field name in goal`"),
    ];
    for (body, place) in cases {
        let (status, answer) = send(h.post("/v1/board", &token, &body)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let message = format!("{place}: The text holds a word that this server does not permit.");
        assert_eq!(answer, json!({ "error": { "code": "text_refused", "message": message } }));
    }
    // One text that the route made, with the place that the route names.
    let (status, answer) = send(h.post("/v1/greeting", &token, &json!({ "word": 1 }))).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "line": "Ada says 1" })));
    let (status, answer) =
        send(h.post("/v1/greeting", &token, &json!({ "word": "badword" }))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        answer,
        json!({ "error": {
            "code": "text_refused",
            "message": "The greeting: The text holds a word that this server does not permit.",
        } })
    );
    // The dashboard counts the texts that the filters refused.
    let (status, dashboard) = send(h.get("/admin/dashboard", ADMIN)).await;
    assert_eq!(status, StatusCode::OK, "{dashboard}");
    assert_eq!(dashboard["moderation"]["refused_by_rule"], json!({ "word": 5 }));

    // A number and a boolean are no text.
    let (status, answer) =
        send(h.post("/v1/board", &token, &json!({ "count": 7, "open": true }))).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
}

#[tokio::test]
async fn a_host_route_has_the_limits_of_the_gateway() {
    let board = Arc::new(Board::default());
    let limits = "[limits]\nmax_body_bytes = 256\nkey_requests_per_minute = 60\nkey_burst = 3";
    let h = with_routes(config(limits), vec![("/v1/board", board.clone())])
        .await
        .expect("the gateway starts");
    let (_, token) = h.owner("Ada").await;

    // A body above the limit of the gateway.
    let large = json!({ "title": "x".repeat(300) });
    let (status, answer) = send(h.post("/v1/board", &token, &large)).await;
    assert_eq!((status, code(&answer)), (StatusCode::PAYLOAD_TOO_LARGE, "payload_too_large"));
    assert_eq!(board.calls.lock().expect("the lock").len(), 0);

    // The limit of requests of one owner token: three at once. The first
    // request above took one of them.
    for _ in 0..2 {
        let (status, answer) = send(h.post("/v1/board", &token, &json!({}))).await;
        assert_eq!(status, StatusCode::CREATED, "{answer}");
    }
    let response = h.post("/v1/board", &token, &json!({})).send().await.expect("an answer");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers().get("retry-after").and_then(|value| value.to_str().ok()),
        Some("1")
    );
    assert_eq!(board.calls.lock().expect("the lock").len(), 2);
}

#[tokio::test]
async fn a_host_route_that_waits_ends_when_the_gateway_stops() {
    let h = with_routes(config(""), vec![("/v1/stuck", Arc::new(Stuck))])
        .await
        .expect("the gateway starts");
    let (_, token) = h.owner("Ada").await;
    let call = tokio::spawn(send(h.post("/v1/stuck", &token, &json!({}))));
    // The call is inside the route when the gateway counts its request.
    // The stop comes later or earlier: both end the call.
    h.gateway.shutdown();
    let (status, answer) = soon(call).await.expect("the task ran");
    assert_eq!((status, code(&answer)), (StatusCode::SERVICE_UNAVAILABLE, "shutdown"));
}

#[tokio::test]
async fn a_path_of_a_host_route_must_be_plain_and_single() {
    for path in ["v1/board", "/v1/board/", "/v1//board", "/v1/board/{id}", "/v1/*rest", "/", ""] {
        let error = with_routes(config(""), vec![(path, Arc::new(Stuck))])
            .await
            .map(|_| ())
            .expect_err("a path that is not plain");
        let message = format!(
            "the gateway configuration is not valid: the path `{path}` of a route of the host \
             is not a plain path such as `/v1/missions`"
        );
        assert_eq!(error.to_string(), message);
    }
    let twice: Vec<(&str, Arc<dyn OwnerRoute>)> =
        vec![("/v1/board", Arc::new(Stuck)), ("/v1/board", Arc::new(Board::default()))];
    let error = with_routes(config(""), twice).await.map(|_| ()).expect_err("one path, two routes");
    assert_eq!(
        error.to_string(),
        "the gateway configuration is not valid: the path `/v1/board` has two routes of the host"
    );
    // Two paths, each with its route.
    let two: Vec<(&str, Arc<dyn OwnerRoute>)> =
        vec![("/v1/board", Arc::new(Board::default())), ("/v1/stuck-2.x", Arc::new(Stuck))];
    let h = with_routes(config(""), two).await.expect("the gateway starts");
    let (_, token) = h.owner("Ada").await;
    let (status, answer) = send(h.post("/v1/board", &token, &json!({ "n": 1 }))).await;
    assert_eq!((status, answer["by"].clone()), (StatusCode::CREATED, json!("Ada")));
}
