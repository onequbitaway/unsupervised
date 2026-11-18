//! The walking skeleton: an agent acts through HTTP and a viewer sees it.

use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use unsup_server::{ServerConfig, WorldShape};

/// Reads frames from the viewer socket until one moves `resident`.
async fn next_move(
    socket: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    resident: u64,
) -> (i64, i64) {
    loop {
        let message = socket.next().await.expect("the feed ended").expect("the feed failed");
        let Message::Text(text) = message else { continue };
        let frame: Value = serde_json::from_str(&text).expect("a frame is JSON");
        for zone in frame["zones"].as_array().into_iter().flatten() {
            for moved in zone["moved"].as_array().into_iter().flatten() {
                if moved[0].as_u64() == Some(resident) {
                    return (
                        moved[1]["x"].as_i64().expect("x"),
                        moved[1]["y"].as_i64().expect("y"),
                    );
                }
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
    };
    let server = unsup_server::start(config).await.expect("the server starts");
    let base = format!("http://{}", server.addr);
    let http = reqwest::Client::new();

    let health = http.get(format!("{base}/healthz")).send().await.expect("healthz");
    assert_eq!(health.status(), 200);

    let (mut viewer, _) =
        tokio_tungstenite::connect_async(format!("ws://{}/v1/spectate", server.addr))
            .await
            .expect("the viewer connects");

    let created: Value = http
        .post(format!("{base}/v1/dev/residents"))
        .json(&json!({ "name": "Ada" }))
        .send()
        .await
        .expect("create")
        .json()
        .await
        .expect("create answer");
    let resident = created["resident"].as_u64().expect("a resident id");
    let token = resident.to_string();

    let before: Value = http
        .get(format!("{base}/v1/observe"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("observe")
        .json()
        .await
        .expect("observation");
    let x = before["sections"]["self"]["pos"]["x"].as_i64().expect("x");
    let y = before["sections"]["self"]["pos"]["y"].as_i64().expect("y");
    let target = (x + 3, y + 2);

    let answer: Value = http
        .post(format!("{base}/v1/act"))
        .bearer_auth(&token)
        .json(&json!({ "verb": "move_to", "args": { "x": target.0, "y": target.1 } }))
        .send()
        .await
        .expect("act")
        .json()
        .await
        .expect("act answer");
    assert_eq!(answer["acks"][0]["status"]["state"], "started", "answer: {answer}");
    assert_eq!(answer["acks"][0]["status"]["ticks"], 3);

    let seen = tokio::time::timeout(Duration::from_secs(10), next_move(&mut viewer, resident))
        .await
        .expect("the viewer sees the move in time");
    assert_eq!(seen, target);

    let after: Value = http
        .get(format!("{base}/v1/observe"))
        .bearer_auth(&token)
        .send()
        .await
        .expect("observe")
        .json()
        .await
        .expect("observation");
    assert_eq!(after["sections"]["self"]["pos"], json!({ "x": target.0, "y": target.1 }));

    let refused = http.get(format!("{base}/v1/observe")).send().await.expect("observe");
    assert_eq!(refused.status(), 401);

    drop(viewer);
    server.stop().await.expect("the server stops");
}
