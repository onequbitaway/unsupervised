//! The walking skeleton: an owner signs up, its agent acts through HTTP
//! and a viewer sees it.

mod common;

use std::time::Duration;

use common::send;
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use unsup_server::{ServerConfig, StoreConfig, WorldShape};

/// Reads the viewer feed until a `delta` message moves `resident`.
async fn next_move(
    socket: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    resident: u64,
) -> (i64, i64) {
    loop {
        let message = socket.next().await.expect("the feed ended").expect("the feed failed");
        let Message::Text(text) = message else { continue };
        let message: Value = serde_json::from_str(&text).expect("a feed message is JSON");
        if message["op"] != "delta" {
            continue;
        }
        for moved in message["moved"].as_array().into_iter().flatten() {
            if moved[0].as_u64() == Some(resident) {
                return (moved[1].as_i64().expect("x"), moved[2].as_i64().expect("y"));
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_resident_moves_through_http_and_a_viewer_sees_it() {
    let config = ServerConfig {
        bind: "127.0.0.1:0".parse().expect("an address"),
        tick_interval: Duration::from_millis(5),
        world: WorldShape { seed: 7, cities: 1, districts: 2 },
        web_dir: None,
        store: StoreConfig::Memory,
        ..ServerConfig::default()
    };
    let server = unsup_server::start(config).await.expect("the server starts");
    assert!(server.dev.is_none(), "this is not the development mode");
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    let health = http.get(format!("{base}/healthz")).send().await.expect("healthz");
    assert_eq!(health.status(), StatusCode::OK);

    let (mut viewer, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/v1/spectate", server.addr))
            .await
            .expect("the viewer connects");
    // The feed sends the residents of subscribed zones only.
    let world: Value = http
        .get(format!("{base}/v1/world"))
        .send()
        .await
        .expect("world")
        .json()
        .await
        .expect("layout");
    let zones: Vec<u64> = world["zones"]
        .as_array()
        .expect("zones")
        .iter()
        .map(|zone| zone["id"].as_u64().expect("a zone id"))
        .collect();
    assert_eq!(zones.len(), 4);
    viewer
        .send(Message::Text(json!({ "op": "subscribe", "zones": zones }).to_string().into()))
        .await
        .expect("the viewer subscribes");

    // An owner signs up and gets a token.
    let (status, owner) =
        send(http.post(format!("{base}/v1/owners")).json(&json!({ "name": "Grace" }))).await;
    assert_eq!(status, StatusCode::CREATED, "{owner}");
    let owner_token = owner["token"].as_str().expect("an owner token").to_string();
    assert!(owner_token.starts_with("unsup_own_"), "{owner_token}");

    // The owner makes a resident and gets its key.
    let (status, created) = send(
        http.post(format!("{base}/v1/residents"))
            .bearer_auth(&owner_token)
            .json(&json!({ "name": "Ada" })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let resident = created["resident"]["id"].as_u64().expect("a resident id");
    let key = created["key"].as_str().expect("a resident key").to_string();
    assert!(key.starts_with("unsup_res_"), "{key}");

    let (status, me) = send(http.get(format!("{base}/v1/me")).bearer_auth(&key)).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!((me["resident"].as_u64(), me["name"].as_str()), (Some(resident), Some("Ada")));

    let (status, before) = send(http.get(format!("{base}/v1/observe")).bearer_auth(&key)).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    let x = before["sections"]["self"]["pos"]["x"].as_i64().expect("x");
    let y = before["sections"]["self"]["pos"]["y"].as_i64().expect("y");
    let target = (x + 3, y + 2);
    let cursor = before["next_notice"].as_u64().expect("a notice cursor");

    let (status, answer) = send(
        http.post(format!("{base}/v1/act"))
            .bearer_auth(&key)
            .json(&json!({ "verb": "move_to", "args": { "x": target.0, "y": target.1 } })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"][0]["status"]["state"], "started", "answer: {answer}");
    assert_eq!(answer["acks"][0]["status"]["ticks"], 3);

    // The resident walks one tile in each tick. The viewer sees every step;
    // the last one is the target.
    let seen = tokio::time::timeout(Duration::from_secs(10), async {
        let mut steps = Vec::new();
        while steps.last() != Some(&target) {
            steps.push(next_move(&mut viewer, resident).await);
        }
        steps
    })
    .await
    .expect("the viewer sees the walk in time");
    assert_eq!(seen.len(), 3, "one visible step for each tick: {seen:?}");
    assert_eq!(seen.last(), Some(&target));

    // The long poll returns the report of the move.
    let (status, news) = send(
        http.get(format!("{base}/v1/wait?timeout=10&kinds=report&since={cursor}"))
            .bearer_auth(&key),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{news}");
    let report = &news["sections"]["reports"][0];
    assert_eq!(report["action_id"], answer["acks"][0]["action_id"]);
    assert_eq!(
        (report["verb"].clone(), report["ok"].clone(), report["result"].clone()),
        (json!("move_to"), json!(true), json!({ "x": target.0, "y": target.1 }))
    );

    let (_, after) = send(http.get(format!("{base}/v1/observe")).bearer_auth(&key)).await;
    assert_eq!(after["sections"]["self"]["pos"], json!({ "x": target.0, "y": target.1 }));

    // Without a token there is no observation.
    let (status, refused) = send(http.get(format!("{base}/v1/observe"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused["error"]["code"], "unauthorized");
    // The shortcuts of the first slice are gone: the resident id is not a
    // token, and no route makes a resident without an owner.
    let (status, refused) =
        send(http.get(format!("{base}/v1/observe")).bearer_auth(resident.to_string())).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(refused["error"]["code"], "invalid_token");
    let (status, refused) =
        send(http.post(format!("{base}/v1/dev/residents")).json(&json!({ "name": "Eve" }))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(refused["error"]["code"], "not_found");

    drop(viewer);
    server.stop().await.expect("the server stops");
}
