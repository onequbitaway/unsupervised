//! A whole server: the development mode, the SQLite store across a
//! restart, the paths that no route takes, and the stop.

mod common;

use std::time::{Duration, Instant};

use common::{Client, TempDir, config, send};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use unsup_gateway::AdminToken;
use unsup_server::{ServerConfig, StoreConfig};

#[tokio::test(flavor = "multi_thread")]
async fn the_development_mode_makes_a_ready_owner_and_an_admin_token() {
    let dev = ServerConfig { store: StoreConfig::Memory, dev: true, ..config() };
    let server = unsup_server::start(dev.clone()).await.expect("the server starts");
    let client = Client::of(&server);
    let access = server.dev.clone().expect("the development mode gives access");

    // The owner token works at once: no sign-up is needed.
    assert!(access.owner_token.starts_with("unsup_own_"), "{}", access.owner_token);
    let (resident, key) = client.resident(&access.owner_token, "Ada").await;
    let (status, me) = send(client.get("/v1/me").bearer_auth(&key)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(me["resident"], resident);
    assert_eq!(me["owner"], json!(access.owner));

    // The server made an admin token, because the configuration had none.
    let admin = access.admin_token.clone().expect("an admin token");
    assert!(admin.starts_with("unsup_adm_") && admin.len() == 58, "{admin}");
    let (status, stats) = send(client.get("/admin/stats").bearer_auth(&admin)).await;
    assert_eq!(status, StatusCode::OK, "{stats}");
    assert_eq!(stats["store"]["owners"], 1);
    assert_eq!(stats["sim"]["residents"], 1);
    let (status, _) = send(client.get("/admin/stats").bearer_auth(&access.owner_token)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    // A debug print of the access holds no token.
    let shown = format!("{access:?}");
    assert!(shown.contains("owner"), "{shown}");
    assert!(!shown.contains(&access.owner_token) && !shown.contains(&admin), "{shown}");
    server.stop().await.expect("the server stops");

    // An admin token of the configuration stays: the server makes none.
    let mut with_admin = dev;
    with_admin.gateway.admin_token =
        Some(AdminToken::new("the-admin-token-of-the-file").expect("a long token"));
    let server = unsup_server::start(with_admin).await.expect("the server starts");
    let client = Client::of(&server);
    let access = server.dev.clone().expect("the development mode gives access");
    assert_eq!(access.admin_token, None);
    let request = client.get("/admin/stats").bearer_auth("the-admin-token-of-the-file");
    let (status, _) = send(request).await;
    assert_eq!(status, StatusCode::OK);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn without_the_development_mode_nothing_is_ready() {
    let server = unsup_server::start(config()).await.expect("the server starts");
    let client = Client::of(&server);
    assert!(server.dev.is_none());
    // No admin token in the configuration: the admin routes are closed.
    let (status, answer) = send(client.get("/admin/stats").bearer_auth("any-token-at-all")).await;
    assert_eq!(
        (status, answer["error"]["code"].clone()),
        (StatusCode::FORBIDDEN, json!("admin_disabled"))
    );
    // No owner exists: the first sign-up gets the id 1.
    let (status, owner) = send(client.post("/v1/owners", &json!({ "name": "First" }))).await;
    assert_eq!((status, owner["owner"]["id"].clone()), (StatusCode::CREATED, json!(1)));
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn accounts_survive_a_restart_and_resident_keys_end_with_the_old_world() {
    let dir = TempDir::new();
    // The server makes the directories of the path.
    let file = dir.path().join("nested").join("data").join("store.sqlite3");
    let with_file = || ServerConfig { store: StoreConfig::Sqlite(file.clone()), ..config() };

    let server = unsup_server::start(with_file()).await.expect("the server starts");
    let client = Client::of(&server);
    let token = client.sign_up("Ada").await;
    let (first, old_key) = client.resident(&token, "One").await;
    let (second, _) = client.resident(&token, "Two").await;
    assert_eq!((first, second), (1, 2));
    server.stop().await.expect("the server stops");
    assert!(file.is_file(), "the store file exists");

    // A second server on the same file. The world is new, so the
    // residents of the old world ended at the start, before any request.
    let server = unsup_server::start(with_file()).await.expect("the server starts again");
    let client = Client::of(&server);
    // The owners were kept: a new owner gets the next id.
    let (status, owner) = send(client.post("/v1/owners", &json!({ "name": "Bob" }))).await;
    assert_eq!((status, owner["owner"]["id"].clone()), (StatusCode::CREATED, json!(2)));
    let other_token = owner["token"].as_str().expect("an owner token");
    // The new world gives the id 1 again, here to another owner. The old
    // record with that id does not stand in the way.
    let (again, new_key) = client.resident(other_token, "Three").await;
    assert_eq!(again, 1);
    // The old key for the id 1 is unknown. It does not reach the new
    // resident with that id.
    let (status, answer) = send(client.get("/v1/me").bearer_auth(&old_key)).await;
    assert_eq!(
        (status, answer["error"]["code"].clone()),
        (StatusCode::UNAUTHORIZED, json!("invalid_token"))
    );
    let (status, me) = send(client.get("/v1/me").bearer_auth(&new_key)).await;
    assert_eq!(
        (status, me["name"].clone(), me["owner"].clone()),
        (StatusCode::OK, json!("Three"), json!(2))
    );
    // The first owner and its token are there, without residents.
    let (status, listed) = send(client.get("/v1/residents").bearer_auth(&token)).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["residents"], json!([]));
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_store_that_cannot_be_opened_stops_the_start() {
    let dir = TempDir::new();
    // A file where the data directory must be.
    let blocker = dir.write("blocker", "not a directory");
    let broken =
        ServerConfig { store: StoreConfig::Sqlite(blocker.join("store.sqlite3")), ..config() };
    let Err(error) = unsup_server::start(broken).await else {
        panic!("the start must fail");
    };
    let message = format!("{error:#}");
    assert!(message.contains("cannot make the data directory"), "{message}");
}

#[tokio::test(flavor = "multi_thread")]
async fn paths_without_a_route_get_a_viewer_file_or_a_json_error() {
    let dir = TempDir::new();
    dir.write("index.html", "<!doctype html><title>viewer</title>");
    dir.write("app.js", "console.log('viewer');");
    let join = dir.write("join.md", "# Join\n\nRead `/v1/catalogue`.\n");
    let mut with_viewer = ServerConfig { web_dir: Some(dir.path().to_path_buf()), ..config() };
    with_viewer.gateway.join_file = Some(join);
    let server = unsup_server::start(with_viewer).await.expect("the server starts");
    let client = Client::of(&server);

    let text = |path: &'static str| {
        let request = client.get(path);
        async move {
            let response = request.send().await.expect("the server answers");
            let kind = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            (response.status(), kind, response.text().await.expect("text"))
        }
    };
    // The viewer is at the root.
    let (status, kind, body) = text("/").await;
    assert_eq!((status, kind.as_str()), (StatusCode::OK, "text/html"));
    assert_eq!(body, "<!doctype html><title>viewer</title>");
    let (status, _, body) = text("/app.js").await;
    assert_eq!((status, body.as_str()), (StatusCode::OK, "console.log('viewer');"));
    let (status, _, body) = text("/missing.png").await;
    assert_eq!((status, body.as_str()), (StatusCode::NOT_FOUND, ""));

    // A path of the API that no route takes is a JSON error, never a file.
    for path in ["/v1/index.html", "/v1/dev/residents", "/admin/app.js", "/mcp/app.js"] {
        let (status, answer) = send(client.get(path)).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(
            answer,
            json!({ "error": { "code": "not_found", "message": "No route has this path." } }),
            "{path}"
        );
    }

    // The join instructions come from the configured file.
    let (status, kind, body) = text("/join.md").await;
    assert_eq!((status, kind.as_str()), (StatusCode::OK, "text/markdown; charset=utf-8"));
    assert_eq!(body, "# Join\n\nRead `/v1/catalogue`.\n");
    // The routes of the viewer service are in the same application.
    let (status, world) = send(client.get("/v1/world")).await;
    assert_eq!((status, world["type"].clone()), (StatusCode::OK, json!("layout")));
    server.stop().await.expect("the server stops");

    // Without viewer files and without a join file, both are JSON errors.
    let mut bare = config();
    bare.gateway.join_file = Some(dir.path().join("no-such-file.md"));
    let server = unsup_server::start(bare).await.expect("the server starts");
    let client = Client::of(&server);
    for path in ["/", "/app.js", "/join.md"] {
        let (status, answer) = send(client.get(path)).await;
        assert_eq!(
            (status, answer["error"]["code"].clone()),
            (StatusCode::NOT_FOUND, json!("not_found")),
            "{path}"
        );
    }
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_stop_ends_long_polls_and_sockets_at_once() {
    // A grace time that the test would notice.
    let patient = ServerConfig { shutdown_grace: Duration::from_secs(120), ..config() };
    let server = unsup_server::start(patient).await.expect("the server starts");
    let client = Client::of(&server);
    let sim = server.sim.clone();
    let token = client.sign_up("Ada").await;
    let (_, key) = client.resident(&token, "One").await;

    // An agent socket that is open, a long poll that waits, and a viewer.
    let mut request =
        format!("ws://{}/v1/ws", server.addr).into_client_request().expect("a request");
    request
        .headers_mut()
        .insert("Authorization", format!("Bearer {key}").parse().expect("a header value"));
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.expect("the socket");
    socket.send(Message::Text(r#"{"op":"ping"}"#.into())).await.expect("the frame is sent");
    let pong = socket.next().await.expect("the socket is open").expect("a frame");
    assert_eq!(pong, Message::Text(r#"{"op":"pong"}"#.into()));
    let poll = tokio::spawn(send(client.get("/v1/wait?timeout=60").bearer_auth(&key)));
    while sim.stats().await.expect("stats").waiting != 1 {
        tokio::task::yield_now().await;
    }
    let (mut viewer, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/v1/spectate", server.addr))
            .await
            .expect("the viewer connects");
    sim.step(1).await.expect("the world steps");
    let frame = viewer.next().await.expect("the feed is open").expect("a frame");
    let Message::Text(text) = frame else { panic!("a text frame, not {frame:?}") };
    let frame: Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(frame["frame"], 1);

    let started = Instant::now();
    server.stop().await.expect("the server stops");
    let took = started.elapsed();
    assert!(took < Duration::from_secs(60), "the stop waited for the grace time: {took:?}");

    let (status, answer) = poll.await.expect("the poll ran");
    assert_eq!(
        (status, answer["error"]["code"].clone()),
        (StatusCode::SERVICE_UNAVAILABLE, json!("shutdown"))
    );
    match socket.next().await {
        Some(Ok(Message::Close(Some(frame)))) => {
            assert_eq!((frame.code, frame.reason.as_str()), (CloseCode::Away, "the server stops"));
        }
        other => panic!("a close frame, not {other:?}"),
    }
    // The simulation stopped with the server, and the feed of the viewer
    // ended with it: the socket gives no frame any more.
    assert_eq!(sim.stats().await, Err(unsup_protocol::SimError::Shutdown));
    let end = tokio::time::timeout(Duration::from_secs(60), viewer.next())
        .await
        .expect("the feed of the viewer ends");
    assert!(!matches!(end, Some(Ok(Message::Text(_) | Message::Binary(_)))), "{end:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_that_does_not_end_is_cut_after_the_grace_time() {
    let grace = Duration::from_millis(500);
    let brief = ServerConfig { shutdown_grace: grace, ..config() };
    let server = unsup_server::start(brief).await.expect("the server starts");

    // A client that promises a body and never sends it. The server says
    // `100 Continue` when the route starts to read the body, so after
    // that line the request is in work.
    let mut slow = TcpStream::connect(server.addr).await.expect("a connection");
    let head = "POST /v1/owners HTTP/1.1\r\nHost: test\r\nContent-Type: application/json\r\n\
                Content-Length: 64\r\nExpect: 100-continue\r\n\r\n";
    slow.write_all(head.as_bytes()).await.expect("the head is sent");
    let mut seen = Vec::new();
    while !seen.ends_with(b"\r\n\r\n") {
        let byte = slow.read_u8().await.expect("the server answers the head");
        seen.push(byte);
    }
    assert_eq!(String::from_utf8_lossy(&seen), "HTTP/1.1 100 Continue\r\n\r\n");

    let started = Instant::now();
    server.stop().await.expect("the server stops");
    let took = started.elapsed();
    assert!(took >= grace, "the request did not get its grace time: {took:?}");
    // The cut ends the request at once. Without it, the stop would wait
    // for the grace time a second time.
    assert!(took < grace * 2, "the stop did not cut the request: {took:?}");
    // The request was cut: it got the answer `shutdown`, and then the
    // server closed the connection.
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), slow.read_to_end(&mut rest))
        .await
        .expect("the server ends the connection")
        .expect("the connection ends in order");
    let rest = String::from_utf8_lossy(&rest);
    assert!(rest.starts_with("HTTP/1.1 503 Service Unavailable\r\n"), "{rest}");
    let body = rest.split_once("\r\n\r\n").map(|(_, body)| body).unwrap_or_default();
    assert_eq!(body, r#"{"error":{"code":"shutdown","message":"The server stops."}}"#);
}
