//! The HTTP reads: the world, the feed cursor, profiles, published state.

mod common;

use reqwest::StatusCode;
use serde_json::{Value, json};
use unsup_kernel::{Pos, ZoneId};
use unsup_runtime::RuntimeConfig;
use unsup_spectator::wire::ServerMessage;

use common::{Harness, Model, PLAZA, START, limits, resident};

/// Returns the ids of the events of a feed page.
fn ids(page: &Value) -> Vec<&str> {
    page["events"]
        .as_array()
        .expect("a list of events")
        .iter()
        .map(|event| event["id"].as_str().expect("an id"))
        .collect()
}

fn error_of(body: &Value) -> (&str, &str) {
    (
        body["error"]["code"].as_str().expect("an error code"),
        body["error"]["message"].as_str().expect("an error message"),
    )
}

#[tokio::test]
async fn the_world_routes_answer_from_the_cache() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.act(bob, "hop", json!({ "x": 30, "y": 30 })).await;
    harness.step().await;

    let world = harness.get_ok("/v1/world").await;
    assert_eq!(world["type"], "layout");
    assert_eq!(world["frame"], 1);
    assert_eq!(world["clock"], json!({ "year": 1, "day": 1, "hour": 0, "minute": 1 }));
    assert_eq!(
        world["cities"],
        json!([{
            "id": 0, "name": "Testville",
            "bounds": { "x0": 0, "y0": 0, "x1": 32, "y1": 32 },
            "zones": [0, 1, 2, 3],
        }])
    );
    let zones = world["zones"].as_array().expect("zones");
    assert_eq!(
        zones.iter().map(|z| z["residents"].as_u64().expect("a number")).collect::<Vec<_>>(),
        [1, 0, 0, 1]
    );
    assert_eq!(
        zones[3],
        json!({
            "id": 3, "city": 0, "name": "Testville 1-1",
            "bounds": { "x0": 16, "y0": 16, "x1": 32, "y1": 32 },
            "residents": 1,
        })
    );

    let zone = harness.get_ok("/v1/world/zones/0").await;
    assert_eq!(
        (&zone["type"], &zone["width"], &zone["height"]),
        (&json!("zone"), &json!(16), &json!(16))
    );
    let tiles = zone["tiles"].as_array().expect("tiles");
    assert_eq!(tiles.len(), 256);
    // The road on the north side, the plaza, and grass between them.
    assert_eq!((&tiles[0], &tiles[15], &tiles[16]), (&json!(1), &json!(1), &json!(0)));
    assert_eq!(tiles[(PLAZA.y0 * 16 + PLAZA.x0) as usize], 2);
    assert_eq!(zone["sites"].as_array().expect("sites").len(), 1);
    assert_eq!(
        (&zone["sites"][0]["kind"], &zone["sites"][0]["name"], &zone["sites"][0]["rect"]),
        (&json!("park"), &json!("Plaza"), &json!({ "x0": 4, "y0": 4, "x1": 9, "y1": 9 }))
    );
    assert_eq!(harness.residents(0).await, vec![resident(ada, "Ada", START, None)]);
    assert_eq!(harness.residents(3).await, vec![resident(bob, "Bob", Pos::new(30, 30), None)]);

    // `residents=false` leaves the residents out and keeps the count.
    let bare = harness.get_ok("/v1/world/zones/0?residents=false").await;
    assert_eq!((&bare["residents"], &bare["zone"]["residents"]), (&json!([]), &json!(1)));
    assert_eq!(bare["tiles"], zone["tiles"]);

    let (status, body) = harness.get("/v1/world/zones/4").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_zone", "Zone 4 does not exist or is not served here."));
    let (status, body) = harness.get("/v1/world/zones/north").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body), ("invalid", "A zone id is a number from 0."));
    let (status, body) = harness.get("/v1/world/zones/0?residents=some").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body).0, "invalid");

    let stats = harness.get_ok("/v1/spectate/stats").await;
    assert_eq!(
        stats,
        json!({
            "frame": 1, "residents": 2, "viewers": 1, "dropped_frames": 0, "resyncs": 0,
            "delay_frames": 0,
        })
    );
    harness.stop().await;
}

#[tokio::test]
async fn the_map_of_a_zone_is_read_again_when_it_is_old() {
    let mut config = limits();
    config.map.refresh_frames = 5;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    harness.act(ada, "hop", json!({ "x": 12, "y": 12 })).await;
    harness.act(ada, "build", json!({ "name": "Corner Shop" })).await;
    let names = |zone: &Value| -> Vec<String> {
        zone["sites"]
            .as_array()
            .expect("sites")
            .iter()
            .map(|site| site["name"].as_str().expect("a name").to_string())
            .collect()
    };
    let tile = |zone: &Value| zone["tiles"][12 * 16 + 12].clone();

    // The cache read the map at frame 0. It keeps that copy for 5 frames.
    harness.steps(4).await;
    let zone = harness.get_ok("/v1/world/zones/0").await;
    assert_eq!((names(&zone), tile(&zone)), (vec!["Plaza".to_string()], json!(0)));
    // The residents are always current.
    assert_eq!(zone["residents"][0]["pos"], json!({ "x": 12, "y": 12 }));

    harness.step().await;
    let zone = harness.get_ok("/v1/world/zones/0").await;
    assert_eq!(
        (names(&zone), tile(&zone)),
        (vec!["Plaza".to_string(), "Corner Shop".to_string()], json!(4))
    );
    harness.stop().await;
}

#[tokio::test]
async fn the_feed_has_a_cursor() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    // Frame 1: the kernel says that Ada came. Frames 2 to 4: two events each.
    harness.step().await;
    for drama in [5, 20, 80] {
        harness.act(ada, "stir", json!({ "drama": drama })).await;
        harness.act(ada, "say", json!({ "text": format!("drama {drama}") })).await;
        harness.step().await;
    }

    // Without a cursor: the newest events, oldest first.
    let page = harness.get_ok("/v1/feed?limit=3").await;
    assert_eq!(ids(&page), ["3-1", "4-0", "4-1"]);
    assert_eq!(
        (&page["next"], &page["more"], &page["gap"]),
        (&json!("4-1"), &json!(false), &json!(false))
    );
    assert_eq!(
        page["events"][1],
        json!({
            "id": "4-0", "frame": 4,
            "clock": { "year": 1, "day": 1, "hour": 0, "minute": 4 },
            "tick": 3, "realm": { "zone": 0 }, "zone": 0,
            "kind": "town.stirred", "actor": 1, "actor_name": "Ada",
            "pos": { "x": 6, "y": 6 }, "drama": 80, "data": {},
        })
    );

    // With the start cursor: everything, page by page, nothing twice.
    let first = harness.get_ok("/v1/feed?after=0-0&limit=3").await;
    assert_eq!(ids(&first), ["1-0", "2-0", "2-1"]);
    assert_eq!((&first["next"], &first["more"]), (&json!("2-1"), &json!(true)));
    assert_eq!(first["events"][0]["kind"], "kernel.spawned");
    let second = harness.get_ok("/v1/feed?after=2-1&limit=3").await;
    assert_eq!(ids(&second), ["3-0", "3-1", "4-0"]);
    assert_eq!(second["more"], true);
    let third = harness.get_ok("/v1/feed?after=4-0&limit=3").await;
    assert_eq!(ids(&third), ["4-1"]);
    assert_eq!((&third["next"], &third["more"]), (&json!("4-1"), &json!(false)));

    // The cursor of the last page gives nothing until a new event comes.
    let idle = harness.get_ok("/v1/feed?after=4-1").await;
    assert_eq!((ids(&idle), &idle["next"]), (Vec::<&str>::new(), &json!("4-1")));
    harness.step().await;
    let still = harness.get_ok("/v1/feed?after=4-1").await;
    assert_eq!((ids(&still), &still["next"]), (Vec::<&str>::new(), &json!("4-1")));
    harness.act(ada, "stir", json!({ "drama": 1 })).await;
    harness.step().await;
    let new = harness.get_ok("/v1/feed?after=4-1").await;
    assert_eq!((ids(&new), &new["next"]), (vec!["6-0"], &json!("6-0")));

    // Filters: a lowest drama score, the start of the kind.
    let notable = harness.get_ok("/v1/feed?after=0-0&min_drama=20").await;
    assert_eq!(ids(&notable), ["3-0", "4-0"]);
    assert_eq!(notable["next"], "4-0");
    let speech = harness.get_ok("/v1/feed?after=0-0&kind=society.").await;
    assert_eq!(ids(&speech), ["2-1", "3-1", "4-1"]);
    // The cursor moved past the events that the filter hid.
    assert_eq!(speech["next"], "6-0");
    assert_eq!(
        speech["events"][0]["data"],
        json!({ "text": { "untrusted": true, "from": 1, "text": "drama 5" } })
    );
    let both = harness.get_ok("/v1/feed?after=0-0&kind=town.&min_drama=50&limit=1").await;
    assert_eq!(ids(&both), ["4-0"]);

    // The greatest page is 500 events; a limit of zero means one.
    let many = harness.get_ok("/v1/feed?limit=100000").await;
    assert_eq!(ids(&many).len(), 8);
    let one = harness.get_ok("/v1/feed?limit=0").await;
    assert_eq!(ids(&one), ["6-0"]);

    for bad in
        ["/v1/feed?after=four", "/v1/feed?after=4", "/v1/feed?limit=-1", "/v1/feed?min_drama=300"]
    {
        let (status, body) = harness.get(bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        assert_eq!(error_of(&body).0, "invalid", "{bad}");
    }
    let (_, body) = harness.get("/v1/feed?after=four").await;
    assert_eq!(
        error_of(&body).1,
        "`after` is not a cursor: an event id is two numbers with a dash, for example 120-3."
    );
    harness.stop().await;
}

#[tokio::test]
async fn a_cursor_that_is_too_old_reports_a_gap() {
    let mut config = limits();
    config.feed.recent_events = 3;
    config.feed.notable_events = 2;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;
    for drama in [1, 2, 30, 3, 40, 50] {
        harness.act(ada, "stir", json!({ "drama": drama })).await;
        harness.step().await;
    }
    // Seven events came: `1-0` (drama 10) to `7-0`. The cache keeps the
    // last three, and the last two with a drama of 11 or more.
    let page = harness.get_ok("/v1/feed?after=1-0").await;
    assert_eq!((ids(&page), &page["gap"]), (vec!["5-0", "6-0", "7-0"], &json!(true)));
    // `4-0` is the last event that was dropped: nothing is lost after it.
    let page = harness.get_ok("/v1/feed?after=4-0").await;
    assert_eq!((ids(&page), &page["gap"]), (vec!["5-0", "6-0", "7-0"], &json!(false)));
    // A question for notable events reads the list that keeps them longer.
    let page = harness.get_ok("/v1/feed?after=1-0&min_drama=11").await;
    assert_eq!((ids(&page), &page["gap"]), (vec!["6-0", "7-0"], &json!(true)));
    let page = harness.get_ok("/v1/feed?after=4-0&min_drama=11").await;
    assert_eq!((ids(&page), &page["gap"]), (vec!["6-0", "7-0"], &json!(false)));
    harness.stop().await;
}

#[tokio::test]
async fn a_profile_says_who_where_and_what() {
    let mut config = limits();
    config.profile.events = 3;
    let mut harness = Harness::start_with(config, RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;
    harness.act(ada, "hop", json!({ "x": 20, "y": 22 })).await;
    harness.act(ada, "stir", json!({ "drama": 30 })).await;
    harness.act(bob, "stir", json!({ "drama": 4 })).await;
    // An event about Ada that Bob made: no actor, `resident` in the data.
    harness.act(bob, "stir", json!({ "drama": 6, "about": ada })).await;
    harness.act(ada, "walk", json!({ "x": 21, "y": 22 })).await;
    harness.step().await;

    let profile = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    let events = profile["events"].clone();
    let mut rest = profile.clone();
    rest.as_object_mut().expect("an object").remove("events");
    // The life story has its own test (`tests/show.rs`).
    let life = rest.as_object_mut().expect("an object").remove("life").expect("a life story");
    assert_eq!(life["born"]["line"], "Ada arrived in Plaza");
    assert_eq!(
        rest,
        json!({
            "id": 1, "name": "Ada", "model": "test-model", "alive": true,
            "zone": 3, "zone_name": "Testville 1-1", "city": 0, "city_name": "Testville",
            "pos": { "x": 20, "y": 22 }, "activity": "walk", "frame": 2,
        })
    );
    // Her arrival, the event about her, and her own event. Not Bob's own,
    // which is `2-0`: the events of zone 0 come before those of zone 3.
    assert_eq!(
        events
            .as_array()
            .expect("events")
            .iter()
            .map(|e| (
                e["id"].as_str().expect("id"),
                e["kind"].as_str().expect("kind"),
                e["drama"].as_u64().expect("drama")
            ))
            .collect::<Vec<_>>(),
        [("1-0", "kernel.spawned", 10), ("2-1", "town.stirred", 6), ("2-2", "town.stirred", 30)]
    );
    assert_eq!((events[1].get("actor"), &events[1]["data"]), (None, &json!({ "resident": 1 })));
    assert_eq!(events[2]["actor"], 1);

    // The profile keeps the newest three events.
    harness.act(ada, "say", json!({ "text": "On my way." })).await;
    harness.step().await;
    let profile = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    assert_eq!(ids(&profile), ["2-1", "2-2", "3-0"]);
    assert_eq!((&profile["pos"], profile.get("activity")), (&json!({ "x": 21, "y": 22 }), None));

    // A resident that left the world keeps its last place and the cause.
    harness.act(ada, "leave", json!({})).await;
    harness.step().await;
    let profile = harness.get_ok(&format!("/v1/residents/{ada}/profile")).await;
    assert_eq!(
        (
            &profile["alive"],
            &profile["cause"],
            &profile["frame"],
            &profile["zone"],
            &profile["pos"]
        ),
        (&json!(false), &json!("left"), &json!(4), &json!(3), &json!({ "x": 21, "y": 22 }))
    );
    assert_eq!(ids(&profile), ["2-2", "3-0", "4-0"]);
    assert_eq!(profile["events"][2]["kind"], "kernel.despawned");
    assert_eq!(profile["events"][2]["actor_name"], "Ada");
    assert_eq!(harness.get_ok("/v1/world").await["zones"][3]["residents"], 0);

    let (status, body) = harness.get("/v1/residents/99/profile").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error_of(&body), ("unknown_resident", "No resident with the id 99 is known."));
    let (status, body) = harness.get("/v1/residents/ada/profile").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body), ("invalid", "A resident id is a number."));
    harness.stop().await;
}

#[tokio::test]
async fn published_state_is_served_by_kind_and_key() {
    let mut harness = Harness::start().await;
    let ada = harness.spawn("Ada").await;
    harness.step().await;

    // Nothing is published yet: every route says so with a 404.
    for (path, message) in [
        ("/v1/stats/economy", "No module has published `finance.stats_published` yet."),
        (
            "/v1/leaderboards/wealth",
            "No module has published the leaderboard `wealth` (`missions.leaderboard_published`) yet.",
        ),
        (
            "/v1/published/finance.stats_published",
            "No module has published `finance.stats_published` yet.",
        ),
        (
            "/v1/published/finance.stats_published/north",
            "No module has published `finance.stats_published` with the key `north` yet.",
        ),
    ] {
        let (status, body) = harness.get(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(error_of(&body), ("not_published", message), "{path}");
    }
    assert_eq!(harness.get_ok("/v1/published").await, json!({ "published": [] }));
    let (status, body) = harness.get("/v1/published/town.stirred").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(error_of(&body), ("invalid", "The kind of published state ends in `_published`."));

    // The global realm answers one frame after the request.
    harness.act(ada, "publish_stats", json!({ "money_supply": 1000 })).await;
    harness.step().await;
    harness.act(ada, "publish_stats", json!({ "key": "north", "money_supply": 400 })).await;
    harness.step().await;
    harness.act(ada, "publish_board", json!({ "board": "wealth", "top": ada })).await;
    harness.step().await;
    harness.act(ada, "publish_stats", json!({ "money_supply": 1200 })).await;
    harness.step().await;

    let clock = |minute: u8| json!({ "year": 1, "day": 1, "hour": 0, "minute": minute });
    let national = json!({
        "kind": "finance.stats_published", "frame": 5, "clock": clock(5),
        "data": { "money_supply": 1200 },
    });
    let north = json!({
        "kind": "finance.stats_published", "key": "north", "frame": 3, "clock": clock(3),
        "data": { "key": "north", "money_supply": 400 },
    });
    // The newest value of each series. The series without a key comes first.
    let economy = harness.get_ok("/v1/stats/economy").await;
    assert_eq!(economy, json!({ "kind": "finance.stats_published", "series": [national, north] }));
    assert_eq!(harness.get_ok("/v1/published/finance.stats_published").await, economy);
    assert_eq!(harness.get_ok("/v1/published/finance.stats_published/north").await, north);

    let board = harness.get_ok("/v1/leaderboards/wealth").await;
    assert_eq!(
        board,
        json!({
            "kind": "missions.leaderboard_published", "key": "wealth", "frame": 4, "clock": clock(4),
            "data": { "key": "wealth", "rows": [{ "resident": 1, "score": 100 }] },
        })
    );
    assert_eq!(harness.get("/v1/leaderboards/fame").await.0, StatusCode::NOT_FOUND);
    assert_eq!(
        harness.get_ok("/v1/published").await,
        json!({ "published": [
            { "kind": "finance.stats_published", "frame": 5 },
            { "kind": "finance.stats_published", "key": "north", "frame": 3 },
            { "kind": "missions.leaderboard_published", "key": "wealth", "frame": 4 },
        ] })
    );

    // Published state is not news: the feed has only the arrival of Ada.
    assert_eq!(ids(&harness.get_ok("/v1/feed?after=0-0").await), ["1-0"]);
    harness.stop().await;
}

#[tokio::test]
async fn a_cache_that_lost_frames_reads_the_world_again() {
    // The simulation keeps 2 frames for a subscriber that is behind.
    let runtime = RuntimeConfig { frame_buffer: 2, ..RuntimeConfig::manual() };
    let mut harness = Harness::start_with(limits(), runtime).await;
    let ada = harness.spawn("Ada").await;
    let bob = harness.spawn("Bob").await;
    harness.step().await;
    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0, 1]).await;
    let mut west = Model::default();
    let mut east = Model::default();
    for _ in 0..2 {
        let snapshot = viewer.next().await;
        west.apply(ZoneId(0), &snapshot);
        east.apply(ZoneId(1), &snapshot);
    }

    // Ada walks to zone 1 and Bob leaves the world, while this thread, the
    // only thread of the server, is blocked for 40 frames. The cache
    // cannot read them in time and loses most of them.
    harness.act(ada, "walk", json!({ "x": 18, "y": 3 })).await;
    harness.act(bob, "leave", json!({})).await;
    let sim = harness.sim.clone();
    let frame = std::thread::spawn(move || futures::executor::block_on(sim.step(40)))
        .join()
        .expect("the stepping thread ends")
        .expect("step");
    assert_eq!(frame, 41);

    // The viewer gets the world and both zones again, from the new read.
    let mut worlds = 0;
    let mut snapshots = 0;
    while snapshots < 2 {
        let message = viewer.next().await;
        west.apply(ZoneId(0), &message);
        east.apply(ZoneId(1), &message);
        match message {
            ServerMessage::World { frame, populations, .. } => {
                worlds += 1;
                assert_eq!(frame, 41);
                assert_eq!(populations[..2], [(ZoneId(0), 0), (ZoneId(1), 1)]);
            }
            ServerMessage::Snapshot { frame, .. } => {
                assert_eq!(worlds, 1, "the world message comes before the snapshots");
                assert_eq!(frame, 41);
                snapshots += 1;
            }
            // Frames that the cache applied before it saw the loss.
            ServerMessage::Delta { .. } | ServerMessage::Frame { .. } => assert_eq!(worlds, 0),
            other => panic!("not expected: {other:?}"),
        }
    }
    assert_eq!(west.list(), Vec::new());
    assert_eq!(east.list(), vec![resident(ada, "Ada", Pos::new(18, 3), None)]);
    assert_eq!(harness.residents(1).await, east.list());
    let stats = harness.get_ok("/v1/spectate/stats").await;
    assert_eq!((&stats["frame"], &stats["residents"]), (&json!(41), &json!(1)));

    // The feed goes on after the new read.
    harness.act(ada, "hop", json!({ "x": 19, "y": 3 })).await;
    harness.step().await;
    for message in viewer.until_frame(42).await {
        east.apply(ZoneId(1), &message);
    }
    assert_eq!(east.list(), vec![resident(ada, "Ada", Pos::new(19, 3), None)]);
    harness.stop().await;
}

#[tokio::test]
async fn the_debug_page_is_served() {
    let harness = Harness::start().await;
    let response =
        reqwest::get(format!("http://{}/debug", harness.addr)).await.expect("a response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").and_then(|v| v.to_str().ok()),
        Some("text/html; charset=utf-8")
    );
    harness.stop().await;
}
