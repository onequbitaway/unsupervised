//! The WebSocket for agents: both codecs, pushes, limits and timeouts.

mod common;

use common::{ADMIN, Deadline, Harness, code, send, soon};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Opens the agent WebSocket. `query` is empty or starts with `?`.
async fn try_connect(h: &Harness, token: Option<&str>, query: &str) -> Result<Socket, Error> {
    let mut request =
        format!("ws://{}/v1/ws{query}", h.addr).into_client_request().expect("a request");
    if let Some(token) = token {
        let value = format!("Bearer {token}").parse().expect("a header value");
        request.headers_mut().insert("Authorization", value);
    }
    tokio_tungstenite::connect_async(request).await.map(|(socket, _)| socket)
}

async fn connect(h: &Harness, key: &str, query: &str) -> Socket {
    try_connect(h, Some(key), query).await.expect("the socket opens")
}

/// Returns the HTTP status with which the server refused the upgrade.
async fn refusal(h: &Harness, token: Option<&str>) -> (StatusCode, Value) {
    match try_connect(h, token, "").await {
        Err(Error::Http(response)) => {
            let body = response.body().as_deref().unwrap_or_default();
            let status = StatusCode::from_u16(response.status().as_u16()).expect("a status");
            (status, serde_json::from_slice(body).expect("a JSON error body"))
        }
        other => panic!("the upgrade must be refused, not {:?}", other.map(|_| "accepted")),
    }
}

/// Reads the next data frame. Pings of the server are skipped.
async fn next_data(socket: &mut Socket) -> Message {
    loop {
        let message = soon(socket.next()).await.expect("the socket is open").expect("a frame");
        if !matches!(message, Message::Ping(_) | Message::Pong(_)) {
            return message;
        }
    }
}

/// Reads the next frame, which must be JSON text.
async fn next_json(socket: &mut Socket) -> Value {
    match next_data(socket).await {
        Message::Text(text) => serde_json::from_str(&text).expect("JSON"),
        other => panic!("a text frame, not {other:?}"),
    }
}

/// Reads the next frame, which must be MessagePack.
async fn next_msgpack(socket: &mut Socket) -> Value {
    match next_data(socket).await {
        Message::Binary(bytes) => rmp_serde::from_slice(&bytes).expect("MessagePack"),
        other => panic!("a binary frame, not {other:?}"),
    }
}

async fn send_json(socket: &mut Socket, frame: Value) {
    socket.send(Message::Text(frame.to_string().into())).await.expect("the frame is sent");
}

async fn send_msgpack(socket: &mut Socket, frame: Value) {
    let bytes = rmp_serde::to_vec_named(&frame).expect("MessagePack");
    socket.send(Message::Binary(bytes.into())).await.expect("the frame is sent");
}

#[tokio::test]
async fn json_frames_and_pushed_news() {
    let h = Harness::start().await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    let mut socket = connect(&h, &ada_key, "").await;

    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));

    let request = json!({ "id": "w1", "verb": "work", "args": { "ticks": 1 } });
    send_json(&mut socket, json!({ "op": "act", "request": request })).await;
    let ack = next_json(&mut socket).await;
    assert_eq!(ack["op"], "ack");
    assert_eq!(ack["ack"]["request_id"], "w1");
    assert_eq!(ack["ack"]["status"]["state"], "started");

    send_json(&mut socket, json!({ "op": "observe", "opts": { "sections": ["self"] } })).await;
    let observation = next_json(&mut socket).await;
    assert_eq!(observation["op"], "observation");
    assert_eq!(observation["observation"]["resident"], ada);
    assert_eq!(observation["observation"]["sections"]["self"]["activity"]["verb"], "work");

    // Bob talks and the work ends in the same frame. The server pushes
    // both, without a request of the client: the report, then the notice.
    h.tell(&bob_key, ada, "hello").await;
    h.step(1).await;
    let report = next_json(&mut socket).await;
    assert_eq!(report["op"], "report");
    assert_eq!(report["report"]["action_id"], ack["ack"]["action_id"]);
    assert_eq!(
        (report["report"]["request_id"].clone(), report["report"]["ok"].clone()),
        (json!("w1"), json!(true))
    );
    let notice = next_json(&mut socket).await;
    assert_eq!(
        notice,
        json!({ "op": "notice", "notice": {
            "seq": 1,
            "tick": notice["notice"]["tick"],
            "kind": "town.told",
            "from": bob,
            "data": { "untrusted": true, "from": bob, "text": "hello" },
        } })
    );

    // A plan gives one acknowledgement for each request.
    let work = json!({ "verb": "work", "args": { "ticks": 1 } });
    send_json(
        &mut socket,
        json!({ "op": "plan", "requests": [work, work, { "verb": "fly" }, work] }),
    )
    .await;
    let states = [
        next_json(&mut socket).await["ack"]["status"]["state"].clone(),
        next_json(&mut socket).await["ack"]["status"]["state"].clone(),
        next_json(&mut socket).await["ack"]["status"]["state"].clone(),
    ];
    assert_eq!(states, [json!("started"), json!("queued"), json!("rejected")]);

    // Frames that the server cannot read. The connection stays open, and
    // the next answer is the first frame after the errors.
    for frame in [
        json!({ "op": "dance" }),
        json!({ "op": "ping", "extra": 1 }),
        json!({ "op": "act" }),
        json!({ "op": "act", "request": { "verb": "Bad Verb" } }),
        json!({ "op": "act", "request": { "verb": "work", "arg": { "ticks": 1 } } }),
        json!({ "op": "act", "request": "work" }),
        json!({ "op": "plan", "requests": [] }),
        json!({ "op": "plan", "requests": [{ "verb": "work", "when": "now" }] }),
        json!({ "op": "plan", "requests": vec![json!({ "verb": "work" }); 17] }),
        json!({ "op": "observe", "opts": { "sections": ["NOT A NAME"] } }),
        json!({ "op": "observe", "opts": { "detial": "full" } }),
        json!({ "op": "subscribe", "kinds": ["bad kind"] }),
    ] {
        send_json(&mut socket, frame.clone()).await;
        let answer = next_json(&mut socket).await;
        assert_eq!(answer["op"], "error", "{frame}");
        assert_eq!(answer["error"]["code"], "invalid_request", "{frame}");
    }
    socket.send(Message::Text("not json".into())).await.expect("sent");
    assert_eq!(next_json(&mut socket).await["error"]["code"], "invalid_request");
    // The error says what to change, as for `POST /v1/act`.
    let typo = json!({ "verb": "work", "arg": { "ticks": 1 } });
    send_json(&mut socket, json!({ "op": "plan", "requests": [{ "verb": "work" }, typo] })).await;
    assert_eq!(
        next_json(&mut socket).await,
        json!({ "op": "error", "error": {
            "code": "invalid_request",
            "message": "requests[1] has the unknown field `arg`. An action has `id`, `verb` \
                        and `args`.",
        } })
    );
    // None of the refused frames reached the world: the resident works
    // on the plan from before and has nothing more in its queue.
    send_json(&mut socket, json!({ "op": "observe", "opts": { "sections": ["self"] } })).await;
    let observation = next_json(&mut socket).await;
    assert_eq!(observation["observation"]["sections"]["self"]["queue"], 1);
    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));
}

#[tokio::test]
async fn messagepack_frames_and_pushed_news() {
    let h = Harness::start().await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (bob, bob_key) = h.agent("Bob").await;
    let mut socket = connect(&h, &ada_key, "").await;

    // A binary frame is MessagePack, and the answer has the same format.
    send_msgpack(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_msgpack(&mut socket).await, json!({ "op": "pong" }));

    let request = json!({ "verb": "work", "args": { "ticks": 1 } });
    send_msgpack(&mut socket, json!({ "op": "act", "request": request })).await;
    let ack = next_msgpack(&mut socket).await;
    assert_eq!(
        (ack["op"].clone(), ack["ack"]["status"]["state"].clone()),
        (json!("ack"), json!("started"))
    );
    assert_eq!(ack["ack"]["status"]["ticks"], 1);

    send_msgpack(&mut socket, json!({ "op": "observe", "opts": { "detail": "full" } })).await;
    let observation = next_msgpack(&mut socket).await;
    assert_eq!(observation["observation"]["resident"], ada);
    assert_eq!(observation["observation"]["sections"]["self"]["routines"], json!([]));

    // Pushes follow the format of the last frame of the client.
    h.tell(&bob_key, ada, "hello").await;
    h.step(1).await;
    let report = next_msgpack(&mut socket).await;
    assert_eq!(
        (report["op"].clone(), report["report"]["result"].clone()),
        (json!("report"), json!({ "worked": 1 }))
    );
    let notice = next_msgpack(&mut socket).await;
    assert_eq!(notice["op"], "notice");
    assert_eq!(
        notice["notice"]["data"],
        json!({ "untrusted": true, "from": bob, "text": "hello" })
    );

    // A broken binary frame gets an error in MessagePack.
    socket.send(Message::Binary(vec![0xc1].into())).await.expect("sent");
    assert_eq!(next_msgpack(&mut socket).await["error"]["code"], "invalid_request");

    // One text frame switches the pushes back to JSON.
    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));
    h.tell(&bob_key, ada, "again").await;
    h.step(1).await;
    assert_eq!(next_json(&mut socket).await["notice"]["data"]["text"], "again");

    // `?codec=msgpack` sets the format of the pushes for a client that
    // only listens.
    let mut listener = connect(&h, &ada_key, "?codec=msgpack").await;
    h.tell(&bob_key, ada, "to the listener").await;
    h.step(1).await;
    assert_eq!(next_msgpack(&mut listener).await["notice"]["data"]["text"], "to the listener");
    // The first socket gets the same notice, in its own format.
    assert_eq!(next_json(&mut socket).await["notice"]["data"]["text"], "to the listener");
}

#[tokio::test]
async fn subscribe_chooses_the_pushes_and_since_replays() {
    let h = Harness::start().await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let tell = |text: &'static str, kind: &'static str| {
        let action = json!({ "verb": "tell", "args": { "to": ada, "text": text, "kind": kind } });
        send(h.post("/v1/act", &bob_key, &action))
    };
    let mut socket = connect(&h, &ada_key, "").await;

    // Only alarms. A ping after the frame proves that the server read
    // the subscription before the news came.
    send_json(&mut socket, json!({ "op": "subscribe", "kinds": ["town.alarm"] })).await;
    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));
    tell("small talk", "town.told").await;
    tell("fire", "town.alarm").await;
    h.step(1).await;
    // The talk came first. It was not pushed, so the alarm is next.
    let pushed = next_json(&mut socket).await;
    assert_eq!(
        (pushed["notice"]["seq"].clone(), pushed["notice"]["kind"].clone()),
        (json!(2), json!("town.alarm"))
    );

    // Only reports.
    send_json(&mut socket, json!({ "op": "subscribe", "kinds": ["report"] })).await;
    send_json(
        &mut socket,
        json!({ "op": "act", "request": { "verb": "work", "args": { "ticks": 1 } } }),
    )
    .await;
    assert_eq!(next_json(&mut socket).await["op"], "ack");
    tell("more talk", "town.told").await;
    h.step(1).await;
    assert_eq!(next_json(&mut socket).await["op"], "report");

    // Everything again.
    send_json(&mut socket, json!({ "op": "subscribe" })).await;
    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));
    tell("last", "town.told").await;
    h.step(1).await;
    assert_eq!(next_json(&mut socket).await["notice"]["data"]["text"], "last");

    // A new connection sees only later news, unless it names a cursor.
    let mut late = connect(&h, &ada_key, "").await;
    let mut replay = connect(&h, &ada_key, "?since=3").await;
    let kinds =
        [next_json(&mut replay).await, next_json(&mut replay).await, next_json(&mut replay).await];
    assert_eq!(
        kinds.iter().map(|frame| frame["op"].clone()).collect::<Vec<_>>(),
        [json!("report"), json!("notice"), json!("notice")]
    );
    // The report (4) comes first, then the talk (3) and the last
    // notice (5).
    assert_eq!(
        (kinds[1]["notice"]["seq"].clone(), kinds[2]["notice"]["seq"].clone()),
        (json!(3), json!(5))
    );
    // A client that lost its connection after the talk (3) connects
    // again with the number after it. It gets the notice that it missed.
    // The report has the number 4, so it comes a second time; nothing
    // is lost.
    let mut again = connect(&h, &ada_key, "?since=4").await;
    let first = next_json(&mut again).await;
    assert_eq!(first["op"], "report");
    assert_eq!(first["report"]["action_id"], kinds[0]["report"]["action_id"]);
    assert_eq!(next_json(&mut again).await["notice"]["seq"], 5);
    drop(again);

    tell("after", "town.told").await;
    h.step(1).await;
    assert_eq!(next_json(&mut late).await["notice"]["data"]["text"], "after");
    assert_eq!(next_json(&mut replay).await["notice"]["data"]["text"], "after");
}

#[tokio::test]
async fn the_upgrade_needs_a_resident_key_and_a_free_place() {
    let h = Harness::with_toml("[ws]\nconnections_per_resident = 1").await;
    let (_, owner_token) = h.owner("Ada").await;
    let (_, key) = h.resident(&owner_token, "One").await;

    let (status, body) = refusal(&h, None).await;
    assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "unauthorized"));
    let (status, body) = refusal(&h, Some("unsup_res_wrong")).await;
    assert_eq!((status, code(&body)), (StatusCode::UNAUTHORIZED, "invalid_token"));
    let (status, body) = refusal(&h, Some(&owner_token)).await;
    assert_eq!((status, code(&body)), (StatusCode::FORBIDDEN, "wrong_token_kind"));

    // A plain GET is not a WebSocket.
    let (status, body) = send(h.get("/v1/ws", &key)).await;
    assert_eq!((status, code(&body)), (StatusCode::UPGRADE_REQUIRED, "upgrade_required"));
    let (status, body) = send(h.get("/v1/ws?codec=xml", &key)).await;
    assert_eq!((status, code(&body)), (StatusCode::BAD_REQUEST, "invalid_request"));

    // One connection is the limit here.
    let first = connect(&h, &key, "").await;
    let (status, body) = refusal(&h, Some(&key)).await;
    assert_eq!((status, code(&body)), (StatusCode::TOO_MANY_REQUESTS, "too_many_connections"));
    let (_, stats) = send(h.get("/admin/stats", ADMIN)).await;
    assert_eq!(stats["gateway"]["ws_connections"], 1);

    // When it closes, the place is free again.
    drop(first);
    let deadline = Deadline::start();
    let second = loop {
        match try_connect(&h, Some(&key), "").await {
            Ok(socket) => break socket,
            Err(Error::Http(_)) => deadline.again().await,
            Err(other) => panic!("the socket opens: {other}"),
        }
    };
    drop(second);
}

#[tokio::test]
async fn frames_count_for_the_rate_limit_of_the_key() {
    let h = Harness::with_toml("[limits]\nkey_requests_per_minute = 1\nkey_burst = 3").await;
    let (_, key) = h.agent("Ada").await;
    // The upgrade is one request. Two frames more fit.
    let mut socket = connect(&h, &key, "").await;
    for _ in 0..2 {
        send_json(&mut socket, json!({ "op": "ping" })).await;
        assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));
    }
    send_json(&mut socket, json!({ "op": "ping" })).await;
    let answer = next_json(&mut socket).await;
    assert_eq!(
        answer,
        json!({ "op": "error", "error": {
            "code": "rate_limited",
            "message": "Too many requests. Try again in 60 seconds.",
        } })
    );
}

#[tokio::test]
async fn a_client_that_reads_too_slowly_is_dropped() {
    let h = Harness::with_toml("[ws]\nqueue = 2").await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let mut socket = connect(&h, &ada_key, "").await;

    // Four notices in one frame of the world, for a queue of two.
    for n in 0..4 {
        h.tell(&bob_key, ada, &format!("message {n}")).await;
    }
    h.step(1).await;

    // The server cuts the connection: no close frame, and not all four.
    let mut received = 0;
    loop {
        match soon(socket.next()).await {
            Some(Ok(Message::Text(_))) => {
                received += 1;
                assert!(received <= 2, "{received} notices came through a queue of two");
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            Some(Ok(other)) => panic!("the server sends no {other:?} to a slow client"),
            Some(Err(_)) | None => break,
        }
    }
    let deadline = Deadline::start();
    let stats = loop {
        let (_, stats) = send(h.get("/admin/stats", ADMIN)).await;
        if stats["gateway"]["ws_connections"] == 0 {
            break stats;
        }
        deadline.again().await;
    };
    assert_eq!(stats["gateway"]["ws_dropped"], 1);

    // The inbox still holds the notices: nothing is lost for a client
    // that comes back with its cursor.
    let (_, inbox) = send(h.get("/v1/observe?since=1", &ada_key)).await;
    let texts: Vec<&Value> =
        inbox["notices"].as_array().expect("notices").iter().map(|n| &n["data"]["text"]).collect();
    assert_eq!(texts, ["message 0", "message 1", "message 2", "message 3"]);
}

/// Opens the agent WebSocket by hand, on a plain TCP stream. This client
/// never answers a ping, as a client does whose machine went away.
/// Returns the stream and the frame bytes that came with the handshake.
async fn silent_client(h: &Harness, key: &str) -> (TcpStream, Vec<u8>) {
    let mut stream = TcpStream::connect(h.addr).await.expect("the port is open");
    let request = format!(
        "GET /v1/ws HTTP/1.1\r\nHost: {}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\
         Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Authorization: Bearer {key}\r\n\r\n",
        h.addr
    );
    stream.write_all(request.as_bytes()).await.expect("the handshake is sent");
    let mut seen = Vec::new();
    let end = loop {
        let mut chunk = [0u8; 512];
        let read = stream.read(&mut chunk).await.expect("the server answers");
        assert!(read > 0, "the server closed during the handshake");
        seen.extend_from_slice(&chunk[..read]);
        if let Some(at) = seen.windows(4).position(|window| window == b"\r\n\r\n") {
            break at + 4;
        }
    };
    let head = String::from_utf8_lossy(&seen[..end]).to_string();
    assert!(head.starts_with("HTTP/1.1 101"), "{head}");
    (stream, seen[end..].to_vec())
}

#[tokio::test]
async fn the_server_pings_and_closes_an_idle_connection() {
    let h = Harness::with_toml("[ws]\nping_interval_ms = 20\nidle_timeout_ms = 60").await;
    let (_, key) = h.agent("Ada").await;
    let (mut stream, mut frames) = silent_client(&h, &key).await;

    // The client sends nothing. The server pings, then gives up: a close
    // frame, then the end of the stream.
    soon(stream.read_to_end(&mut frames)).await.expect("the server closes the stream");
    const PING: [u8; 2] = [0x89, 0x00];
    // Close, 14 bytes: the code 1008 and the reason.
    let mut close = vec![0x88, 14, 0x03, 0xf0];
    close.extend_from_slice(b"idle timeout");
    assert!(frames.ends_with(&close), "the last frame is the close: {frames:?}");
    let pings = &frames[..frames.len() - close.len()];
    assert!(!pings.is_empty(), "the server pinged before it closed");
    assert!(pings.chunks(2).all(|frame| frame == PING), "only pings before the close: {pings:?}");
}

#[tokio::test]
async fn a_client_that_answers_pings_stays_connected() {
    let h = Harness::with_toml("[ws]\nping_interval_ms = 10\nidle_timeout_ms = 40").await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let mut socket = connect(&h, &ada_key, "").await;

    // Read ten pings: 100 ms, more than twice the idle timeout. Reading
    // answers each ping.
    let mut pings = 0;
    while pings < 10 {
        match soon(socket.next()).await.expect("the socket is open").expect("a frame") {
            Message::Ping(_) => pings += 1,
            other => panic!("only pings, not {other:?}"),
        }
    }
    h.tell(&bob_key, ada, "still there?").await;
    h.step(1).await;
    assert_eq!(next_json(&mut socket).await["notice"]["data"]["text"], "still there?");
}

#[tokio::test]
async fn shutdown_closes_the_connections() {
    let h = Harness::start().await;
    let (_, key) = h.agent("Ada").await;
    let mut socket = connect(&h, &key, "").await;
    send_json(&mut socket, json!({ "op": "ping" })).await;
    assert_eq!(next_json(&mut socket).await, json!({ "op": "pong" }));

    h.gateway.shutdown();
    match next_data(&mut socket).await {
        Message::Close(Some(frame)) => {
            assert_eq!((frame.code, frame.reason.as_str()), (CloseCode::Away, "the server stops"));
        }
        other => panic!("a close frame, not {other:?}"),
    }
    // A call that waits ends too.
    let (status, body) = send(h.get("/v1/wait?timeout=60", &key)).await;
    assert_eq!((status, code(&body)), (StatusCode::SERVICE_UNAVAILABLE, "shutdown"));
}

#[tokio::test]
async fn a_resident_that_leaves_the_world_ends_its_connection() {
    let h = Harness::start().await;
    let (resident, key) = h.agent("Ada").await;
    let mut socket = connect(&h, &key, "").await;
    let despawn = format!("/admin/residents/{resident}/despawn");
    let (status, _) = send(h.request(reqwest::Method::POST, &despawn, ADMIN)).await;
    assert_eq!(status, StatusCode::OK);

    send_json(&mut socket, json!({ "op": "observe" })).await;
    let answer = next_json(&mut socket).await;
    assert_eq!(
        (answer["op"].clone(), answer["error"]["code"].clone()),
        (json!("error"), json!("resident_gone"))
    );
    // The server ends the connection after the error.
    loop {
        match soon(socket.next()).await {
            Some(Ok(Message::Ping(_) | Message::Close(_))) => {}
            Some(Ok(other)) => panic!("no more frames, not {other:?}"),
            Some(Err(_)) | None => break,
        }
    }
}
