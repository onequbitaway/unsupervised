//! An agent over the agent WebSocket, in JSON and in MessagePack: the
//! same first day as over HTTPS, with pushed news, and a reconnect that
//! loses nothing and repeats nothing.

mod common;

use common::agent::Agent;
use common::link::{Link, accepted, report, state};
use common::socket::{Codec, Socket};
use common::story::first_day;
use common::{World, id};
use serde_json::{Value, json};

/// Plays the first day over a socket and checks what only a socket has:
/// every report comes as a pushed frame, one time, with the next
/// sequence number.
async fn first_day_over(codec: Codec) {
    let world = World::start().await;
    let (_, key) = world.newcomer("Ada").await;
    let mut socket = Socket::open(&world, &key, codec, "").await;
    socket.ping().await;

    let day = first_day(&world, &mut socket).await;
    assert!(day.shifts >= 1 && day.earned > 0, "{day:?}");

    // The socket was open from the first moment of the resident, so the
    // pushes are the whole inbox: every number from 1, without a gap.
    let seqs = socket.inbox.seqs();
    let count = seqs.len() as u64;
    assert!(count >= 9, "the day has at least nine reports: {seqs:?}");
    assert_eq!(seqs, (1..=count).collect::<Vec<u64>>(), "no number is missing");
    // An observation over the socket agrees with the pushes.
    let observation = socket.observe("brief", &["self"]).await;
    assert_eq!(id(&observation["next_notice"], "next_notice"), count + 1);
    assert_eq!(observation["sections"]["self"]["cash"], day.cash);

    socket.close().await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newcomer_lives_its_first_day_over_the_socket_in_json() {
    first_day_over(Codec::Json).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_newcomer_lives_its_first_day_over_the_socket_in_messagepack() {
    first_day_over(Codec::MsgPack).await;
}

/// Sends a private message from one resident to another over HTTPS.
async fn message(from: &mut Agent, to: u64, text: &str) {
    let ack = from.act("message", json!({ "to": to, "text": text })).await;
    assert_eq!(state(&ack), "done", "{ack}");
}

/// Returns the `seq` of each entry, in the order of the list.
fn seqs(news: &[Value]) -> Vec<u64> {
    news.iter().map(|entry| id(&entry["seq"], "seq")).collect()
}

/// Returns the tag and the `seq` of a pushed frame: a `report` or a
/// `notice`.
fn pushed_seq(frame: &Value) -> (String, u64) {
    let kind = frame["op"].as_str().expect("a frame has an `op`").to_string();
    assert!(kind == "report" || kind == "notice", "pushed news, not {frame}");
    let seq = id(&frame[kind.as_str()]["seq"], "seq");
    (kind, seq)
}

async fn a_reconnect_loses_and_repeats_nothing(codec: Codec) {
    let world = World::start().await;
    let (resident, key) = world.newcomer("Ada").await;
    let mut friend = Agent::join(&world, "Bo").await;

    // The first connection gets a notice and a report.
    let mut socket = Socket::open(&world, &key, codec, "").await;
    socket.ping().await;
    message(&mut friend, resident, "one").await;
    socket.news(&world).await;
    let ack = accepted(&mut socket, "wait", json!({ "ticks": 2 })).await;
    let first = report(&mut socket, &world, id(&ack["action_id"], "action_id")).await;
    assert_eq!(seqs(&socket.inbox.notices), [1], "the message is the first news");
    assert_eq!(id(&first["seq"], "seq"), 2, "the report has the next number");

    // The client starts three actions and loses the connection. While it
    // is away, the actions end and two more messages come.
    let plan = (1..=3)
        .map(|n| json!({ "id": format!("away-{n}"), "verb": "wait", "args": { "ticks": n } }));
    let acks = socket.plan(plan.collect()).await;
    assert_eq!(acks.len(), 3);
    let cursors = socket.cursors(1);
    assert_eq!(cursors, "since=2&reports_since=3");
    let before = socket.close().await;
    world.step(2).await;
    message(&mut friend, resident, "two").await;
    world.step(6).await;
    message(&mut friend, resident, "three").await;
    world.step(2).await;

    // The client connects again with its two cursors. The server pushes
    // what the client missed: first the reports, then the notices, each
    // in the order of `seq`.
    let mut socket = Socket::open(&world, &key, codec, &cursors).await;
    let mut order = Vec::new();
    for _ in 0..5 {
        order.push(pushed_seq(&common::soon(socket.frame()).await));
    }
    let pushed: Vec<(&str, u64)> = order.iter().map(|(kind, seq)| (kind.as_str(), *seq)).collect();
    assert_eq!(
        pushed,
        [("report", 3), ("report", 5), ("report", 6), ("notice", 4), ("notice", 7)],
        "the reports come first, then the notices"
    );

    // Nothing is lost and nothing came two times: the two connections
    // together have each number from 1 to 7 one time.
    let mut all = before.seqs();
    all.extend(order.iter().map(|(_, seq)| *seq));
    all.sort_unstable();
    assert_eq!(all, [1, 2, 3, 4, 5, 6, 7]);
    // Nothing more waits: the next frame is the answer to a ping.
    socket.ping().await;
    assert_eq!(socket.inbox.seqs(), Vec::<u64>::new(), "no more news was pushed");
    socket.close().await;

    // A client that sends `since` alone can get a report a second time:
    // the cursor of the notices is behind the last report.
    let mut socket = Socket::open(&world, &key, codec, "since=5").await;
    let mut again = Vec::new();
    for _ in 0..3 {
        again.push(pushed_seq(&common::soon(socket.frame()).await));
    }
    let again: Vec<(&str, u64)> = again.iter().map(|(kind, seq)| (kind.as_str(), *seq)).collect();
    assert_eq!(again, [("report", 5), ("report", 6), ("notice", 7)]);
    socket.close().await;
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_in_json_loses_and_repeats_nothing() {
    a_reconnect_loses_and_repeats_nothing(Codec::Json).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reconnect_in_messagepack_loses_and_repeats_nothing() {
    a_reconnect_loses_and_repeats_nothing(Codec::MsgPack).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribe_selects_the_pushes_and_the_server_answers_in_the_format_of_the_frame() {
    let world = World::start().await;
    let (resident, key) = world.newcomer("Ada").await;
    let mut friend = Agent::join(&world, "Bo").await;
    let mut socket = Socket::open(&world, &key, Codec::Json, "").await;

    // Only reports: a message is not pushed, a report is.
    socket.send(&json!({ "op": "subscribe", "kinds": ["report"] })).await;
    socket.ping().await;
    message(&mut friend, resident, "not pushed").await;
    world.step(2).await;
    let ack = accepted(&mut socket, "wait", json!({ "ticks": 1 })).await;
    let done = report(&mut socket, &world, id(&ack["action_id"], "action_id")).await;
    assert_eq!(id(&done["seq"], "seq"), 2, "the message took the number 1");
    assert_eq!(socket.inbox.notices.len(), 0, "the message was not pushed");

    // Only one kind of notice: a report is not pushed, the message is.
    socket.send(&json!({ "op": "subscribe", "kinds": ["society.message"] })).await;
    let ack = accepted(&mut socket, "wait", json!({ "ticks": 1 })).await;
    world.step(3).await;
    message(&mut friend, resident, "pushed").await;
    socket.news(&world).await;
    assert_eq!(seqs(&socket.inbox.notices), [4], "the report with the number 3 was not pushed");
    assert_eq!(socket.inbox.report_of(id(&ack["action_id"], "action_id")), None);

    // `subscribe` without `kinds` selects all pushes again.
    socket.send(&json!({ "op": "subscribe" })).await;
    let ack = accepted(&mut socket, "wait", json!({ "ticks": 1 })).await;
    report(&mut socket, &world, id(&ack["action_id"], "action_id")).await;
    message(&mut friend, resident, "pushed too").await;
    socket.news(&world).await;
    assert_eq!(socket.inbox.seqs(), [2, 4, 5, 6]);

    // A frame that is not a request gets an `error` frame, and the
    // connection stays open.
    let error = socket.refused(&json!({ "op": "dance" })).await;
    assert_eq!(error["code"], "invalid_request", "{error}");
    let error = socket.refused(&json!({ "op": "observe", "opts": { "since": 3 } })).await;
    assert_eq!(error["code"], "invalid_request", "`opts` has the fields of ObserveOpts: {error}");
    socket.ping().await;

    // The inbox that the pushes skipped is still there for `observe`.
    socket
        .send(&json!({ "op": "observe", "opts": { "since_notice": 1, "sections": ["reports"] } }))
        .await;
    let observation = socket.answer("observation").await["observation"].clone();
    assert_eq!(seqs(observation["notices"].as_array().expect("notices")), [1, 4, 6]);
    assert_eq!(seqs(observation["sections"]["reports"].as_array().expect("reports")), [2, 3, 5]);
    socket.close().await;
    world.stop().await;
}
