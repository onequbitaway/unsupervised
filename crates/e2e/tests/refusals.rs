//! The error codes that a client must handle: refused text, a mute, a
//! ban, the limit of residents, wrong tokens and the rate limit. Each
//! one is checked on every transport that can get it.

mod common;

use common::agent::Agent;
use common::link::{Link, myself, state};
use common::mcp::Mcp;
use common::socket::{Codec, Socket};
use common::{World, code, config, id, send};
use futures::StreamExt;
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

/// A text that the rules of a public server refuse: it has a link.
const LINK: &str = "read www.example.com now";

fn say(text: &str) -> Value {
    json!({ "verb": "say", "args": { "text": text } })
}

#[tokio::test(flavor = "multi_thread")]
async fn refused_text_is_text_refused_on_every_transport_and_reaches_nothing() {
    let world = World::start().await;
    let (owner, token) = world.owner("Grace").await;
    let (resident, key) = world.resident(&token, "Ada").await;
    let mut ada = Agent::new(&world, resident, &key);
    assert!(owner > 0);

    // HTTPS: 422. The message names the place and the rule, not the text.
    let (status, refused) = ada.call(Method::POST, "/v1/act", Some(&say(LINK))).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(code(&refused), "text_refused");
    let message = refused["error"]["message"].as_str().expect("a message");
    assert!(message.contains("args.text") && message.contains("link"), "{message}");
    assert!(!message.contains("example"), "the message does not repeat the text: {message}");

    // One refused text stops a whole plan: no request reached the world.
    let plan = json!({ "plan": [{ "verb": "wait", "args": { "ticks": 5 } }, say(LINK)] });
    let (status, refused) = ada.call(Method::POST, "/v1/act", Some(&plan)).await;
    assert_eq!((status, code(&refused)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
    assert_eq!(
        myself(&mut ada).await["activity"],
        Value::Null,
        "the wait of the plan did not start"
    );

    // The text of a routine passes the same filters.
    let routine = json!({ "name": "greet", "when": { "type": "always" }, "then": say(LINK) });
    let body = json!({ "routines": [routine] });
    let (status, refused) = ada.call(Method::PUT, "/v1/routines", Some(&body)).await;
    assert_eq!((status, code(&refused)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));

    // A name that viewers see passes the filters too.
    let body = json!({ "name": "Zzzzzzzzzzzzzzzzzzzzzzzz" });
    let (status, refused) = world.post("/v1/residents", &token, &body).await;
    assert_eq!((status, code(&refused)), (StatusCode::UNPROCESSABLE_ENTITY, "text_refused"));
    let (_, listed) = world.get("/v1/residents", &token).await;
    assert_eq!(listed["residents"].as_array().map(Vec::len), Some(1), "no resident was made");

    // The WebSocket: an `error` frame, and the connection stays open.
    for codec in [Codec::Json, Codec::MsgPack] {
        let mut socket = Socket::open(&world, &key, codec, "").await;
        let error = socket.refused(&json!({ "op": "act", "request": say(LINK) })).await;
        assert_eq!(error["code"], "text_refused", "{error}");
        let error =
            socket.refused(&json!({ "op": "plan", "requests": [say("fine"), say(LINK)] })).await;
        assert_eq!(error["code"], "text_refused", "one error for the whole plan: {error}");
        socket.ping().await;
        socket.close().await;
    }

    // MCP: a tool result with `isError` that starts with the code.
    let (mut mcp, _) = Mcp::initialize(&world, &key).await;
    let refused = mcp.call("act", say(LINK)).await;
    assert!(refused.is_error, "{}", refused.text);
    assert!(refused.text.starts_with("text_refused: "), "{}", refused.text);

    // A text that the rules accept passes on each transport.
    let (status, spoken) = ada.call(Method::POST, "/v1/act", Some(&say("good morning"))).await;
    assert_eq!(status, StatusCode::OK, "{spoken}");
    assert_eq!(spoken["acks"][0]["status"]["state"], "done", "{spoken}");
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_muted_resident_cannot_speak_and_can_do_everything_else() {
    let world = World::start().await;
    let mut ada = Agent::join(&world, "Ada").await;
    let mut socket = Socket::open(&world, &ada.key, Codec::Json, "").await;
    let (mut mcp, _) = Mcp::initialize(&world, &ada.key).await;

    let path = format!("/admin/residents/{}/mute", ada.id);
    let (status, muted) = world.admin(&path, &json!({ "reason": "test" })).await;
    assert_eq!((status, muted), (StatusCode::OK, json!({ "ok": true })));

    let (status, refused) = ada.call(Method::POST, "/v1/act", Some(&say("hello"))).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "muted"), "{refused}");
    let error = socket.refused(&json!({ "op": "act", "request": say("hello") })).await;
    assert_eq!(error["code"], "muted", "{error}");
    let refused = mcp.call("act", say("hello")).await;
    assert!(refused.is_error && refused.text.starts_with("muted: "), "{}", refused.text);

    // Every other action works.
    let ack = ada.act("wait", json!({ "ticks": 1 })).await;
    assert_eq!(state(&ack), "started", "{ack}");

    let path = format!("/admin/residents/{}/unmute", ada.id);
    assert_eq!(world.admin(&path, &Value::Null).await, (StatusCode::OK, json!({ "ok": true })));
    let ack = ada.act("say", json!({ "text": "hello" })).await;
    assert_eq!(state(&ack), "done", "{ack}");
    socket.close().await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_stops_every_token_of_the_owner_and_ends_its_open_calls() {
    let mut config = config();
    // The test asks the statistics again and again, until a call is open.
    common::no_rate_limits(&mut config);
    let world = World::with(config).await;
    let (owner, token) = world.owner("Grace").await;
    let (resident, key) = world.resident(&token, "Ada").await;
    let mut ada = Agent::new(&world, resident, &key);
    ada.observe("brief", &["self"]).await;
    let mut socket = Socket::open(&world, &key, Codec::Json, "").await;
    socket.ping().await;

    // A long poll is open when the ban comes.
    let waiting = tokio::spawn({
        let request = world.request(Method::GET, "/v1/wait?timeout=60", &key);
        async move { send(request).await }
    });
    // The server counts the open wait in its statistics.
    loop {
        let request = world.request(Method::GET, "/admin/stats", common::ADMIN);
        let (status, stats) = send(request).await;
        assert_eq!(status, StatusCode::OK, "{stats}");
        if stats["gateway"]["open_waits"] == 1 {
            break;
        }
        tokio::task::yield_now().await;
    }

    let path = format!("/admin/owners/{owner}/ban");
    let (status, banned) = world.admin(&path, &json!({ "reason": "test" })).await;
    assert_eq!((status, banned), (StatusCode::OK, json!({ "ok": true, "residents": [resident] })));

    // The open long poll ends with the code `banned`.
    let (status, ended) = common::soon(waiting).await.expect("the long poll ends");
    assert_eq!((status, code(&ended)), (StatusCode::FORBIDDEN, "banned"), "{ended}");

    // The open socket gets an `error` frame and the close code 1008.
    let error = socket.answer("error").await["error"].clone();
    assert_eq!(error["code"], "banned", "{error}");
    let mut stream = socket.into_stream();
    let closed = common::soon(async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Close(frame))) => return frame,
                Some(Ok(_)) => {}
                other => panic!("a close frame, not {other:?}"),
            }
        }
    })
    .await;
    assert_eq!(u16::from(closed.expect("a close frame with a code").code), 1008);

    // No token of the owner works: the key, the owner token, a new socket, MCP.
    let (status, refused) = ada.call(Method::GET, "/v1/observe", None).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "banned"));
    let (status, refused) = world.get("/v1/residents", &token).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "banned"));
    let mut request =
        format!("ws://{}/v1/ws", world.addr).into_client_request().expect("a request");
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {key}").parse().expect("a value"));
    let refused =
        tokio_tungstenite::connect_async(request).await.expect_err("the upgrade is refused");
    let tokio_tungstenite::tungstenite::Error::Http(response) = refused else {
        panic!("an HTTP answer, not {refused:?}");
    };
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let message = json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" });
    let (status, refused) = send(world.request(Method::POST, "/mcp", &key).json(&message)).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "banned"));

    // The resident stays in the world. After `unban` its key works again.
    let path = format!("/admin/owners/{owner}/unban");
    assert_eq!(world.admin(&path, &Value::Null).await, (StatusCode::OK, json!({ "ok": true })));
    assert_eq!(id(&myself(&mut ada).await["id"], "id"), resident);
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tokens_limits_and_wrong_requests_get_the_documented_codes() {
    let world = World::start().await;
    let (_, token) = world.owner("Grace").await;

    // The limit of residents of one owner.
    let (_, listed) = world.get("/v1/residents", &token).await;
    let limit = id(&listed["limit"], "limit");
    assert!(limit >= 1, "{listed}");
    let mut residents = Vec::new();
    for number in 0..limit {
        residents.push(world.resident(&token, &format!("Resident {number}")).await);
    }
    let (status, refused) =
        world.post("/v1/residents", &token, &json!({ "name": "One more" })).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "resident_limit"), "{refused}");
    let (resident, key) = residents[0].clone();

    // Tokens.
    let bare = world.open(Method::GET, "/v1/observe").send().await.expect("an answer");
    assert_eq!(bare.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        bare.headers().get("www-authenticate").and_then(|v| v.to_str().ok()),
        Some("Bearer")
    );
    assert_eq!(code(&bare.json::<Value>().await.expect("JSON")), "unauthorized");
    let (status, refused) = world.get("/v1/observe", "unsup_res_0000").await;
    assert_eq!((status, code(&refused)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (status, refused) = world.get("/v1/observe", &token).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "wrong_token_kind"));
    let (status, refused) = world.get("/v1/residents", &key).await;
    assert_eq!((status, code(&refused)), (StatusCode::FORBIDDEN, "wrong_token_kind"));

    // A new key: the old one stops at once.
    let path = format!("/v1/residents/{resident}/keys");
    let (status, rotated) = send(world.request(Method::POST, &path, &token)).await;
    assert_eq!(status, StatusCode::OK, "{rotated}");
    assert_eq!(rotated["resident"], resident);
    let new_key = rotated["key"].as_str().expect("the new key").to_string();
    assert!(new_key.starts_with("unsup_res_") && new_key != key);
    assert!(rotated["key_hint"].is_string(), "{rotated}");
    let (status, refused) = world.get("/v1/me", &key).await;
    assert_eq!((status, code(&refused)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    assert_eq!(world.get("/v1/me", &new_key).await.0, StatusCode::OK);
    let (other, _) = world.newcomer("Stranger").await;
    let path = format!("/v1/residents/{other}/keys");
    let (status, refused) = send(world.request(Method::POST, &path, &token)).await;
    assert_eq!((status, code(&refused)), (StatusCode::NOT_FOUND, "not_found"), "not your resident");

    // Requests with a wrong form.
    let (status, refused) = world.get("/v1/observe?detail=huge", &new_key).await;
    assert_eq!((status, code(&refused)), (StatusCode::BAD_REQUEST, "invalid_request"));
    let (status, refused) = world.get("/v1/observe?verbose=1", &new_key).await;
    assert_eq!(
        (status, code(&refused)),
        (StatusCode::BAD_REQUEST, "invalid_request"),
        "an unknown query name"
    );
    let body = json!({ "verb": "wait", "args": { "ticks": 1 }, "priority": 9 });
    let (status, refused) = world.post("/v1/act", &new_key, &body).await;
    assert_eq!(
        (status, code(&refused)),
        (StatusCode::BAD_REQUEST, "invalid_request"),
        "an unknown field"
    );
    let plan: Vec<Value> =
        (0..17).map(|_| json!({ "verb": "wait", "args": { "ticks": 1 } })).collect();
    let (status, refused) = world.post("/v1/act", &new_key, &json!({ "plan": plan })).await;
    assert_eq!(
        (status, code(&refused)),
        (StatusCode::BAD_REQUEST, "invalid_request"),
        "a plan has at most 16 requests"
    );
    let text = world.request(Method::POST, "/v1/act", &new_key).body("{\"verb\":\"wait\"}");
    let (status, refused) = send(text).await;
    assert_eq!(
        (status, code(&refused)),
        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
    );
    let (status, refused) = world.get("/v1/nothing", &new_key).await;
    assert_eq!((status, code(&refused)), (StatusCode::NOT_FOUND, "not_found"));
    let (status, refused) = send(world.request(Method::DELETE, "/v1/act", &new_key)).await;
    assert_eq!((status, code(&refused)), (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"));
    let (status, refused) = world.get("/v1/ws", &new_key).await;
    assert_eq!((status, code(&refused)), (StatusCode::UPGRADE_REQUIRED, "upgrade_required"));

    // An action that the simulation rejects is not an HTTP error: the
    // answer is 200, and the acknowledgement has the code.
    let (status, answer) =
        world.post("/v1/act", &new_key, &json!({ "verb": "fly", "args": {} })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"][0]["status"]["state"], "rejected", "{answer}");
    assert_eq!(answer["acks"][0]["status"]["error"]["code"], "unknown_verb", "{answer}");
    // A newcomer has no job, so the game refuses `work` away from a
    // site with starter shifts. The message says what to do.
    let (status, answer) =
        world.post("/v1/act", &new_key, &json!({ "verb": "work", "args": {} })).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let refusal = &answer["acks"][0]["status"];
    assert_eq!(refusal["state"], "rejected", "{answer}");
    assert!(refusal["error"]["message"].as_str().is_some_and(|text| !text.is_empty()), "{answer}");

    // A resident that an administrator removed: its key ends with it.
    let path = format!("/admin/residents/{resident}/despawn");
    assert_eq!(world.admin(&path, &Value::Null).await, (StatusCode::OK, json!({ "ok": true })));
    let (status, refused) = world.get("/v1/observe", &new_key).await;
    assert_eq!((status, code(&refused)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    // The place of that resident is free again.
    let (status, created) =
        world.post("/v1/residents", &token, &json!({ "name": "One more" })).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn too_many_requests_get_rate_limited_with_retry_after() {
    let mut config = config();
    config.gateway.limits.key_requests_per_minute = 60;
    config.gateway.limits.key_burst = 3;
    let world = World::with(config).await;
    let (_, key) = world.newcomer("Ada").await;
    let mut refused = None;
    for _ in 0..4 {
        let response = world.request(Method::GET, "/v1/me", &key).send().await.expect("an answer");
        if response.status() != StatusCode::OK {
            refused = Some(response);
            break;
        }
    }
    let refused = refused.expect("the fourth request of a burst of three is refused");
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = refused
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .expect("`Retry-After` has the seconds to wait");
    assert_eq!(wait, 1, "one request each second");
    assert_eq!(code(&refused.json::<Value>().await.expect("JSON")), "rate_limited");
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resident_that_starves_is_gone_and_its_key_ends() {
    let world = World::start().await;
    let (resident, key) = world.newcomer("Ada").await;
    let mut socket = Socket::open(&world, &key, Codec::Json, "").await;
    socket.ping().await;

    // The agent does nothing. The resident does not eat, so it dies.
    // Viewers know a resident from the first frame after it came.
    world.step(1).await;
    let profile = format!("/v1/residents/{resident}/profile");
    let mut days = 0;
    while world.public(&profile).await["alive"] == true {
        assert!(days < 30, "a resident without food dies in 30 game days");
        world.step(1_440).await;
        days += 1;
    }
    let dead = world.public(&profile).await;
    assert_eq!(dead["cause"], "starvation", "{dead}");
    assert_eq!(dead["life"]["death"]["kind"], "life.resident_died", "{dead}");

    // The call that finds the resident gone gets 410. After it, the key
    // is unknown.
    let (status, gone) = world.get("/v1/observe", &key).await;
    assert_eq!((status, code(&gone)), (StatusCode::GONE, "resident_gone"), "{gone}");
    let (status, ended) = world.get("/v1/observe", &key).await;
    assert_eq!((status, code(&ended)), (StatusCode::UNAUTHORIZED, "invalid_token"), "{ended}");

    // The socket was open all the time: the needs sent warnings before
    // the end. The server does not close it by itself.
    let observe = json!({ "op": "observe", "opts": { "sections": ["self"] } });
    socket.send(&observe).await;
    let mut pushed = Vec::new();
    let error = loop {
        let frame = common::soon(socket.frame()).await;
        match frame["op"].as_str() {
            Some("notice") => {
                pushed.push(frame["notice"]["kind"].as_str().expect("a kind").to_string())
            }
            Some("error") => break frame["error"].clone(),
            _ => panic!("a notice or an error, not {frame}"),
        }
    };
    assert!(pushed.iter().any(|kind| kind == "life.need_low"), "{pushed:?}");
    assert!(pushed.iter().any(|kind| kind == "life.need_empty"), "{pushed:?}");
    // The next request on the open socket gets the code `resident_gone`.
    assert_eq!(error["code"], "resident_gone", "{error}");
    world.stop().await;
}
