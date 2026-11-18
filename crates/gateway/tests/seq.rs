//! The sequence number of an action report: the agent WebSocket and the
//! MCP cursor use it, so that a client gets no report a second time.

mod common;

use common::{Deadline, Harness, agent_socket, next_frame, send};
use reqwest::StatusCode;
use serde_json::{Value, json};

/// Starts work of one tick for a resident. Its report comes with the
/// next frame.
async fn work(h: &Harness, key: &str) {
    let action = json!({ "verb": "work", "args": { "ticks": 1 } });
    let (status, answer) = send(h.post("/v1/act", key, &action)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["acks"][0]["status"]["state"], "started", "{answer}");
}

/// Returns what a frame is: `report 4` or `notice 3`.
fn label(frame: &Value) -> String {
    let op = frame["op"].as_str().expect("an op");
    format!("{op} {}", frame[op]["seq"])
}

/// Reads `count` pushed frames and returns their labels.
async fn pushed(socket: &mut common::AgentSocket, count: usize) -> Vec<String> {
    let mut labels = Vec::new();
    for _ in 0..count {
        labels.push(label(&next_frame(socket).await));
    }
    labels
}

/// Makes the inbox of Ada: notice 1, report 2, notice 3, report 4,
/// notice 5.
async fn history(h: &Harness) -> (String, String) {
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    h.tell(&bob_key, ada, "first").await;
    for text in ["second", "third"] {
        work(h, &ada_key).await;
        h.step(1).await;
        h.tell(&bob_key, ada, text).await;
    }
    h.step(1).await;
    let (_, inbox) = send(h.get("/v1/observe?since=1&sections=reports", &ada_key)).await;
    let seqs = |list: &Value| -> Vec<u64> {
        list.as_array().expect("a list").iter().map(|e| e["seq"].as_u64().expect("seq")).collect()
    };
    assert_eq!(seqs(&inbox["notices"]), [1, 3, 5]);
    assert_eq!(seqs(&inbox["sections"]["reports"]), [2, 4], "a report has its sequence number");
    (ada_key, bob_key)
}

#[tokio::test]
async fn a_socket_with_two_cursors_gets_each_report_and_each_notice_one_time() {
    // The test holds six connections of one resident.
    let h = Harness::with_toml("[ws]\nconnections_per_resident = 0").await;
    let (ada_key, bob_key) = history(&h).await;

    // One cursor: the reports from it come first, then the notices.
    let mut all = agent_socket(&h, &ada_key, "?since=1").await;
    assert_eq!(
        pushed(&mut all, 5).await,
        ["report 2", "report 4", "notice 1", "notice 3", "notice 5"]
    );

    // A client that lost its connection after `report 2`, `report 4` and
    // `notice 1` has two cursors: the number after its last notice and
    // the number after its last report. It gets what it missed, and no
    // report a second time.
    let mut again = agent_socket(&h, &ada_key, "?since=2&reports_since=5").await;
    assert_eq!(pushed(&mut again, 2).await, ["notice 3", "notice 5"]);
    // With the notice cursor alone, as an older client sends it, the
    // reports from that number come again.
    let mut older = agent_socket(&h, &ada_key, "?since=2").await;
    assert_eq!(pushed(&mut older, 4).await, ["report 2", "report 4", "notice 3", "notice 5"]);
    // A client that lost its connection in the middle of the reports.
    let mut middle = agent_socket(&h, &ada_key, "?since=1&reports_since=3").await;
    assert_eq!(pushed(&mut middle, 4).await, ["report 4", "notice 1", "notice 3", "notice 5"]);
    // The report cursor alone: the notices are those from now on.
    let mut reports = agent_socket(&h, &ada_key, "?reports_since=4").await;
    assert_eq!(pushed(&mut reports, 1).await, ["report 4"]);

    // Later news comes one time on every connection, the one with both
    // cursors at the end of the inbox included.
    let mut current = agent_socket(&h, &ada_key, "?since=6&reports_since=6").await;
    work(&h, &ada_key).await;
    h.tell(&bob_key, 1, "fourth").await;
    h.step(1).await;
    for socket in [&mut all, &mut again, &mut older, &mut middle, &mut reports, &mut current] {
        assert_eq!(pushed(socket, 2).await, ["report 7", "notice 6"]);
    }

    // A cursor that is not a number is refused.
    let (status, _) = send(h.get("/v1/ws?reports_since=soon", &ada_key)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// Calls an MCP tool without a session. Returns the text of the result.
async fn tool(h: &Harness, key: &str, name: &str, arguments: Value) -> String {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": name, "arguments": arguments,
            "_meta": { "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                       "io.modelcontextprotocol/clientCapabilities": {} } } });
    let request = h
        .post("/mcp", key, &body)
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("Mcp-Method", "tools/call")
        .header("Mcp-Name", name);
    let (status, answer) = send(request).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["result"]["isError"], false, "{answer}");
    answer["result"]["content"][0]["text"].as_str().expect("a text").to_string()
}

#[tokio::test]
async fn two_mcp_calls_at_the_same_time_share_the_news_without_a_repeat() {
    let h = Harness::start().await;
    let (ada, ada_key) = h.agent("Ada").await;
    let (_, bob_key) = h.agent("Bob").await;
    let seen = tool(&h, &ada_key, "observe", json!({ "sections": [] })).await;
    assert!(seen.ends_with("news: none\n"), "{seen}");

    // Two waits of the same client, with the same cursor.
    work(&h, &ada_key).await;
    let waits: Vec<_> = (0..2)
        .map(|_| {
            let (h, key) = (&h, &ada_key);
            async move { tool(h, key, "wait", json!({ "timeout": 50 })).await }
        })
        .collect();
    let news = async {
        let deadline = Deadline::start();
        while h.sim.stats().await.expect("stats").waiting != 2 {
            deadline.again().await;
        }
        // A report and a notice for Ada in one frame. Both waits end.
        h.tell(&bob_key, ada, "for one of you").await;
        h.step(1).await;
    };
    let (answers, ()) = tokio::join!(futures::future::join_all(waits), news);

    // One call shows the report and the notice. The other one shows
    // nothing: the first call took the news.
    let with_news: Vec<&String> =
        answers.iter().filter(|text| !text.contains("news: none")).collect();
    assert_eq!(with_news.len(), 1, "{answers:?}");
    let text = with_news[0];
    assert!(text.contains("text: \"for one of you\""), "{text}");
    assert!(text.contains("result: {worked: 1}") && text.contains("seq: 2"), "{text}");
    let quiet = answers.iter().find(|text| text.contains("news: none")).expect("the other call");
    assert!(!quiet.contains("for one of you") && !quiet.contains("worked"), "{quiet}");

    // The cursor is behind the news: the next call has nothing either.
    let seen = tool(&h, &ada_key, "observe", json!({})).await;
    assert!(seen.ends_with("news: none\n"), "{seen}");
}
