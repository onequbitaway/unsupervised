//! The show: the director and the stories on the feed and by HTTP, the
//! life story across a restart, replay clips, and the delay of the stream.

mod common;

use std::sync::Arc;

use reqwest::StatusCode;
use serde_json::{Value, json};
use unsup_kernel::{Pos, ZoneId};
use unsup_runtime::RuntimeConfig;
use unsup_runtime::storage::{FsStorage, MemStorage, Storage};
use unsup_spectator::Config;
use unsup_spectator::wire::{
    Channel, ClientMessage, Clip, Focus, ServerMessage, Shot, Story, StoryStatus, Zoom,
};

use common::{Harness, Model, START, limits, resident, world};

/// The limits of these tests: the director moves fast, so that a test
/// needs few frames.
fn fast() -> Config {
    let mut config = limits();
    config.director.wide_min_frames = 1;
    config.director.min_hold_frames = 3;
    config.director.breaking_hold_frames = 2;
    config
}

fn theft(thief: u64, victim: u64) -> Value {
    json!({
        "kind": "crime.theft_committed", "drama": 55,
        "data": { "thief": thief, "victim": victim, "amount": 4500 },
    })
}

fn shot_of(message: &ServerMessage) -> &Shot {
    match message {
        ServerMessage::Director { shot } => shot,
        other => panic!("not a director message: {other:?}"),
    }
}

fn stories_of(message: &ServerMessage) -> (&[Story], bool) {
    match message {
        ServerMessage::Stories { stories, reset } => (stories, *reset),
        other => panic!("not a stories message: {other:?}"),
    }
}

fn error_of(body: &Value) -> (&str, &str) {
    (
        body["error"]["code"].as_str().expect("an error code"),
        body["error"]["message"].as_str().expect("an error message"),
    )
}

#[tokio::test]
async fn the_director_and_the_stories_are_on_the_feed_and_on_http() {
    let mut harness = Harness::start_with(fast(), RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;

    // A viewer that chooses the channels gets the state of each at once.
    let (mut viewer, _) = harness.viewer().await;
    let (mut plain, _) = harness.viewer().await;
    viewer.tune(&[Channel::Director, Channel::Stories]).await;
    let first = viewer.next().await;
    let wide = shot_of(&first);
    // Nothing happened yet: a wide view of the one city.
    assert_eq!(
        (wide.seq, wide.frame, wide.zoom, &wide.focus, wide.caption.as_str(), wide.breaking),
        (1, 1, Zoom::City, &Focus::None, "Testville · 2 residents", false)
    );
    assert_eq!(stories_of(&viewer.next().await), (&[][..], true));
    let state = harness.get_ok("/v1/director").await;
    assert_eq!(state["frame"], 1);
    assert_eq!(state["shot"], serde_json::to_value(wide).expect("json"));

    // A theft: the director cuts to the thief, and a story opens. The show
    // comes before the `frame` message of its frame.
    harness.act(ada, "report", theft(1, 2)).await;
    harness.step().await;
    let messages = viewer.until_frame(2).await;
    assert_eq!(messages.len(), 3, "{messages:?}");
    let shot = shot_of(&messages[0]);
    assert_eq!(
        (shot.seq, shot.frame, shot.zoom, shot.breaking, shot.drama),
        (2, 2, Zoom::Close, false, 55)
    );
    assert_eq!(shot.focus, Focus::Resident { id: ada, name: Some("Ada".into()) });
    assert_eq!((shot.zone, shot.pos), (Some(ZoneId(0)), Some(START)));
    assert_eq!(shot.caption, "Ada stole $45.00 from Bob");
    assert_eq!(shot.event.map(|id| id.to_string()).as_deref(), Some("2-0"));
    assert_eq!(
        (shot.story, shot.label.as_deref(), shot.headline.as_deref()),
        (Some(1), Some("Crime"), Some("Ada stole $45.00 from Bob"))
    );
    let (stories, reset) = stories_of(&messages[1]);
    assert!(!reset);
    assert_eq!(stories.len(), 1);
    let story = &stories[0];
    assert_eq!(
        (story.id, story.kind.as_str(), story.status, story.events, story.headline.as_str()),
        (1, "crime", StoryStatus::Open, 1, "Ada stole $45.00 from Bob")
    );
    assert_eq!(
        story.residents.iter().map(|r| (r.id, r.name.as_str())).collect::<Vec<_>>(),
        [(ada, "Ada"), (bob, "Bob")]
    );
    assert!(matches!(messages[2], ServerMessage::Frame { frame: 2, .. }));
    // A viewer that did not choose the channels gets none of this.
    let others = plain.until_frame(2).await;
    assert_eq!(others.len(), 1, "{others:?}");

    // The same by HTTP.
    assert_eq!(
        harness.get_ok("/v1/director").await["shot"],
        serde_json::to_value(shot).expect("json")
    );
    let page = harness.get_ok("/v1/stories").await;
    assert_eq!((&page["frame"], page["stories"].as_array().map(Vec::len)), (&json!(2), Some(1)));
    assert_eq!(page["stories"][0], serde_json::to_value(story).expect("json"));
    assert_eq!(harness.get_ok("/v1/stories/1").await, page["stories"][0]);

    // The arrest joins the story. The shot of Ada stays (3 frames at
    // least) and gets the new words: the same number.
    let arrest = json!({ "kind": "crime.arrest_made", "drama": 50, "data": { "suspect": 1 } });
    harness.act(bob, "report", arrest).await;
    harness.step().await;
    let messages = viewer.until_frame(3).await;
    assert_eq!(messages.len(), 2, "{messages:?}");
    let (stories, _) = stories_of(&messages[0]);
    assert_eq!(
        (stories[0].id, stories[0].events, stories[0].latest.as_str()),
        (1, 2, "Ada was arrested")
    );
    assert_eq!(
        stories[0].timeline.iter().map(|beat| beat.event.to_string()).collect::<Vec<_>>(),
        ["2-0", "3-0"]
    );

    // Breaking news cuts in at once, before the least time of the shot.
    let died = json!({ "kind": "life.resident_died", "drama": 80, "data": { "resident": 2, "cause": "old_age", "age": 81 } });
    harness.act(bob, "report", died).await;
    harness.step().await;
    let messages = viewer.until_frame(4).await;
    let breaking = shot_of(&messages[0]);
    assert_eq!(
        (breaking.seq, breaking.frame, breaking.breaking, breaking.caption.as_str()),
        (3, 4, true, "Bob died at the age of 81 (old age)")
    );
    assert_eq!(breaking.focus, Focus::Resident { id: bob, name: Some("Bob".into()) });

    // A viewer that comes later gets the shot on air and every story.
    let (mut late, _) = harness.viewer().await;
    late.tune(&[Channel::Stories, Channel::Director]).await;
    assert_eq!(shot_of(&late.next().await), breaking);
    let all = late.next().await;
    let (stories, reset) = stories_of(&all);
    assert!(reset);
    // The death opened a story of its own. Open stories with the newest
    // event come first.
    assert_eq!(
        stories.iter().map(|story| (story.id, story.kind.as_str())).collect::<Vec<_>>(),
        [(2, "death"), (1, "crime")]
    );

    // Without channels the show stops for this viewer.
    viewer.tune(&[]).await;
    viewer.send(&ClientMessage::Ping).await;
    assert_eq!(viewer.next().await, ServerMessage::Pong);
    harness.act(ada, "report", theft(1, 2)).await;
    harness.step().await;
    assert_eq!(viewer.until_frame(5).await.len(), 1);
    harness.stop().await;
}

#[tokio::test]
async fn the_show_routes_refuse_what_is_not_valid() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    let (status, body) = harness.get("/v1/stories/9").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_story", "No story has the id 9."));
    let (status, body) = harness.get("/v1/stories/first").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body), ("invalid", "A story id is a number."));
    let (status, body) = harness.get("/v1/stories?status=old").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body), ("invalid", "`status` is `open` or `all`."));
    assert_eq!(harness.get_ok("/v1/stories?status=open&limit=1").await["stories"], json!([]));

    // A channel that does not exist is not a request.
    let (mut viewer, _) = harness.viewer().await;
    viewer
        .send_raw(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"op":"tune","channels":["weather"]}"#.into(),
        ))
        .await;
    match viewer.next().await {
        ServerMessage::Error { error } => assert_eq!(error.code, "bad_request"),
        other => panic!("not an error: {other:?}"),
    }

    let (status, body) = harness.get("/v1/places/12/history").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_place", "No site with the id 12 is known."));
    let (status, _) = harness.get("/v1/places/plaza/history").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    // Ada arrived on the plaza: the plaza has a history.
    let plaza = harness.get_ok("/v1/world/zones/0").await["sites"][0]["id"].clone();
    let history = harness.get_ok(&format!("/v1/places/{plaza}/history")).await;
    assert_eq!(
        (&history["site"], &history["name"], &history["zone"]),
        (&plaza, &json!("Plaza"), &json!(0))
    );
    assert_eq!(
        history["events"],
        json!([{
            "event": "1-0", "frame": 1, "clock": { "year": 1, "day": 1, "hour": 0, "minute": 1 },
            "kind": "kernel.spawned", "drama": 10, "line": "Ada arrived in Plaza",
            "zone": 0, "site": plaza,
        }])
    );
    assert_eq!(ada.0, 1);
    harness.stop().await;
}

/// Gives Ada a life: a job, a loan, a theft against her and an office.
async fn live_a_life(
    harness: &mut Harness,
) -> (unsup_kernel::ResidentId, unsup_kernel::ResidentId) {
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;
    let hired = json!({ "kind": "economy.worker_hired", "drama": 12, "data": { "resident": 1, "job": 3, "wage": 1200 } });
    let loan = json!({ "kind": "finance.loan_granted", "drama": 20, "data": { "resident": 1, "amount": 50_000 } });
    harness.act(ada, "report", hired).await;
    harness.act(ada, "report", loan).await;
    harness.act(bob, "report", theft(2, 1)).await;
    harness.step().await;
    let won = json!({ "kind": "politics.election_decided", "drama": 85, "data": { "city": 0, "office": "mayor", "winner": 1 } });
    harness.act(ada, "report", won).await;
    harness.step().await;
    (ada, bob)
}

fn lines(entries: &Value) -> Vec<&str> {
    entries
        .as_array()
        .expect("a list of entries")
        .iter()
        .map(|entry| entry["line"].as_str().expect("a line"))
        .collect()
}

#[tokio::test]
async fn a_life_story_survives_a_restart() {
    let storage = MemStorage::new();
    let shared: Arc<dyn Storage> = Arc::new(storage.clone());
    let mut harness =
        Harness::start_world(world(), limits(), RuntimeConfig::manual(), Arc::clone(&shared)).await;
    let (ada, _) = live_a_life(&mut harness).await;
    let before = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    let life = &before["life"];
    assert_eq!(life["born"]["line"], "Ada arrived in Plaza");
    assert_eq!(lines(&life["jobs"]), ["Ada got a job at a business"]);
    assert_eq!(lines(&life["deals"]), ["Ada got a loan of $500.00"]);
    assert_eq!(lines(&life["crimes"]), ["Bob stole $45.00 from Ada"]);
    assert_eq!(lines(&life["offices"]), ["Ada won the election for mayor in Testville"]);
    assert_eq!((life.get("death"), life.get("homes")), (None, None));

    // The server stops. The simulation keeps its world.
    let kept = harness.take_world();
    harness.feed_closed().await;
    harness.stop().await;
    assert!(!storage.list("chronicle").expect("list").is_empty());

    // A new service on the same world and the same storage.
    let mut harness =
        Harness::start_world(kept, limits(), RuntimeConfig::manual(), Arc::clone(&shared)).await;
    let after = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    assert_eq!(after["life"], before["life"]);
    // The feed of the new service starts empty: only the chronicle kept
    // the events.
    assert_eq!(after["events"], json!([]));
    // The life goes on in the same story.
    let lease = json!({ "kind": "economy.lease_started", "drama": 12, "data": { "tenant": 1, "rent": 800 } });
    harness.act(ada, "report", lease).await;
    harness.step().await;
    let life = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await["life"].clone();
    assert_eq!(lines(&life["homes"]), ["Ada rented a place for $8.00 a day"]);
    assert_eq!(lines(&life["jobs"]), ["Ada got a job at a business"]);
    harness.take_world();
    harness.feed_closed().await;
    harness.stop().await;

    // A new world has no past: the old chronicle is not its history.
    let mut harness =
        Harness::start_world(world(), limits(), RuntimeConfig::manual(), shared).await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    let life = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await["life"].clone();
    assert_eq!(life, json!({ "born": before["life"]["born"] }));
    harness.stop().await;
}

#[tokio::test]
async fn the_chronicle_and_a_pinned_clip_are_files_on_the_disk() {
    let dir = std::env::temp_dir().join(format!(
        "unsup-spectator-show-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    // One storage value for both services: a directory can be open one
    // time, and the tasks of the first service end after the test goes on.
    let storage: Arc<dyn Storage> = Arc::new(FsStorage::open(&dir).expect("a directory"));
    let open = || Arc::clone(&storage);
    let mut config = limits();
    config.clips.before_frames = 1;
    config.clips.after_frames = 1;
    let mut harness =
        Harness::start_world(world(), config.clone(), RuntimeConfig::manual(), open()).await;
    let (ada, _) = live_a_life(&mut harness).await;
    // The election was at frame 3. Its clip is complete at frame 4.
    harness.step().await;
    let (status, pin) = harness.post("/v1/clips/3-0/pin").await;
    assert_eq!(
        (status, &pin),
        (StatusCode::OK, &json!({ "id": "3-0", "pinned": true, "link": "/?clip=3-0" }))
    );
    let clip = harness.get_ok("/v1/clips/3-0").await;
    let kept = harness.take_world();
    harness.feed_closed().await;
    harness.stop().await;

    let journals: Vec<_> = std::fs::read_dir(dir.join("chronicle"))
        .expect("the directory of the chronicle")
        .map(|entry| entry.expect("an entry").file_name().into_string().expect("a name"))
        .collect();
    assert_eq!(journals, ["journal-0000000001.jsonl"]);
    assert!(dir.join("clips").join("3-0.json").is_file());

    let mut harness = Harness::start_world(kept, config, RuntimeConfig::manual(), open()).await;
    let life = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await["life"].clone();
    assert_eq!(lines(&life["offices"]), ["Ada won the election for mayor in Testville"]);
    // The pinned clip is there again, with every frame.
    assert_eq!(harness.get_ok("/v1/clips/3-0").await, clip);
    assert_eq!(harness.get_ok("/v1/clips").await["clips"][0]["id"], "3-0");
    // The service writes one more time when the simulation stops. The
    // test waits for that before it removes the directory.
    harness.take_world();
    harness.feed_closed().await;
    harness.stop().await;
    // The second service wrote a new journal and removed the first.
    assert!(dir.join("chronicle").join("journal-0000000002.jsonl").is_file());
    assert!(!dir.join("chronicle").join("journal-0000000001.jsonl").exists());
    std::fs::remove_dir_all(&dir).expect("the directory of the test goes");
}

#[tokio::test]
async fn a_clip_replays_the_frames_around_an_event() {
    let mut config = limits();
    config.clips.window_frames = 8;
    config.clips.before_frames = 2;
    config.clips.after_frames = 3;
    config.clips.auto_pin_drama = 101;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;
    // A viewer follows the zone live. The clip must show the same.
    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0]).await;
    let mut live = Model::default();
    live.apply(ZoneId(0), &viewer.next().await);
    let mut seen_at = std::collections::BTreeMap::new();

    // Ada walks east, one tile in each frame. Bob steals at frame 5.
    for frame in 2..=9u64 {
        harness.act(ada, "hop", json!({ "x": 6 + frame as i32, "y": 6 })).await;
        if frame == 5 {
            harness.act(bob, "report", theft(2, 1)).await;
        }
        if frame == 7 {
            harness.act(bob, "walk", json!({ "x": 2, "y": 2 })).await;
        }
        assert_eq!(harness.step().await, frame);
        for message in viewer.until_frame(frame).await {
            live.apply(ZoneId(0), &message);
        }
        seen_at.insert(frame, live.list());
    }

    // The theft is the second event of frame 5? No: Ada only moved. It is
    // the first.
    let clip: Clip = serde_json::from_value(harness.get_ok("/v1/clips/5-0").await).expect("a clip");
    assert_eq!((clip.id.to_string().as_str(), clip.zone, clip.pinned), ("5-0", ZoneId(0), false));
    assert_eq!((clip.from, clip.to, clip.complete), (3, 8, true));
    assert_eq!(clip.line, "Bob stole $45.00 from Ada");
    assert_eq!(clip.event.kind, "crime.theft_committed");
    assert_eq!(clip.link, "/?clip=5-0");
    // The clip starts with the residents as they were after frame 2.
    assert_eq!(clip.residents, seen_at[&2]);
    assert_eq!(
        clip.residents,
        vec![resident(ada, "Ada", Pos::new(8, 6), None), resident(bob, "Bob", START, None)]
    );
    assert_eq!(clip.frames.iter().map(|frame| frame.frame).collect::<Vec<_>>(), [3, 4, 5, 6, 7, 8]);
    assert_eq!(clip.frames[2].events.iter().map(|e| e.id.to_string()).collect::<Vec<_>>(), ["5-0"]);
    // A viewer that plays the clip sees after each frame what the live
    // viewer saw.
    let mut replay = Model::default();
    replay.apply(
        ZoneId(0),
        &ServerMessage::Snapshot { zone: ZoneId(0), frame: 2, residents: clip.residents.clone() },
    );
    for frame in &clip.frames {
        replay.apply(
            ZoneId(0),
            &ServerMessage::Delta {
                frame: frame.frame,
                zone: ZoneId(0),
                tick: frame.tick,
                moved: frame.moved.clone(),
                entered: frame.entered.clone(),
                left: frame.left.clone(),
                activity: frame.activity.clone(),
                events: frame.events.clone(),
            },
        );
        assert_eq!(replay.list(), seen_at[&frame.frame], "after frame {}", frame.frame);
    }
    // Frame 7: Bob started to walk. The clip has his activity.
    assert_eq!(clip.frames[4].activity, vec![(bob, Some("walk".to_string()))]);

    // A clip that is not complete says so, and a pin for it waits.
    harness.act(ada, "report", theft(1, 2)).await;
    assert_eq!(harness.step().await, 10);
    let fresh = harness.get_ok("/v1/clips/10-0").await;
    assert_eq!(
        (&fresh["from"], &fresh["to"], &fresh["complete"]),
        (&json!(8), &json!(10), &json!(false))
    );
    let (_, pin) = harness.post("/v1/clips/10-0/pin").await;
    assert_eq!(pin, json!({ "id": "10-0", "pinned": false, "link": "/?clip=10-0" }));
    assert_eq!(harness.get_ok("/v1/clips").await, json!({ "clips": [] }));
    // The clip of frame 5 is pinned at once: it is complete.
    let (status, pin) = harness.post("/v1/clips/5-0/pin").await;
    assert_eq!((status, &pin["pinned"]), (StatusCode::OK, &json!(true)));

    // 8 frames are kept. At frame 13 the frame 5 is gone, and the clip of
    // frame 10 is complete: the pin that waited is done.
    harness.steps(3).await;
    let list = harness.get_ok("/v1/clips").await;
    assert_eq!(
        list["clips"]
            .as_array()
            .expect("clips")
            .iter()
            .map(|c| c["id"].as_str().expect("id"))
            .collect::<Vec<_>>(),
        ["10-0", "5-0"]
    );
    assert_eq!(list["clips"][1]["line"], "Bob stole $45.00 from Ada");
    let pinned: Clip =
        serde_json::from_value(harness.get_ok("/v1/clips/5-0").await).expect("a clip");
    assert_eq!(pinned, Clip { pinned: true, ..clip });
    // An event of the same age that nobody pinned has no clip any more.
    harness.act(ada, "stir", json!({ "drama": 20 })).await;
    harness.steps(9).await;
    let (status, body) = harness.get("/v1/clips/14-0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        error_of(&body),
        ("unknown_clip", "The server does not have the frames around the event 14-0 any more.")
    );
    harness.stop().await;
}

#[tokio::test]
async fn a_clip_that_cannot_be_is_refused() {
    let mut config = limits();
    config.clips.public_pin = false;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    harness.act(ada, "announce", json!({ "drama": 40 })).await;
    harness.step().await;

    let (status, body) = harness.get("/v1/clips/2-0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_clip", "The event 2-0 has no place, so it has no clip."));
    let (status, body) = harness.get("/v1/clips/2-7").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(
        error_of(&body),
        ("unknown_clip", "The server has no public event with the id 2-7.")
    );
    let (status, body) = harness.get("/v1/clips/second").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body).0, "invalid");
    // This server does not let viewers pin.
    let (status, body) = harness.post("/v1/clips/1-0/pin").await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error_of(&body), ("pin_disabled", "This server does not let viewers pin clips."));
    // The clip itself is there.
    assert_eq!(harness.get_ok("/v1/clips/1-0").await["event"]["kind"], "kernel.spawned");
    harness.stop().await;

    let mut off = limits();
    off.clips.window_frames = 0;
    let mut harness = Harness::start_with(off, RuntimeConfig::manual()).await;
    harness.spawn("Ada").await;
    harness.step().await;
    let (status, body) = harness.get("/v1/clips/1-0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_clip", "This server keeps no clips."));
    harness.stop().await;
}

#[tokio::test]
async fn a_delay_holds_back_everything_that_viewers_get() {
    const DELAY: u64 = 3;
    let mut config = fast();
    config.stream.delay_frames = DELAY;
    config.map.refresh_frames = 1;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    // The simulation is at `live`. Viewers get the frame that is 3 older.
    let live = harness.step().await;
    let stats = harness.get_ok("/v1/spectate/stats").await;
    assert_eq!((&stats["delay_frames"], &stats["frame"]), (&json!(DELAY), &json!(live - DELAY)));
    assert_eq!(harness.get_ok("/v1/world").await["frame"], live - DELAY);
    let (mut viewer, world_message) = harness.viewer().await;
    assert!(
        matches!(world_message, ServerMessage::World { frame, .. } if frame == live - DELAY),
        "{world_message:?}"
    );
    viewer.subscribe(&[0]).await;
    viewer.tune(&[Channel::Director]).await;
    let mut model = Model::default();
    // The snapshot of the zone and the shot on air.
    model.apply(ZoneId(0), &viewer.next().await);
    let mut shots = vec![shot_of(&viewer.next().await).clone()];

    // In one frame Ada goes to another tile, steals and builds a shop.
    harness.act(ada, "hop", json!({ "x": 10, "y": 10 })).await;
    harness.act(ada, "report", theft(1, 2)).await;
    harness.act(ada, "build", json!({ "name": "Corner Shop" })).await;
    let acted = harness.step().await;
    assert_eq!(acted, live + 1);
    let theft_id = format!("{acted}-0");

    // For 3 frames nothing of it reaches a viewer, by no route.
    for frame in acted..acted + DELAY {
        if frame > acted {
            assert_eq!(harness.step().await, frame);
        }
        for message in viewer.until_frame(frame - DELAY).await {
            model.apply(ZoneId(0), &message);
            if let ServerMessage::Director { shot } = &message {
                shots.push(shot.clone());
            }
            // The arrival of Ada can come in these frames. The theft
            // cannot.
            assert!(
                !matches!(&message, ServerMessage::Delta { events, .. }
                    if events.iter().any(|event| event.kind.starts_with("crime."))),
                "{message:?}"
            );
        }
        // Ada was not in the world at the first frames of the delayed
        // view, or stands where she arrived.
        assert!(model.list().iter().all(|r| r.pos == START), "frame {frame}: {:?}", model.list());
        let zone = harness.get_ok("/v1/world/zones/0").await;
        assert!(
            zone["residents"]
                .as_array()
                .expect("residents")
                .iter()
                .all(|r| r["pos"] == json!({ "x": 6, "y": 6 })),
            "frame {frame}: {zone}"
        );
        assert_eq!(zone["sites"].as_array().expect("sites").len(), 1, "frame {frame}");
        let feed = harness.get_ok("/v1/feed?kind=crime.").await;
        assert_eq!(feed["events"], json!([]), "frame {frame}");
        assert_eq!(harness.get_ok("/v1/stories").await["stories"], json!([]), "frame {frame}");
        assert_eq!(
            harness.get_ok("/v1/director").await["shot"]["focus"],
            json!({ "type": "none" })
        );
        let (status, _) = harness.get(&format!("/v1/clips/{theft_id}")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "frame {frame}");
        let profile = harness.get(&format!("/v1/residents/{ada}/profile")).await.1;
        assert!(profile["life"].get("crimes").is_none(), "frame {frame}: {profile}");
        assert_eq!(shots.len(), 1, "frame {frame}: {shots:?}");
    }

    // The fourth frame brings all of it.
    let now = harness.step().await;
    assert_eq!(now, acted + DELAY);
    let messages = viewer.until_frame(acted).await;
    for message in &messages {
        model.apply(ZoneId(0), message);
    }
    assert_eq!(model.list(), vec![resident(ada, "Ada", Pos::new(10, 10), None)]);
    let cut = messages
        .iter()
        .find_map(|message| match message {
            ServerMessage::Director { shot } => Some(shot),
            _ => None,
        })
        .expect("the director cuts to the thief");
    // The shot has the frame of the event, not the frame of the viewers'
    // clock: the director saw the same events as without a delay.
    assert_eq!((cut.frame, cut.caption.as_str()), (acted, "Ada stole $45.00 from resident 2"));
    assert_eq!(cut.focus, Focus::Resident { id: ada, name: Some("Ada".into()) });
    let zone = harness.get_ok("/v1/world/zones/0").await;
    assert_eq!(zone["residents"][0]["pos"], json!({ "x": 10, "y": 10 }));
    let feed = harness.get_ok("/v1/feed?kind=crime.").await;
    assert_eq!(feed["events"][0]["id"], theft_id);
    assert_eq!(harness.get_ok("/v1/stories").await["stories"][0]["lead"], theft_id);
    assert_eq!(harness.get_ok(&format!("/v1/clips/{theft_id}")).await["zone"], 0);
    let profile = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    assert_eq!(lines(&profile["life"]["crimes"]), ["Ada stole $45.00 from resident 2"]);

    // The map of the zone waited like everything else. The first request
    // above read it again from the simulation, with the shop, in the frame
    // of the building. The cache got that map 3 frames later: now.
    assert_eq!(
        zone["sites"]
            .as_array()
            .expect("sites")
            .iter()
            .map(|s| s["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        ["Plaza", "Corner Shop"]
    );
    harness.stop().await;
}
