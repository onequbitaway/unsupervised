//! The HTTPS API over real HTTP: sign-up, residents, keys, actions,
//! observations, waiting and routines.

mod common;

use std::time::{Duration, Instant};

use common::{Deadline, Harness, code, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

#[tokio::test]
async fn sign_up_to_action_end_to_end() {
    let h = Harness::start().await;

    let health = h.http.get(h.url("/healthz")).send().await.expect("healthz");
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.text().await.expect("text"), "ok");

    // An owner signs up. The token is in the answer, once.
    let sign_up = h.http.post(h.url("/v1/owners")).json(&json!({ "name": " Ada Lovelace " }));
    let (status, body) = send(sign_up).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        (body["owner"]["id"].clone(), body["owner"]["name"].clone()),
        (json!(1), json!("Ada Lovelace"))
    );
    let token = body["token"].as_str().expect("a token").to_string();
    assert!(token.starts_with("unsup_own_") && token.len() == 74, "{token}");

    // The owner makes a resident. The key is in the answer, once.
    let new = json!({ "name": "Ada", "model": "test-model" });
    let (status, body) = send(h.post("/v1/residents", &token, &new)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let key = body["key"].as_str().expect("a key").to_string();
    assert!(key.starts_with("unsup_res_") && key.len() == 74, "{key}");
    assert_eq!(
        body["resident"],
        json!({
            "id": 1,
            "name": "Ada",
            "model": "test-model",
            "created_at": body["resident"]["created_at"],
            "key_hint": format!("{}...", &key[..16]),
        })
    );

    // The list shows the resident and its hint, never the key.
    let response = h.get("/v1/residents", &token).send().await.expect("list");
    assert_eq!(response.status(), StatusCode::OK);
    let text = response.text().await.expect("text");
    assert!(!text.contains(&key), "the list holds the key");
    let listed: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(listed["limit"], 3);
    assert_eq!(listed["residents"].as_array().map(Vec::len), Some(1));
    assert_eq!(listed["residents"][0]["id"], 1);
    assert_eq!(listed["residents"][0]["key_hint"], format!("{}...", &key[..16]));

    // The key works for the resident.
    let (status, me) = send(h.get("/v1/me", &key)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me, json!({ "resident": 1, "name": "Ada", "model": "test-model", "owner": 1 }));

    let (status, before) = send(h.get("/v1/observe", &key)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(before["resident"], 1);
    assert_eq!(before["sections"]["self"]["name"], "Ada");
    assert_eq!(before["sections"]["self"]["activity"], Value::Null);
    assert_eq!((before["notices"].clone(), before["next_notice"].clone()), (json!([]), json!(1)));

    // One action: the answer has the acknowledgement and an observation.
    let action = json!({ "id": "job-1", "verb": "work", "args": { "ticks": 2 } });
    let (status, answer) = send(h.post("/v1/act", &key, &action)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"].as_array().map(Vec::len), Some(1));
    let ack = &answer["acks"][0];
    assert_eq!(
        (ack["request_id"].clone(), ack["verb"].clone(), ack["status"]["state"].clone()),
        (json!("job-1"), json!("work"), json!("started"))
    );
    assert_eq!(ack["status"]["ticks"], 2);
    assert_eq!(answer["observation"]["sections"]["self"]["activity"]["verb"], "work");

    // The action ends after two ticks. Its report is in the inbox.
    h.step(2).await;
    let (_, after) = send(h.get("/v1/observe?since=1&sections=reports", &key)).await;
    assert_eq!(
        after["sections"].as_object().map(|s| s.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["reports".to_string()])
    );
    let report = &after["sections"]["reports"][0];
    assert_eq!(report["action_id"], ack["action_id"]);
    assert_eq!(
        (report["request_id"].clone(), report["ok"].clone(), report["result"].clone()),
        (json!("job-1"), json!(true), json!({ "worked": 2 }))
    );
    assert_eq!(after["next_notice"], 2);
    // With the new cursor the report is not sent again.
    let (_, later) = send(h.get("/v1/observe?since=2", &key)).await;
    assert_eq!(later["sections"]["reports"], json!([]));

    // The catalogue needs no token.
    let (status, catalogue) = send(h.http.get(h.url("/v1/catalogue"))).await;
    assert_eq!(status, StatusCode::OK);
    let verbs = catalogue["verbs"].as_array().expect("verbs");
    let work = verbs.iter().find(|v| v["verb"] == "work").expect("the verb work");
    assert_eq!((work["module"].clone(), work["free"].clone()), (json!("town"), json!(false)));
    assert!(verbs.iter().any(|v| v["verb"] == "set_routines" && v["module"] == "kernel"));
}

#[tokio::test]
async fn each_authentication_failure_has_its_answer() {
    let h = Harness::start().await;
    let (_, owner_token) = h.owner("Ada").await;
    let (resident, key) = h.resident(&owner_token, "One").await;
    let (_, other_owner) = h.owner("Eve").await;

    // No token: every route with authentication says so.
    let routes = [
        (Method::POST, "/v1/residents"),
        (Method::GET, "/v1/residents"),
        (Method::POST, "/v1/residents/1/keys"),
        (Method::GET, "/v1/me"),
        (Method::POST, "/v1/act"),
        (Method::GET, "/v1/observe"),
        (Method::GET, "/v1/wait"),
        (Method::PUT, "/v1/routines"),
        (Method::PUT, "/v1/webhook"),
        (Method::GET, "/v1/ws"),
        (Method::POST, "/mcp"),
        (Method::DELETE, "/mcp"),
        (Method::POST, "/admin/residents/1/freeze"),
        (Method::GET, "/admin/stats"),
    ];
    for (method, path) in routes {
        let response = h.http.request(method.clone(), h.url(path)).send().await.expect("answer");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{method} {path}");
        assert_eq!(
            response.headers().get("www-authenticate").and_then(|v| v.to_str().ok()),
            Some("Bearer"),
            "{method} {path}"
        );
        let body: Value = response.json().await.expect("JSON");
        assert_eq!(code(&body), "unauthorized", "{method} {path}");
    }

    // A header that is not a bearer token.
    for value in ["Basic dXNlcjpwYXNz", "Bearer", "Bearer ", "Bearer a b", &key] {
        let request = h.http.get(h.url("/v1/me")).header("Authorization", value);
        let (status, body) = send(request).await;
        assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "unauthorized"), "{value:?}");
    }

    // A token that the server does not know.
    let last = key.chars().last().expect("a key has characters");
    let changed = format!("{}{}", &key[..key.len() - 1], if last == '0' { '1' } else { '0' });
    let never_made = format!("unsup_res_{}", "0".repeat(64));
    for token in ["12345", "unsup_res_short", &never_made, &changed] {
        let (status, body) = send(h.get("/v1/me", token)).await;
        assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "invalid_token"), "{token}");
    }
    let (status, body) = send(h.get("/v1/residents", &changed.replace("_res_", "_own_"))).await;
    assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "invalid_token"));

    // A good token of the wrong kind.
    for path in ["/v1/me", "/v1/observe", "/v1/wait?timeout=0"] {
        let (status, body) = send(h.get(path, &owner_token)).await;
        assert_eq!((status, code(&body)), (StatusCode::FORBIDDEN, "wrong_token_kind"), "{path}");
        assert_eq!(
            body["error"]["message"],
            "This route needs a resident key. You sent an owner token."
        );
    }
    let (status, body) = send(h.get("/v1/residents", &key)).await;
    assert_eq!((status, code(&body)), (StatusCode::FORBIDDEN, "wrong_token_kind"));
    let (status, body) = send(h.post("/v1/residents", &key, &json!({ "name": "Two" }))).await;
    assert_eq!((status, code(&body)), (StatusCode::FORBIDDEN, "wrong_token_kind"));

    // A good owner token for a resident of another owner.
    let path = format!("/v1/residents/{resident}/keys");
    let (status, body) = send(h.request(Method::POST, &path, &other_owner)).await;
    assert_eq!((status, code(&body)), (StatusCode::NOT_FOUND, "not_found"));
    // The key of the resident was not replaced.
    let (status, _) = send(h.get("/v1/me", &key)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn key_rotation_invalidates_the_old_key() {
    let h = Harness::start().await;
    let (_, owner_token) = h.owner("Ada").await;
    let (resident, old_key) = h.resident(&owner_token, "One").await;
    let (other, other_key) = h.resident(&owner_token, "Two").await;

    let path = format!("/v1/residents/{resident}/keys");
    let (status, body) = send(h.request(Method::POST, &path, &owner_token)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let new_key = body["key"].as_str().expect("a new key").to_string();
    assert_ne!(new_key, old_key);
    assert_eq!(
        body,
        json!({ "resident": resident, "key": new_key, "key_hint": format!("{}...", &new_key[..16]) })
    );

    let (status, body) = send(h.get("/v1/me", &old_key)).await;
    assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (status, body) = send(h.get("/v1/me", &new_key)).await;
    assert_eq!((status, body["resident"].clone()), (StatusCode::OK, json!(resident)));
    // The key of the other resident still works.
    let (status, body) = send(h.get("/v1/me", &other_key)).await;
    assert_eq!((status, body["resident"].clone()), (StatusCode::OK, json!(other)));
    let (_, listed) = send(h.get("/v1/residents", &owner_token)).await;
    assert_eq!(listed["residents"][0]["key_hint"], format!("{}...", &new_key[..16]));

    let (status, body) =
        send(h.request(Method::POST, "/v1/residents/999/keys", &owner_token)).await;
    assert_eq!((status, code(&body)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, body) =
        send(h.request(Method::POST, "/v1/residents/abc/keys", &owner_token)).await;
    assert_eq!((status, code(&body)), (StatusCode::BAD_REQUEST, "invalid_request"));
}

#[tokio::test]
async fn a_plan_runs_in_order_and_stops_at_a_rejected_request() {
    let h = Harness::start().await;
    let (resident, key) = h.agent("Ada").await;

    let plan = json!({ "plan": [
        { "verb": "work", "args": { "ticks": 2 } },
        { "verb": "work", "args": { "ticks": 1 }, "id": "second" },
        { "verb": "tell", "args": { "to": resident, "text": "note to self" } },
    ] });
    let (status, answer) = send(h.post("/v1/act", &key, &plan)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let states: Vec<&str> = answer["acks"]
        .as_array()
        .expect("acks")
        .iter()
        .map(|ack| ack["status"]["state"].as_str().expect("a state"))
        .collect();
    // The first starts, the second waits behind it, the free verb runs.
    assert_eq!(states, ["started", "queued", "done"]);
    assert_eq!(answer["acks"][1]["status"]["position"], 1);
    assert_eq!(answer["acks"][1]["request_id"], "second");
    assert_eq!(answer["observation"]["sections"]["self"]["queue"], 1);

    // The server stops at the request that the simulation rejects.
    let plan = json!({ "plan": [
        { "verb": "work", "args": { "ticks": 1 } },
        { "verb": "fly" },
        { "verb": "work", "args": { "ticks": 1 } },
    ] });
    let (_, answer) = send(h.post("/v1/act", &key, &plan)).await;
    let acks = answer["acks"].as_array().expect("acks");
    assert_eq!(acks.len(), 2, "{answer}");
    assert_eq!(acks[0]["status"], json!({ "state": "queued", "position": 2 }));
    assert_eq!(acks[1]["status"]["state"], "rejected");
    assert_eq!(acks[1]["status"]["error"]["code"], "unknown_verb");
    assert_eq!(answer["observation"]["sections"]["self"]["queue"], 2);

    // Three actions run: 2 + 1 + 1 ticks.
    h.step(4).await;
    let (_, after) = send(h.get("/v1/observe", &key)).await;
    let reports = after["sections"]["reports"].as_array().expect("reports");
    assert_eq!(
        reports.iter().map(|r| r["result"]["worked"].clone()).collect::<Vec<_>>(),
        [json!(2), json!(1), json!(1)]
    );
    assert_eq!(after["notices"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        after["notices"][0]["data"],
        json!({ "untrusted": true, "from": resident, "text": "note to self" })
    );
    assert_eq!(after["sections"]["self"]["activity"], Value::Null);
}

#[tokio::test]
async fn act_without_an_observation_returns_only_the_acknowledgements() {
    let h = Harness::start().await;
    let (resident, key) = h.agent("Ada").await;
    let observations = || async { h.sim.stats().await.expect("stats").observations };
    let work = json!({ "verb": "work", "args": { "ticks": 2 }, "id": "w1" });

    // The default: the acknowledgements and a fresh observation.
    let built = observations().await;
    let (status, answer) = send(h.post("/v1/act", &key, &work)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let fields: Vec<&String> = answer.as_object().expect("an object").keys().collect();
    assert_eq!(fields, ["acks", "observation"]);
    assert_eq!(answer["observation"]["sections"]["self"]["activity"]["verb"], "work");
    assert_eq!(observations().await, built + 1);
    // `observe=true` is the default.
    let tell = json!({ "verb": "tell", "args": { "to": resident, "text": "hello" } });
    let (_, answer) = send(h.post("/v1/act?observe=true", &key, &tell)).await;
    assert_eq!(answer["observation"]["resident"], resident);
    assert_eq!(observations().await, built + 2);

    // `observe=false`: only the acknowledgements, and the simulation
    // builds no observation.
    let built = observations().await;
    let queued = json!({ "verb": "work", "args": { "ticks": 1 }, "id": "w2" });
    let (status, answer) = send(h.post("/v1/act?observe=false", &key, &queued)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer,
        json!({ "acks": [{
            "request_id": "w2", "action_id": answer["acks"][0]["action_id"], "verb": "work",
            "status": { "state": "queued", "position": 1 },
        }] })
    );
    // A plan: one acknowledgement for each request that was sent.
    let plan = json!({ "plan": [
        { "verb": "tell", "args": { "to": resident, "text": "one" } },
        { "verb": "fly" },
        { "verb": "tell", "args": { "to": resident, "text": "never sent" } },
    ] });
    let (status, answer) = send(h.post("/v1/act?observe=false", &key, &plan)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let fields: Vec<&String> = answer.as_object().expect("an object").keys().collect();
    assert_eq!(fields, ["acks"]);
    let states: Vec<&Value> = answer["acks"]
        .as_array()
        .expect("acks")
        .iter()
        .map(|ack| &ack["status"]["state"])
        .collect();
    assert_eq!(states, ["done", "rejected"]);
    assert_eq!(observations().await, built, "no observation was built");
    // The actions were taken: a later observation shows them.
    let (_, seen) = send(h.get("/v1/observe", &key)).await;
    assert_eq!(seen["sections"]["self"]["queue"], 1);

    // The checks of the request are the same: the query, the body, the
    // text filters.
    for query in ["observe=no", "observe=", "observe=false&detail=huge", "observe=false&x=1"] {
        let (status, answer) = send(h.post(&format!("/v1/act?{query}"), &key, &work)).await;
        assert_eq!(
            (status, code(&answer)),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{query}"
        );
    }
    let (status, answer) =
        send(h.post("/v1/act?observe=false", &key, &json!({ "plan": [] }))).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    // The options of the observation are read and not used.
    let (status, answer) =
        send(h.post("/v1/act?observe=false&detail=full&since=3&sections=self", &key, &tell)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer.get("observation"), None);
}

#[tokio::test]
async fn act_refuses_a_body_with_a_wrong_shape() {
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;
    let too_long: Vec<Value> = (0..17).map(|_| json!({ "verb": "work" })).collect();
    let cases = [
        (json!([1, 2]), "The body must be an action request or `{ \"plan\": [...] }`."),
        (json!({ "plan": [] }), "`plan` must hold at least one action request."),
        (json!({ "plan": too_long }), "`plan` holds at most 16 action requests."),
        (json!({ "plan": 5 }), "`plan` must be a list of action requests."),
        (
            json!({ "plan": [{ "verb": "work" }], "verb": "work" }),
            "A plan body has only the field `plan`, not `verb`.",
        ),
        (
            json!({ "verb": "work", "arg": {} }),
            "The request has the unknown field `arg`. An action has `id`, `verb` and `args`.",
        ),
        (json!({ "args": {} }), "The request is not valid: missing field `verb`."),
        (
            json!({ "verb": "Work Hard" }),
            "The request: `verb` has 1 to 64 lowercase letters, digits or `_`.",
        ),
        (json!({ "verb": "work", "args": [2] }), "The request: `args` must be an object."),
        (json!({ "verb": "work", "id": "" }), "The request: `id` has 1 to 64 bytes."),
        (
            json!({ "plan": [{ "verb": "work" }, "work"] }),
            "plan[1] must be an object with `verb` and `args`.",
        ),
    ];
    for (body, message) in cases {
        let (status, answer) = send(h.post("/v1/act", &key, &body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(answer, json!({ "error": { "code": "invalid_request", "message": message } }));
    }
    // Nothing was sent to the simulation.
    let (_, after) = send(h.get("/v1/observe", &key)).await;
    assert_eq!(after["sections"]["self"]["activity"], Value::Null);

    // Text that is not JSON, and JSON with the wrong content type.
    let raw = h.request(Method::POST, "/v1/act", &key);
    let (status, answer) =
        send(raw.header("Content-Type", "application/json").body("{ not json")).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let raw = h.request(Method::POST, "/v1/act", &key);
    let (status, answer) =
        send(raw.header("Content-Type", "text/plain").body(r#"{"verb":"work"}"#)).await;
    assert_eq!(
        (status, code(&answer)),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
    );

    // A path of the gateway with a method that it does not take.
    let (status, answer) = send(h.request(Method::DELETE, "/v1/act", &key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"));
}

#[tokio::test]
async fn observe_checks_its_query() {
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;

    let (status, full) = send(h.get("/v1/observe?detail=full", &key)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["sections"]["self"]["routines"], json!([]));
    let (_, brief) = send(h.get("/v1/observe?detail=brief", &key)).await;
    assert_eq!(brief["sections"]["self"].get("routines"), None);
    let (_, picked) = send(h.get("/v1/observe?sections=self,nearby", &key)).await;
    let names: Vec<&String> = picked["sections"].as_object().expect("sections").keys().collect();
    assert_eq!(names, ["nearby", "self"]);
    let (_, none) = send(h.get("/v1/observe?sections=", &key)).await;
    assert_eq!(none["sections"], json!({}));

    let many = (0..17).map(|i| format!("s{i}")).collect::<Vec<_>>().join(",");
    let cases = [
        "detail=huge".to_string(),
        "since=-1".to_string(),
        "since=soon".to_string(),
        "sections=Self".to_string(),
        "sections=self,,nearby".to_string(),
        format!("sections={many}"),
        "verbose=1".to_string(),
    ];
    for query in cases {
        let (status, answer) = send(h.get(&format!("/v1/observe?{query}"), &key)).await;
        assert_eq!(
            (status, code(&answer)),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{query}"
        );
    }
}

/// Waits until the simulation holds `count` waiting calls.
async fn waiting(h: &Harness, count: u64) {
    let deadline = Deadline::start();
    while h.sim.stats().await.expect("stats").waiting != count {
        deadline.again().await;
    }
}

#[tokio::test]
async fn wait_returns_when_a_notice_arrives() {
    let h = Harness::start().await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    h.step(1).await;

    // The timeout is long: without the wake the test fails by time.
    let call = tokio::spawn(send(h.get("/v1/wait?timeout=60", &ada_key)));
    waiting(&h, 1).await;
    h.step(2).await;
    assert_eq!(h.sim.stats().await.expect("stats").waiting, 1, "frames without news do not wake");

    let started = Instant::now();
    h.tell(&bob_key, ada, "hello Ada").await;
    h.step(1).await;
    let (status, observation) = call.await.expect("the call ran");
    assert!(started.elapsed() < Duration::from_secs(30), "the notice woke the call");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(observation["resident"], ada);
    assert_eq!(
        observation["notices"],
        json!([{
            "seq": 1,
            "tick": observation["notices"][0]["tick"],
            "kind": "town.told",
            "from": bob,
            "data": { "untrusted": true, "from": bob, "text": "hello Ada" },
        }])
    );
    assert_eq!(observation["next_notice"], 2);
    waiting(&h, 0).await;

    // With the cursor of an earlier observation, news that is there
    // already answers at once.
    let (_, again) = send(h.get("/v1/wait?timeout=60&since=1", &ada_key)).await;
    assert_eq!(again["notices"][0]["seq"], 1);
}

#[tokio::test]
async fn wait_returns_without_news_when_the_time_is_over() {
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;

    let started = Instant::now();
    let (status, observation) = send(h.get("/v1/wait?timeout=0.2&since=1", &key)).await;
    assert!(started.elapsed() >= Duration::from_millis(200), "the call waited its time");
    assert_eq!(status, StatusCode::OK);
    assert_eq!(observation["notices"], json!([]));
    assert_eq!(observation["sections"]["reports"], json!([]));
    assert_eq!(observation["next_notice"], 1);

    // A timeout of zero is a poll.
    let (status, poll) = send(h.get("/v1/wait?timeout=0", &key)).await;
    assert_eq!((status, poll["notices"].clone()), (StatusCode::OK, json!([])));

    for query in ["timeout=61", "timeout=-1", "timeout=soon", "timeout=NaN", "kinds=Bad!", "x=1"] {
        let (status, answer) = send(h.get(&format!("/v1/wait?{query}"), &key)).await;
        assert_eq!(
            (status, code(&answer)),
            (StatusCode::BAD_REQUEST, "invalid_request"),
            "{query}"
        );
    }
}

#[tokio::test]
async fn wait_filters_by_kind_and_limits_open_calls() {
    let h = Harness::with_toml("[limits]\nwaits_per_resident = 1").await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    h.step(1).await;

    let call = tokio::spawn(send(h.get("/v1/wait?timeout=60&kinds=town.alarm,report", &ada_key)));
    waiting(&h, 1).await;

    // One call is open, and the limit is one.
    let (status, answer) = send(h.get("/v1/wait?timeout=60", &ada_key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::TOO_MANY_REQUESTS, "too_many_connections"));

    // A notice of another kind does not end the call.
    h.tell(&bob_key, ada, "just talk").await;
    h.step(1).await;
    assert_eq!(h.sim.stats().await.expect("stats").waiting, 1);

    let alarm =
        json!({ "verb": "tell", "args": { "to": ada, "text": "fire", "kind": "town.alarm" } });
    let (status, _) = send(h.post("/v1/act", &bob_key, &alarm)).await;
    assert_eq!(status, StatusCode::OK);
    h.step(1).await;
    let (_, observation) = call.await.expect("the call ran");
    let kinds: Vec<&str> = observation["notices"]
        .as_array()
        .expect("notices")
        .iter()
        .map(|n| n["kind"].as_str().expect("a kind"))
        .collect();
    assert_eq!(kinds, ["town.told", "town.alarm"], "everything since the call comes with the wake");

    // The place is free again.
    let (status, _) = send(h.get("/v1/wait?timeout=0", &ada_key)).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn routines_are_replaced_and_checked() {
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;
    let routine = |name: &str, verb: &str| json!({ "name": name, "when": { "type": "always" }, "then": { "verb": verb, "args": { "ticks": 1 } } });

    let (status, answer) =
        send(h.put("/v1/routines", &key, &json!({ "routines": [routine("keep busy", "work")] })))
            .await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "routines": 1 })));
    let (_, full) = send(h.get("/v1/observe?detail=full", &key)).await;
    let routines = full["sections"]["self"]["routines"].as_array().expect("routines");
    assert_eq!(routines.len(), 1);
    assert_eq!(routines[0]["name"], "keep busy");
    assert_eq!(full["sections"]["self"]["activity"], Value::Null);

    // The resident is idle, so the routine starts its action.
    h.step(1).await;
    let (_, busy) = send(h.get("/v1/observe", &key)).await;
    assert_eq!(busy["sections"]["self"]["activity"]["verb"], "work");

    // The simulation checks the routines. Its refusal keeps the old ones.
    let (status, answer) =
        send(h.put("/v1/routines", &key, &json!({ "routines": [routine("up", "fly")] }))).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "unknown_verb"));
    let nine: Vec<Value> = (0..9).map(|i| routine(&format!("r{i}"), "work")).collect();
    let (status, answer) = send(h.put("/v1/routines", &key, &json!({ "routines": nine }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        answer,
        json!({ "error": { "code": "invalid_args", "message": "A resident has at most 8 routines." } })
    );
    let (_, full) = send(h.get("/v1/observe?detail=full", &key)).await;
    assert_eq!(full["sections"]["self"]["routines"][0]["name"], "keep busy");

    for body in
        [json!({}), json!({ "routines": [{ "name": "x" }] }), json!({ "routines": [], "x": 1 })]
    {
        let (status, answer) = send(h.put("/v1/routines", &key, &body)).await;
        assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"), "{body}");
    }
    // A routine and its action take only the fields that they have. A
    // wrong name does not pass as an absent field.
    let mut misspelled = routine("r", "work");
    misspelled["then"] = json!({ "verb": "work", "arg": { "ticks": 1 } });
    let mut extra = routine("r", "work");
    extra["every"] = json!(5);
    let cases = [
        (
            json!([routine("ok", "work"), misspelled]),
            "routines[1].then has the unknown field `arg`. An action has `id`, `verb` and `args`.",
        ),
        (
            json!([extra]),
            "routines[0] has the unknown field `every`. A routine has `name`, `when` and `then`.",
        ),
        (json!(["work"]), "routines[0] must be an object with `name`, `when` and `then`."),
    ];
    for (routines, message) in cases {
        let (status, answer) =
            send(h.put("/v1/routines", &key, &json!({ "routines": routines }))).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(answer, json!({ "error": { "code": "invalid_request", "message": message } }));
    }
    // The refused lists changed nothing.
    let (_, full) = send(h.get("/v1/observe?detail=full", &key)).await;
    assert_eq!(full["sections"]["self"]["routines"][0]["name"], "keep busy");

    // An empty list removes the routines.
    let (status, answer) = send(h.put("/v1/routines", &key, &json!({ "routines": [] }))).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "routines": 0 })));
    let (_, full) = send(h.get("/v1/observe?detail=full", &key)).await;
    assert_eq!(full["sections"]["self"]["routines"], json!([]));
}
