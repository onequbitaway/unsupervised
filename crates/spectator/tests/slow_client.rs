//! A viewer that does not read: it loses frames, it gets new snapshots,
//! and it never holds the simulation.

mod common;

use std::time::Instant;

use serde_json::json;
use unsup_kernel::{Pos, ZoneId};
use unsup_runtime::RuntimeConfig;
use unsup_spectator::wire::ServerMessage;

use common::{Harness, Model, limits, resident};

/// A speech that is long enough to fill the buffers of a local socket in
/// a few hundred frames.
fn long_speech() -> String {
    "la ".repeat(20_000)
}

#[tokio::test]
async fn a_slow_viewer_loses_frames_and_gets_new_snapshots() {
    let mut config = limits();
    config.socket.client_queue = 8;
    config.socket.max_zones = 2;
    // This test is about the queue, not about the time limit of a message.
    config.socket.send_timeout_ms = 600_000;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;

    let (mut slow, _) = harness.viewer().await;
    slow.subscribe(&[0]).await;
    assert!(matches!(slow.next().await, ServerMessage::Snapshot { zone: ZoneId(0), .. }));

    // The viewer stops reading. Every frame carries a long speech and a
    // move. The frames go on until the server says that a viewer lost one.
    let speech = long_speech();
    let mut frames: u64 = 0;
    loop {
        frames += 1;
        assert!(frames < 5_000, "the viewer never lost a frame");
        harness.act(bob, "say", json!({ "text": speech })).await;
        // A new tile in every frame, so that an old state is never equal
        // to the new one.
        harness.act(ada, "hop", json!({ "x": 1 + frames % 14, "y": 1 + frames / 14 % 14 })).await;
        harness.step().await;
        let stats = harness.get_ok("/v1/spectate/stats").await;
        if stats["dropped_frames"].as_u64().expect("a number") > 0 {
            break;
        }
    }
    // The simulation ran every frame while the viewer did not read.
    let stats = harness.sim.stats().await.expect("stats");
    assert_eq!((stats.frame, stats.frames_run), (frames + 1, frames + 1));
    let truth = harness.residents(0).await;
    assert_eq!(
        truth,
        vec![
            resident(
                ada,
                "Ada",
                Pos::new(1 + (frames % 14) as i32, 1 + (frames / 14 % 14) as i32),
                None
            ),
            resident(bob, "Bob", common::START, None),
        ]
    );

    // The viewer reads again. First come the frames that were in the queue
    // and in the socket, in order and without a hole. Then, without one
    // more frame of the simulation, the world and a new snapshot. The loop
    // ends when the viewer knows the last frame.
    let mut model = Model::default();
    let mut delivered: u64 = 0;
    let mut dropped: u64 = 0;
    let mut worlds: u64 = 0;
    let mut owed_snapshot = false;
    let mut known = 1;
    while known < frames + 1 || owed_snapshot {
        let message = slow.next().await;
        model.apply(ZoneId(0), &message);
        match message {
            ServerMessage::Frame { frame, .. } => {
                assert_eq!(frame, known + 1, "the delivered frames have no hole");
                known = frame;
                delivered += 1;
            }
            ServerMessage::Delta { frame, .. } => assert_eq!(frame, known + 1),
            ServerMessage::World { frame, dropped: lost, .. } => {
                assert!(frame > known, "a world message at frame {frame} after frame {known}");
                assert!(lost >= 1, "a new world message counts the lost frames");
                known = frame;
                dropped += lost;
                worlds += 1;
                owed_snapshot = true;
            }
            ServerMessage::Snapshot { zone, frame, .. } => {
                assert!(owed_snapshot, "a snapshot without a world message before it");
                assert_eq!(zone, ZoneId(0));
                assert!(frame >= known);
                owed_snapshot = false;
            }
            other => panic!("not expected: {other:?}"),
        }
    }
    assert!(worlds >= 1, "the viewer got no new world message");
    assert!(delivered < frames, "{delivered} of {frames} frames were delivered");
    assert!(delivered + dropped <= frames, "{delivered} delivered, {dropped} lost of {frames}");
    // The viewer and the simulation agree again.
    assert_eq!(model.list(), truth);

    let stats = harness.get_ok("/v1/spectate/stats").await;
    assert_eq!(stats["dropped_frames"], json!(dropped));
    assert_eq!(stats["resyncs"], json!(worlds));

    // The deltas go on from the snapshot.
    harness.act(ada, "hop", json!({ "x": 9, "y": 9 })).await;
    harness.step().await;
    let messages = slow.until_frame(frames + 2).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    for message in &messages {
        model.apply(ZoneId(0), message);
    }
    assert_eq!(model.list(), harness.residents(0).await);
    assert_eq!(model.list()[0].pos, Pos::new(9, 9));
    harness.stop().await;
}

#[tokio::test]
async fn a_viewer_that_never_reads_is_disconnected() {
    let mut config = limits();
    config.socket.client_queue = 8;
    config.socket.max_zones = 2;
    config.socket.send_timeout_ms = 100;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;

    let (mut stuck, _) = harness.viewer().await;
    stuck.subscribe(&[0]).await;
    assert!(matches!(stuck.next().await, ServerMessage::Snapshot { zone: ZoneId(0), .. }));
    assert_eq!(harness.get_ok("/v1/spectate/stats").await["viewers"], json!(2));

    // The viewer never reads again. A message that cannot leave for 100
    // milliseconds ends the connection; the simulation goes on meanwhile.
    let speech = long_speech();
    let start = Instant::now();
    let mut frames: u64 = 0;
    loop {
        assert!(start.elapsed().as_secs() < 60, "the viewer was never disconnected");
        frames += 1;
        harness.act(bob, "say", json!({ "text": speech })).await;
        harness.step().await;
        if harness.get_ok("/v1/spectate/stats").await["viewers"] == json!(1) {
            break;
        }
    }
    assert_eq!(harness.sim.stats().await.expect("stats").frames_run, frames + 1);
    drop(stuck);
    harness.stop().await;
}
