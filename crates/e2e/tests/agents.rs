//! The reference agents against the real server, as child processes.
//!
//! The tests are ignored by default: they need `python3`, `node` and the
//! built JavaScript packages, and they run for some game days.
//!
//! ```sh
//! cargo test -p unsup-e2e -- --include-ignored
//! ```

mod common;

use std::time::Duration;

use common::child::{run, start};
use common::{World, config, id, int, no_rate_limits, repo_root, soon};
use futures::StreamExt;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

/// Real time of one game minute in these tests.
const TICK: Duration = Duration::from_millis(4);
/// Number of house residents of the test.
const RESIDENTS: u64 = 6;
/// Game days that the house residents live.
const DAYS: u64 = 3;
const DAY: u64 = 1_440;

/// Starts a world that runs by itself, fast, with room for many residents
/// of one owner.
async fn fast_world() -> World {
    let mut config = config();
    no_rate_limits(&mut config);
    config.tick_interval = TICK;
    config.gateway.limits.residents_per_owner = 0;
    World::with(config).await
}

/// Watches the world as a viewer does, until the world reaches a frame.
/// The feed of the viewers has one `frame` message at the end of each
/// frame. `alive` is called for each frame: it stops the test when the
/// process under test is gone.
async fn watch_until(world: &World, frame: u64, mut alive: impl FnMut()) {
    let address = format!("ws://{}/v1/spectate", world.addr);
    let (mut feed, _) =
        tokio_tungstenite::connect_async(address).await.expect("the viewer connects");
    loop {
        let message = feed.next().await.expect("the feed is open").expect("a message");
        let Message::Text(text) = message else { continue };
        let message: Value = serde_json::from_str(&text).expect("a feed message is JSON");
        if message["op"] == "frame" {
            alive();
            if id(&message["frame"], "frame") >= frame {
                return;
            }
        }
    }
}

/// Returns the public events of one kind, oldest first.
async fn events(world: &World, kind: &str) -> Vec<Value> {
    let feed = world.public(&format!("/v1/feed?limit=500&kind={kind}")).await;
    feed["events"].as_array().expect("events").clone()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs python3 and runs three game days: run with --include-ignored"]
async fn house_residents_stay_alive_earn_and_eat_for_days() {
    let world = fast_world().await;
    let (_, owner_token) = world.owner("Operator").await;
    let dir = repo_root().join("agents/scripted");
    let state = std::env::temp_dir().join(format!("unsup-e2e-house-{}.json", std::process::id()));
    let settings =
        std::env::temp_dir().join(format!("unsup-e2e-house-{}.config.json", std::process::id()));
    let seconds = TICK.as_secs_f64();
    let settings_text = json!({ "min_decision_seconds": 0.0, "error_seconds": 0.2 }).to_string();
    std::fs::write(&settings, settings_text).expect("the configuration is written");

    // The command of the README for house residents: one owner token, N
    // residents.
    let args = [
        "-m".to_string(),
        "scripted_agent".into(),
        "--url".into(),
        world.base.clone(),
        "--residents".into(),
        RESIDENTS.to_string(),
        "--model".into(),
        "house".into(),
        "--state".into(),
        state.display().to_string(),
        "--config".into(),
        settings.display().to_string(),
        "--tick-seconds".into(),
        seconds.to_string(),
    ];
    let mut house =
        start("python3", &args, &dir, &[("UNSUPERVISED_OWNER_TOKEN", owner_token.clone())]);

    // The world runs by itself. The test watches it until the game days are over.
    soon(watch_until(&world, DAYS * DAY, || {
        assert!(house.is_running(), "the house residents run")
    }))
    .await;
    let finished = house.stop().await;
    let _ = std::fs::remove_file(&state);
    let _ = std::fs::remove_file(&settings);
    assert!(finished.stdout.starts_with(&format!("running {RESIDENTS} resident(s)")), "{finished}");

    // Every resident is alive, and the owner has all of them.
    let (_, listed) = world.get("/v1/residents", &owner_token).await;
    let residents: Vec<u64> = listed["residents"]
        .as_array()
        .expect("residents")
        .iter()
        .map(|resident| id(&resident["id"], "id"))
        .collect();
    assert_eq!(residents.len() as u64, RESIDENTS, "nobody died: {listed}\n{finished}");
    assert!(events(&world, "life.resident_died").await.is_empty(), "{finished}");

    // Each one earned, ate, slept at home, and has money now.
    let (shifts, meals, sleeps) = (
        events(&world, "economy.shift_completed").await,
        events(&world, "life.meal_eaten").await,
        events(&world, "life.slept").await,
    );
    let count =
        |events: &[Value], resident: u64| events.iter().filter(|e| e["actor"] == resident).count();
    for &resident in &residents {
        let profile = world.public(&format!("/v1/residents/{resident}/profile")).await;
        assert_eq!(profile["alive"], true, "{profile}");
        assert_eq!(profile["model"], "house");
        assert!(
            count(&shifts, resident) as u64 >= DAYS,
            "resident {resident} worked each day\n{finished}"
        );
        assert!(
            count(&meals, resident) as u64 >= DAYS,
            "resident {resident} ate each day\n{finished}"
        );
        assert!(
            count(&sleeps, resident) as u64 >= DAYS - 1,
            "resident {resident} slept each night\n{finished}"
        );
    }
    let good_sleep = sleeps.iter().filter(|event| event["data"]["quality"] == "fair").count();
    let nights: Vec<String> = sleeps
        .iter()
        .map(|event| {
            format!(
                "{} {} {}t day {} {:02}h",
                event["actor"],
                event["data"]["quality"],
                event["data"]["ticks"],
                event["clock"]["day"],
                event["clock"]["hour"]
            )
        })
        .collect();
    assert!(
        good_sleep * 10 >= sleeps.len() * 9,
        "the residents sleep in the shelter: {good_sleep} of {}: {nights:#?}\n{finished}",
        sleeps.len()
    );
    let board = world.public("/v1/leaderboards/wealth").await;
    let top = board["data"]["top"].as_array().expect("the board");
    for &resident in &residents {
        let entry = top.iter().find(|entry| entry["resident"] == resident).expect("on the board");
        assert!(int(&entry["score"], "score") > 0, "resident {resident} has money: {board}");
    }
    // They met and talked.
    assert!(!events(&world, "society.said").await.is_empty(), "the residents talk\n{finished}");
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs node and the built agents/llm: run with --include-ignored after `npm ci && npm run build`"]
async fn the_llm_agent_lives_in_the_real_world_with_a_fake_model() {
    let mut config = config();
    no_rate_limits(&mut config);
    let world = World::with(config).await;
    let clock = world.clock().await;
    let dir = repo_root().join("agents/llm");
    let env = [("UNSUPERVISED_URL", world.base.clone()), ("E2E_CLOCK_URL", clock.url.clone())];
    let finished = run("node", ["e2e/real-server.mjs"], &dir, &env, Duration::from_secs(240)).await;
    assert!(finished.success, "`node e2e/real-server.mjs` works:\n{finished}");
    let facts = finished.fact("llm");
    let resident = id(&facts["resident"], "resident");

    // The server agrees with what the fake model saw: the resident is in
    // the world with the name of the environment and the name of the model.
    world.step(1).await;
    let profile = world.public(&format!("/v1/residents/{resident}/profile")).await;
    assert_eq!(profile["name"], "Lia");
    assert_eq!(profile["model"], "fake-model");
    assert_eq!(profile["alive"], true);
    let of_resident = |events: Vec<Value>| {
        events.into_iter().filter(|event| event["actor"] == resident).count() as u64
    };
    let shifts = of_resident(events(&world, "economy.shift_completed").await);
    assert_eq!(shifts, id(&facts["shifts"], "shifts"), "{finished}");
    assert!(shifts >= 2);
    assert_eq!(of_resident(events(&world, "life.meal_eaten").await), 1);
    // It spoke one time: the server refused its second text, the link.
    let said = events(&world, "society.said").await;
    let texts: Vec<&Value> = said
        .iter()
        .filter(|event| event["actor"] == resident)
        .map(|event| &event["data"]["text"]["text"])
        .collect();
    assert_eq!(texts, [&json!("Good day. I am new here.")], "{said:?}");
    // The stranger got nothing for its message.
    let stranger = id(&facts["stranger"], "stranger");
    let (_, stranger_profile) = world.read(&format!("/v1/residents/{stranger}/profile")).await;
    assert_eq!(stranger_profile["name"], "Mallory");
    assert!(int(&facts["cash"], "cash") > 0, "{finished}");
    world.stop().await;
}
