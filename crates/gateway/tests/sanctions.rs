//! Sanctions over real HTTP: mute, freeze and ban, their removal, and a
//! restart of the gateway on the same store and world.

mod common;

use std::sync::Arc;

use common::market::market;
use common::{ADMIN, Deadline, Harness, code, config, send, soon};
use futures::{SinkExt, StreamExt};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error, Message};
use unsup_store::{
    Identity, IdentityError, IdentityFuture, IdentityProvider, LocalSignUp, SignUp, Store,
};

type Socket = common::AgentSocket;

async fn try_socket(h: &Harness, key: &str) -> Result<Socket, Error> {
    let mut request = format!("ws://{}/v1/ws", h.addr).into_client_request().expect("a request");
    let value = format!("Bearer {key}").parse().expect("a header value");
    request.headers_mut().insert("Authorization", value);
    tokio_tungstenite::connect_async(request).await.map(|(socket, _)| socket)
}

/// Reads the next text frame as JSON.
async fn next_json(socket: &mut Socket) -> Value {
    common::next_frame(socket).await
}

async fn admin(h: &Harness, path: &str) -> (StatusCode, Value) {
    send(h.request(Method::POST, path, ADMIN)).await
}

async fn admin_with(h: &Harness, path: &str, body: Value) -> (StatusCode, Value) {
    send(h.post(path, ADMIN, &body)).await
}

async fn sanctions(h: &Harness) -> Value {
    let (status, list) = send(h.get("/admin/sanctions", ADMIN)).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    list["sanctions"].clone()
}

fn tell(to: u64, text: &str) -> Value {
    json!({ "to": to, "text": text })
}

const MUTED: &str =
    "An administrator muted this resident. It cannot send text. Every other action works.";
const BANNED: &str = "An administrator banned the owner of this token. No token of the owner and \
                      no key of its residents works.";

#[tokio::test]
async fn a_muted_resident_cannot_send_text_on_any_way_in() {
    let h = market(config("")).await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;

    // Routines from before the mute: the first one speaks, the second
    // one does not. The first routine that holds is the one that runs.
    let talk = json!({ "name": "talk", "when": { "type": "always" },
        "then": { "verb": "tell", "args": tell(ada, "from a routine") } });
    let toil = json!({ "name": "toil", "when": { "type": "always" },
        "then": { "verb": "work", "args": { "ticks": 5 } } });
    let (status, _) =
        send(h.put("/v1/routines", &bob_key, &json!({ "routines": [talk, toil] }))).await;
    assert_eq!(status, StatusCode::OK);

    let mute = format!("/admin/residents/{bob}/mute");
    let (status, answer) = admin_with(&h, &mute, json!({ "reason": "spam in the square" })).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    // A second mute changes nothing: the first reason stays.
    let (status, _) = admin_with(&h, &mute, json!({ "reason": "again" })).await;
    assert_eq!(status, StatusCode::OK);
    let list = sanctions(&h).await;
    assert_eq!(list.as_array().map(Vec::len), Some(1), "{list}");
    assert_eq!(
        (list[0]["kind"].clone(), list[0]["resident"].clone(), list[0]["reason"].clone()),
        (json!("mute"), json!(bob), json!("spam in the square"))
    );
    assert!(list[0]["created_at"].as_u64().is_some_and(|at| at > 1_600_000_000), "{list}");

    // The mute removed the routine that speaks, and kept the other.
    let (_, seen) = send(h.get("/v1/observe?detail=full&sections=self", &bob_key)).await;
    let names: Vec<&Value> = seen["sections"]["self"]["routines"]
        .as_array()
        .expect("routines")
        .iter()
        .map(|routine| &routine["name"])
        .collect();
    assert_eq!(names, ["toil"]);

    // HTTP: one action, a plan with speech, and routines with speech.
    let muted = json!({ "error": { "code": "muted", "message": MUTED } });
    let (status, answer) = h.act(&bob_key, "tell", tell(ada, "hello")).await;
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &muted));
    let plan = json!({ "plan": [
        { "verb": "earn", "args": { "cents": 5 } },
        { "verb": "tell", "args": tell(ada, "in a plan") },
    ] });
    let (status, answer) = send(h.post("/v1/act", &bob_key, &plan)).await;
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &muted));
    let (status, answer) =
        send(h.put("/v1/routines", &bob_key, &json!({ "routines": [talk] }))).await;
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &muted));
    // The WebSocket.
    let mut ws = try_socket(&h, &bob_key).await.expect("the socket opens");
    let frame = json!({ "op": "act", "request": { "verb": "tell", "args": tell(ada, "ws") } });
    ws.send(Message::Text(frame.to_string().into())).await.expect("the frame is sent");
    assert_eq!(
        next_json(&mut ws).await,
        json!({ "op": "error", "error": { "code": "muted", "message": MUTED } })
    );
    // The MCP tool.
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "act", "arguments": { "verb": "tell", "args": tell(ada, "mcp") },
            "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                       "io.modelcontextprotocol/clientCapabilities": {} } } });
    let request = h
        .post("/mcp", &bob_key, &body)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", "act");
    let (_, answer) = send(request).await;
    assert_eq!(answer["result"]["isError"], true, "{answer}");
    assert_eq!(answer["result"]["content"][0]["text"], format!("muted: {MUTED}"));

    // Everything without speech works, and the plan above did nothing.
    h.done(&bob_key, "earn", json!({ "cents": 7 })).await;
    let (_, seen) = send(h.get("/v1/observe", &bob_key)).await;
    assert_eq!(seen["sections"]["self"]["cash"], 7);
    // Another resident is not muted.
    h.done(&ada_key, "tell", tell(bob, "hello Bob")).await;
    h.step(1).await;
    // Ada heard nothing from Bob: not from a request, not from a routine.
    let (_, inbox) = send(h.get("/v1/observe?since=1", &ada_key)).await;
    assert_eq!(inbox["notices"], json!([]));

    // The mute is lifted: Bob speaks again.
    let (status, answer) = admin(&h, &format!("/admin/residents/{bob}/unmute")).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    assert_eq!(sanctions(&h).await, json!([]));
    h.done(&bob_key, "tell", tell(ada, "I am back")).await;
    h.step(1).await;
    let (_, inbox) = send(h.get("/v1/observe?since=1", &ada_key)).await;
    assert_eq!(inbox["notices"][0]["data"]["text"], "I am back");

    // A resident that does not exist cannot be muted, and a reason has
    // limits.
    let (status, answer) = admin(&h, "/admin/residents/999/mute").await;
    assert_eq!((status, code(&answer)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, answer) = admin_with(&h, &mute, json!({ "reason": "x".repeat(501) })).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let (status, answer) = admin_with(&h, &mute, json!({ "because": "typo" })).await;
    assert_eq!((status, code(&answer)), (StatusCode::BAD_REQUEST, "invalid_request"));
    assert_eq!(sanctions(&h).await, json!([]));
}

/// A provider that knows everyone as the same account.
struct OneAccount;

impl IdentityProvider for OneAccount {
    fn name(&self) -> &str {
        "one"
    }

    fn identify<'a>(&'a self, _sign_up: &'a SignUp) -> IdentityFuture<'a> {
        Box::pin(async {
            Ok::<_, IdentityError>(Identity {
                provider: "one".into(),
                subject: "account-42".into(),
                name: "The Account".into(),
            })
        })
    }
}

#[tokio::test]
async fn a_ban_stops_every_token_and_key_of_the_owner() {
    let providers: Vec<Arc<dyn IdentityProvider>> =
        vec![Arc::new(LocalSignUp), Arc::new(OneAccount)];
    let h = Harness::with_providers(config("[limits]\nsignups_per_hour = 0"), providers).await;
    let sign_in = || send(h.http.post(h.url("/v1/owners")).json(&json!({ "provider": "one" })));
    let (status, signed) = sign_in().await;
    assert_eq!(status, StatusCode::CREATED, "{signed}");
    let owner = signed["owner"]["id"].as_u64().expect("an owner id");
    let token = signed["token"].as_str().expect("a token").to_string();
    let (one, one_key) = h.resident(&token, "One").await;
    let (_, two_key) = h.resident(&token, "Two").await;
    let (_, other_key) = h.agent("Other").await;

    // An open socket and a waiting call of the owner that gets banned.
    let mut ws = try_socket(&h, &one_key).await.expect("the socket opens");
    ws.send(Message::Text(r#"{"op":"ping"}"#.into())).await.expect("the frame is sent");
    assert_eq!(next_json(&mut ws).await, json!({ "op": "pong" }));
    let waiting = tokio::spawn(send(h.get("/v1/wait?timeout=60", &two_key)));
    let deadline = Deadline::start();
    while h.sim.stats().await.expect("stats").waiting != 1 {
        deadline.again().await;
    }

    let ban = format!("/admin/owners/{owner}/ban");
    let (status, answer) = admin_with(&h, &ban, json!({ "reason": "sold accounts" })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["residents"].as_array().map(Vec::len), Some(2), "{answer}");

    // The socket gets the reason and closes. The waiting call ends.
    assert_eq!(
        next_json(&mut ws).await,
        json!({ "op": "error", "error": { "code": "banned", "message": BANNED } })
    );
    match soon(ws.next()).await {
        Some(Ok(Message::Close(Some(frame)))) => {
            assert_eq!(
                (frame.code, frame.reason.as_str()),
                (CloseCode::Policy, "the owner is banned")
            );
        }
        other => panic!("a close frame, not {other:?}"),
    }
    let (status, answer) = waiting.await.expect("the call ran");
    let banned = json!({ "error": { "code": "banned", "message": BANNED } });
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &banned));

    // The owner token, both resident keys, a new socket and a new
    // sign-in of the same account: all refused.
    let (status, answer) = send(h.get("/v1/residents", &token)).await;
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &banned));
    for key in [&one_key, &two_key] {
        let (status, answer) = send(h.get("/v1/observe", key)).await;
        assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &banned));
        let (status, answer) = h.act(key, "work", json!({ "ticks": 1 })).await;
        assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &banned));
    }
    match try_socket(&h, &one_key).await {
        Err(Error::Http(response)) => assert_eq!(response.status().as_u16(), 403),
        other => panic!("the upgrade must be refused, not {:?}", other.map(|_| "accepted")),
    }
    let (status, answer) = sign_in().await;
    assert_eq!((status, &answer), (StatusCode::FORBIDDEN, &banned));
    // Nothing acted: the residents of the owner do nothing.
    assert_eq!(h.sim.stats().await.expect("stats").inputs_applied, 3, "three spawns");
    // Another owner is not touched.
    let (status, _) = send(h.get("/v1/observe", &other_key)).await;
    assert_eq!(status, StatusCode::OK);

    // The ban is lifted: the old token and the old keys work again.
    let (status, answer) = admin(&h, &format!("/admin/owners/{owner}/unban")).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    assert_eq!(sanctions(&h).await, json!([]));
    let (status, listed) = send(h.get("/v1/residents", &token)).await;
    assert_eq!((status, listed["residents"].as_array().map(Vec::len)), (StatusCode::OK, Some(2)));
    let (status, me) = send(h.get("/v1/me", &one_key)).await;
    assert_eq!((status, me["resident"].clone()), (StatusCode::OK, json!(one)));
    let (status, again) = sign_in().await;
    assert_eq!((status, again["owner"]["id"].clone()), (StatusCode::OK, json!(owner)));

    // An owner that does not exist cannot be banned.
    let (status, answer) = admin(&h, "/admin/owners/999/ban").await;
    assert_eq!((status, code(&answer)), (StatusCode::NOT_FOUND, "not_found"));
}

#[tokio::test]
async fn sanctions_are_in_force_after_a_restart_and_can_be_lifted_then() {
    let h = Harness::start().await;
    let (_, ada_token) = h.owner("Ada").await;
    let (ada, ada_key) = h.resident(&ada_token, "Ada").await;
    let (bob_owner, bob_token) = h.owner("Bob").await;
    let (bob, bob_key) = h.resident(&bob_token, "Bob").await;
    let (cy, cy_key) = h.agent("Cy").await;
    let (dee, dee_key) = h.agent("Dee").await;

    let (status, _) = admin_with(&h, &format!("/admin/residents/{ada}/mute"), json!({})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) =
        admin_with(&h, &format!("/admin/owners/{bob_owner}/ban"), json!({ "reason": "bots" }))
            .await;
    assert_eq!(status, StatusCode::OK);
    let freeze = json!({ "reason": "stuck in a loop" });
    let (status, answer) = admin_with(&h, &format!("/admin/residents/{cy}/freeze"), freeze).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })));
    // A muted resident that leaves the world takes its mute with it.
    let (status, _) = admin(&h, &format!("/admin/residents/{dee}/mute")).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = admin(&h, &format!("/admin/residents/{dee}/despawn")).await;
    assert_eq!(status, StatusCode::OK);
    let before = sanctions(&h).await;
    let kinds = |list: &Value| -> Vec<(String, u64)> {
        list.as_array()
            .expect("a list")
            .iter()
            .map(|entry| {
                let id = entry.get("resident").or(entry.get("owner")).and_then(Value::as_u64);
                (entry["kind"].as_str().expect("a kind").to_string(), id.expect("an id"))
            })
            .collect()
    };
    assert_eq!(
        kinds(&before),
        [("mute".to_string(), ada), ("freeze".to_string(), cy), ("ban".to_string(), bob_owner)]
    );
    drop(dee_key);

    // A new gateway on the same store and the same world.
    let (sim, runtime, store) = h.into_parts();
    let h = Harness::serve(sim, runtime, store, config(""), vec![Arc::new(LocalSignUp)]).await;
    assert_eq!(sanctions(&h).await, before, "the list is the same, with reasons and times");
    let work = json!({ "ticks": 1 });

    let (status, answer) = h.act(&ada_key, "tell", tell(bob, "hello")).await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "muted"));
    let (status, answer) = send(h.get("/v1/observe", &bob_key)).await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "banned"));
    let (status, answer) = send(h.get("/v1/residents", &bob_token)).await;
    assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "banned"));
    let (status, answer) = h.act(&cy_key, "work", work.clone()).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"][0]["status"]["error"]["code"], "blocked", "{answer}");

    // Each one is lifted after the restart.
    for path in [
        format!("/admin/residents/{ada}/unmute"),
        format!("/admin/owners/{bob_owner}/unban"),
        format!("/admin/residents/{cy}/unfreeze"),
    ] {
        let (status, answer) = admin(&h, &path).await;
        assert_eq!((status, answer), (StatusCode::OK, json!({ "ok": true })), "{path}");
    }
    assert_eq!(sanctions(&h).await, json!([]));
    assert_eq!(h.store.sanctions().expect("the store"), []);
    h.done(&ada_key, "tell", tell(bob, "hello")).await;
    let (status, _) = send(h.get("/v1/observe", &bob_key)).await;
    assert_eq!(status, StatusCode::OK);
    let (_, answer) = h.act(&cy_key, "work", work).await;
    assert_eq!(answer["acks"][0]["status"]["state"], "started", "{answer}");
}

#[tokio::test]
async fn the_new_admin_routes_refuse_a_wrong_token() {
    let h = Harness::start().await;
    let (owner, owner_token) = h.owner("Ada").await;
    let (resident, key) = h.resident(&owner_token, "One").await;
    let routes = [
        (Method::POST, format!("/admin/residents/{resident}/mute")),
        (Method::POST, format!("/admin/residents/{resident}/unmute")),
        (Method::POST, format!("/admin/owners/{owner}/ban")),
        (Method::POST, format!("/admin/owners/{owner}/unban")),
        (Method::GET, "/admin/sanctions".to_string()),
        (Method::GET, "/admin/dashboard".to_string()),
        (Method::GET, "/admin/alerts".to_string()),
        (Method::GET, "/admin/flags".to_string()),
        (Method::POST, "/admin/flags/1/dismiss".to_string()),
        (Method::POST, "/admin/rollback".to_string()),
    ];
    let almost = format!("{}x", &ADMIN[..ADMIN.len() - 1]);
    for (method, path) in &routes {
        for token in ["guess", almost.as_str(), owner_token.as_str(), key.as_str()] {
            let request = h.request(method.clone(), path, token).json(&json!({ "frame": 0 }));
            let (status, answer) = send(request).await;
            assert_eq!(
                (status, code(&answer)),
                (StatusCode::UNAUTHORIZED, "invalid_token"),
                "{method} {path} with {token}"
            );
        }
        // No token at all.
        let (status, answer) = send(h.http.request(method.clone(), h.url(path))).await;
        assert_eq!((status, code(&answer)), (StatusCode::UNAUTHORIZED, "unauthorized"), "{path}");
    }
    // Nothing happened: no sanction, and the owner and its resident work.
    assert_eq!(h.store.sanctions().expect("the store"), []);
    let (status, _) = send(h.get("/v1/residents", &owner_token)).await;
    assert_eq!(status, StatusCode::OK);
    h.tell(&key, resident, "still here").await;

    // A server without an admin token closes the new routes too.
    let closed = Harness::with(unsup_gateway::GatewayConfig::default()).await;
    for (method, path) in &routes {
        let (status, answer) = send(closed.request(method.clone(), path, ADMIN)).await;
        assert_eq!((status, code(&answer)), (StatusCode::FORBIDDEN, "admin_disabled"), "{path}");
    }
}
