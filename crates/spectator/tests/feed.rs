//! The WebSocket feed: subscription, snapshot then deltas, zone filtering.

mod common;

use serde_json::json;
use tokio_tungstenite::tungstenite::Message;
use unsup_kernel::{Pos, ResidentId, ZoneId};
use unsup_protocol::{Clock, RealmId, Tick};
use unsup_runtime::RuntimeConfig;
use unsup_spectator::wire::{ClientMessage, ErrorBody, EventId, FeedEvent, ServerMessage};

use common::{Harness, Model, START, Viewer, limits, resident};

fn clock(minute: u8) -> Clock {
    Clock { year: 1, day: 1, hour: 0, minute }
}

/// The `frame` message of a frame in which nothing happened for the world.
fn quiet_frame(frame: u64, viewers: u32) -> ServerMessage {
    ServerMessage::Frame {
        frame,
        clock: clock(frame as u8),
        populations: Vec::new(),
        headlines: Vec::new(),
        viewers,
    }
}

fn error(message: ServerMessage) -> ErrorBody {
    match message {
        ServerMessage::Error { error } => error,
        other => panic!("not an error: {other:?}"),
    }
}

#[tokio::test]
async fn a_new_viewer_gets_the_world_first() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    assert_eq!(ada, ResidentId(1));
    harness.step().await;

    let (_viewer, world) = harness.viewer().await;
    assert_eq!(
        world,
        ServerMessage::World {
            protocol: 1,
            frame: 1,
            clock: clock(1),
            populations: vec![(ZoneId(0), 1), (ZoneId(1), 0), (ZoneId(2), 0), (ZoneId(3), 0)],
            dropped: 0,
            // The viewer of this test and the control viewer of the harness.
            viewers: 2,
        }
    );
    harness.stop().await;
}

#[tokio::test]
async fn a_subscription_gives_a_snapshot_and_then_deltas() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;

    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0]).await;
    assert_eq!(
        viewer.next().await,
        ServerMessage::Snapshot {
            zone: ZoneId(0),
            frame: 1,
            residents: vec![
                resident(ada, "Ada", START, None),
                resident(bob, "Bob", START.offset(1, 0), None)
            ],
        }
    );

    // One resident moves at once. The other starts an action of two ticks.
    harness.act(ada, "hop", json!({ "x": 7, "y": 5 })).await;
    harness.act(bob, "walk", json!({ "x": 3, "y": 9 })).await;
    harness.step().await;
    assert_eq!(
        viewer.until_frame(2).await,
        vec![
            ServerMessage::Delta {
                frame: 2,
                zone: ZoneId(0),
                tick: Tick(2),
                moved: vec![(ada, 7, 5)],
                entered: Vec::new(),
                left: Vec::new(),
                activity: vec![(bob, Some("walk".to_string()))],
                events: Vec::new(),
            },
            quiet_frame(2, 2),
        ]
    );

    // The action ends in the next frame: a move and no activity.
    harness.step().await;
    assert_eq!(
        viewer.until_frame(3).await,
        vec![
            ServerMessage::Delta {
                frame: 3,
                zone: ZoneId(0),
                tick: Tick(3),
                moved: vec![(bob, 3, 9)],
                entered: Vec::new(),
                left: Vec::new(),
                activity: vec![(bob, None)],
                events: Vec::new(),
            },
            quiet_frame(3, 2),
        ]
    );

    // A frame without a change in the zone sends only the `frame` message.
    harness.step().await;
    assert_eq!(viewer.until_frame(4).await, vec![quiet_frame(4, 2)]);

    // A viewer that comes later gets the state of now in its snapshot.
    let (mut late, _) = harness.viewer().await;
    late.subscribe(&[0]).await;
    assert_eq!(
        late.next().await,
        ServerMessage::Snapshot {
            zone: ZoneId(0),
            frame: 4,
            residents: vec![
                resident(ada, "Ada", Pos::new(7, 5), None),
                resident(bob, "Bob", Pos::new(3, 9), None),
            ],
        }
    );
    harness.stop().await;
}

#[tokio::test]
async fn a_viewer_gets_only_the_zones_that_it_subscribed_to() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;

    let (mut west, _) = harness.viewer().await;
    let (mut east, _) = harness.viewer().await;
    west.subscribe(&[0]).await;
    east.subscribe(&[1]).await;
    assert!(matches!(west.next().await, ServerMessage::Snapshot { zone: ZoneId(0), .. }));
    assert_eq!(
        east.next().await,
        ServerMessage::Snapshot { zone: ZoneId(1), frame: 1, residents: Vec::new() }
    );

    // A move inside zone 0: only the viewer of zone 0 gets a delta.
    harness.act(bob, "hop", json!({ "x": 2, "y": 2 })).await;
    harness.step().await;
    assert_eq!(
        west.until_frame(2).await,
        vec![
            ServerMessage::Delta {
                frame: 2,
                zone: ZoneId(0),
                tick: Tick(2),
                moved: vec![(bob, 2, 2)],
                entered: Vec::new(),
                left: Vec::new(),
                activity: Vec::new(),
                events: Vec::new(),
            },
            quiet_frame(2, 3),
        ]
    );
    assert_eq!(east.until_frame(2).await, vec![quiet_frame(2, 3)]);

    // Ada crosses to zone 1 and speaks there. Zone 0 sees her leave, zone 1
    // sees her enter and hears her. Both see the new numbers of residents.
    harness.act(ada, "hop", json!({ "x": 20, "y": 6 })).await;
    harness.act(ada, "say", json!({ "text": "Hello, east side." })).await;
    harness.step().await;
    let frame = ServerMessage::Frame {
        frame: 3,
        clock: clock(3),
        populations: vec![(ZoneId(0), 1), (ZoneId(1), 1)],
        headlines: Vec::new(),
        viewers: 3,
    };
    assert_eq!(
        west.until_frame(3).await,
        vec![
            ServerMessage::Delta {
                frame: 3,
                zone: ZoneId(0),
                tick: Tick(3),
                moved: Vec::new(),
                entered: Vec::new(),
                left: vec![ada],
                activity: Vec::new(),
                events: Vec::new(),
            },
            frame.clone(),
        ]
    );
    let said = FeedEvent {
        id: EventId { frame: 3, index: 0 },
        frame: 3,
        clock: clock(3),
        tick: Tick(2),
        realm: RealmId::Zone(ZoneId(1)),
        zone: Some(ZoneId(1)),
        kind: "society.said".to_string(),
        actor: Some(ada),
        actor_name: Some("Ada".to_string()),
        pos: Some(Pos::new(20, 6)),
        drama: 3,
        data: json!({ "text": { "untrusted": true, "from": 1, "text": "Hello, east side." } }),
    };
    assert_eq!(
        east.until_frame(3).await,
        vec![
            ServerMessage::Delta {
                frame: 3,
                zone: ZoneId(1),
                tick: Tick(3),
                moved: Vec::new(),
                entered: vec![resident(ada, "Ada", Pos::new(20, 6), None)],
                left: Vec::new(),
                activity: Vec::new(),
                events: vec![said],
            },
            frame,
        ]
    );
    harness.stop().await;
}

#[tokio::test]
async fn unsubscribe_stops_the_deltas_of_a_zone() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;

    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0, 1]).await;
    assert!(matches!(viewer.next().await, ServerMessage::Snapshot { zone: ZoneId(0), .. }));
    assert!(matches!(viewer.next().await, ServerMessage::Snapshot { zone: ZoneId(1), .. }));

    // Zone 3 was never subscribed: the answer names only zone 0.
    let zones = vec![ZoneId(0), ZoneId(3)];
    viewer.send(&ClientMessage::Unsubscribe { zones }).await;
    assert_eq!(viewer.next().await, ServerMessage::Unsubscribed { zones: vec![ZoneId(0)] });

    // Ada moves in zone 0, then to zone 1. The viewer sees only zone 1.
    harness.act(ada, "hop", json!({ "x": 1, "y": 1 })).await;
    harness.step().await;
    assert_eq!(viewer.until_frame(2).await, vec![quiet_frame(2, 2)]);
    harness.act(ada, "hop", json!({ "x": 17, "y": 1 })).await;
    harness.step().await;
    let messages = viewer.until_frame(3).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(
        matches!(&messages[0], ServerMessage::Delta { zone: ZoneId(1), entered, .. }
            if entered == &[resident(ada, "Ada", Pos::new(17, 1), None)]),
        "{messages:?}"
    );

    // A second subscription to the same zone gives a new snapshot.
    viewer.subscribe(&[0, 1]).await;
    assert_eq!(
        viewer.next().await,
        ServerMessage::Snapshot { zone: ZoneId(0), frame: 3, residents: Vec::new() }
    );
    // Zone 1 was subscribed already: no second snapshot for it.
    viewer.send(&ClientMessage::Ping).await;
    assert_eq!(viewer.next().await, ServerMessage::Pong);
    harness.stop().await;
}

#[tokio::test]
async fn a_bad_request_is_refused_and_changes_nothing() {
    let mut config = limits();
    config.socket.max_zones = 2;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    let (mut viewer, _) = harness.viewer().await;

    // An unknown zone refuses the whole request, also the good zone in it.
    viewer.subscribe(&[0, 99]).await;
    assert_eq!(
        error(viewer.next().await),
        ErrorBody {
            code: "unknown_zone".to_string(),
            message: "Zone 99 does not exist or is not served here.".to_string(),
        }
    );
    viewer.subscribe(&[0, 1, 2]).await;
    assert_eq!(
        error(viewer.next().await),
        ErrorBody {
            code: "too_many_zones".to_string(),
            message: "A viewer can subscribe to at most 2 zones.".to_string(),
        }
    );
    viewer.send_raw(Message::Text("subscribe to everything".into())).await;
    assert_eq!(error(viewer.next().await).code, "bad_request");
    viewer.send_raw(Message::Text(r#"{"op":"act","verb":"hop"}"#.into())).await;
    assert_eq!(error(viewer.next().await).code, "bad_request");
    viewer.send_raw(Message::Binary(vec![1, 2, 3].into())).await;
    assert_eq!(
        error(viewer.next().await),
        ErrorBody {
            code: "bad_request".to_string(),
            message: "Send JSON in text frames.".to_string(),
        }
    );

    // No refused request subscribed the viewer to zone 0.
    harness.act(ada, "hop", json!({ "x": 1, "y": 1 })).await;
    harness.step().await;
    assert_eq!(viewer.until_frame(2).await, vec![quiet_frame(2, 2)]);

    // The connection still works: two zones are allowed, the third is not.
    viewer.subscribe(&[0, 1]).await;
    assert!(matches!(viewer.next().await, ServerMessage::Snapshot { zone: ZoneId(0), .. }));
    assert!(matches!(viewer.next().await, ServerMessage::Snapshot { zone: ZoneId(1), .. }));
    viewer.subscribe(&[2]).await;
    assert_eq!(error(viewer.next().await).code, "too_many_zones");
    harness.stop().await;
}

#[tokio::test]
async fn a_message_above_the_size_limit_closes_the_connection() {
    let mut config = limits();
    config.socket.max_message_bytes = 64;
    let harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let (mut viewer, _) = harness.viewer().await;
    let zones: Vec<String> = (0..64).map(|zone| zone.to_string()).collect();
    let long = format!(r#"{{"op":"subscribe","zones":[{}]}}"#, zones.join(","));
    assert!(long.len() > 64);
    viewer.send_raw(Message::Text(long.into())).await;
    assert_eq!(viewer.try_next().await, None);
    harness.stop().await;
}

#[tokio::test]
async fn every_viewer_gets_the_headlines_and_only_them() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    // This viewer has no subscription.
    let (mut viewer, _) = harness.viewer().await;

    // One event below the headline score, one at it, one from the global
    // realm above it.
    harness.act(ada, "stir", json!({ "drama": 70 })).await;
    harness.act(ada, "stir", json!({ "drama": 71 })).await;
    harness.act(ada, "announce", json!({ "drama": 90 })).await;
    harness.step().await;
    let messages = viewer.until_frame(2).await;
    assert_eq!(messages.len(), 1, "{messages:?}");
    let ServerMessage::Frame { headlines, .. } = &messages[0] else {
        panic!("not a frame: {messages:?}");
    };
    assert_eq!(
        headlines.iter().map(|e| (e.id.to_string(), e.kind.as_str(), e.drama)).collect::<Vec<_>>(),
        // The global realm answers in the same frame; its events come first.
        [("2-0".to_string(), "town.announced", 90), ("2-2".to_string(), "town.stirred", 71)]
    );
    assert_eq!(
        (headlines[0].zone, headlines[0].realm, headlines[0].actor_name.as_deref()),
        (None, RealmId::Global, Some("Ada"))
    );
    assert_eq!((headlines[1].zone, headlines[1].pos), (Some(ZoneId(0)), Some(START)));
    harness.stop().await;
}

#[tokio::test]
async fn a_resident_that_leaves_and_comes_back_between_two_frames_is_there() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;
    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0, 1]).await;
    let mut home = Model::default();
    let mut away = Model::default();
    for _ in 0..2 {
        let snapshot = viewer.next().await;
        home.apply(ZoneId(0), &snapshot);
        away.apply(ZoneId(1), &snapshot);
    }

    // Ada goes to zone 1 and back before the next frame. Bob goes to zone
    // 1 and stays. The kernel lists Ada in `entered` and in `left` of both
    // zones; the feed must resolve that.
    harness.act(ada, "hop", json!({ "x": 20, "y": 6 })).await;
    harness.act(bob, "hop", json!({ "x": 21, "y": 6 })).await;
    harness.act(ada, "hop", json!({ "x": 8, "y": 8 })).await;
    harness.step().await;
    let messages = viewer.until_frame(2).await;
    assert_eq!(
        messages,
        vec![
            ServerMessage::Delta {
                frame: 2,
                zone: ZoneId(0),
                tick: Tick(2),
                moved: Vec::new(),
                entered: vec![resident(ada, "Ada", Pos::new(8, 8), None)],
                left: vec![bob],
                activity: Vec::new(),
                events: Vec::new(),
            },
            ServerMessage::Delta {
                frame: 2,
                zone: ZoneId(1),
                tick: Tick(2),
                moved: Vec::new(),
                entered: vec![resident(bob, "Bob", Pos::new(21, 6), None)],
                left: Vec::new(),
                activity: Vec::new(),
                events: Vec::new(),
            },
            ServerMessage::Frame {
                frame: 2,
                clock: clock(2),
                populations: vec![(ZoneId(0), 1), (ZoneId(1), 1)],
                headlines: Vec::new(),
                viewers: 2,
            },
        ]
    );
    for message in &messages {
        home.apply(ZoneId(0), message);
        away.apply(ZoneId(1), message);
    }
    // The viewer and the simulation agree about both zones.
    assert_eq!(home.list(), harness.residents(0).await);
    assert_eq!(away.list(), harness.residents(1).await);
    assert_eq!(home.list(), vec![resident(ada, "Ada", Pos::new(8, 8), None)]);
    harness.stop().await;
}

#[tokio::test]
async fn the_feed_ends_when_the_simulation_stops() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    let (mut viewer, _) = harness.viewer().await;

    harness.stop_simulation();
    // The server closes the feed of every viewer.
    assert_eq!(viewer.try_next().await, None);
    // A viewer that comes now is turned away at once.
    let mut late = Viewer::connect(harness.addr).await;
    assert_eq!(late.try_next().await, None);
    // The HTTP reads still answer, with the last state of the world.
    let world = harness.get_ok("/v1/world").await;
    assert_eq!((&world["frame"], &world["zones"][0]["residents"]), (&json!(1), &json!(1)));
    let profile = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    assert_eq!(profile["name"], "Ada");
    harness.stop().await;
}
