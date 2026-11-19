//! The reference agents against the real server, as child processes.
//!
//! The tests are ignored by default: they need `python3`, `node` and the
//! built JavaScript packages, and they run for some game days.
//!
//! ```sh
//! cargo test -p unsup-e2e -- --include-ignored
//! ```

mod common;

use std::time::{Duration, Instant};

use common::child::{run, start};
use common::{PATIENCE, World, config, id, int, no_rate_limits, repo_root};
use serde_json::{Value, json};

/// Real time of one game minute in the test of the house residents, while
/// every one of them waits.
const TICK: Duration = Duration::from_millis(4);
/// Number of house residents of the test.
const RESIDENTS: u64 = 6;
/// Game days that the house residents live.
const DAYS: u64 = 3;
const DAY: u64 = 1_440;
/// Real time after which a house resident that does not come back to its
/// long poll fails the test.
const STUCK: Duration = Duration::from_secs(60);

/// Starts a world that moves only when the test steps it, with room for
/// many residents of one owner.
async fn stepped_world() -> World {
    let mut config = config();
    no_rate_limits(&mut config);
    config.gateway.limits.residents_per_owner = 0;
    World::with(config).await
}

/// Runs the world up to a frame, as a clock that stops while a client
/// thinks.
///
/// One frame runs for each [`TICK`] of real time, and only while each of
/// the `clients` blocks in its long poll: `GET /v1/wait`. A client that
/// got news decides and plans with the world at a stand. So a slow
/// machine makes the test longer, and it does not take game time from a
/// client. The clients still wait in real time: `TICK` is the speed that
/// they are told. `alive` is called before each frame: it stops the test
/// when the process under test is gone.
///
/// # Panics
///
/// Panics when the clients do not all wait again after [`STUCK`].
async fn run_while_they_wait(world: &World, clients: u64, frame: u64, mut alive: impl FnMut()) {
    let mut pace = tokio::time::interval(TICK);
    pace.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut moved = Instant::now();
    loop {
        pace.tick().await;
        alive();
        let waiting = world.open_waits().await;
        if waiting < clients {
            assert!(
                moved.elapsed() < STUCK,
                "{waiting} of {clients} clients wait after {STUCK:?}: one does not come back to its long poll"
            );
            continue;
        }
        moved = Instant::now();
        if world.step(1).await >= frame {
            return;
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
    let world = stepped_world().await;
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

    // The game days pass while the house residents wait for news.
    run_while_they_wait(&world, RESIDENTS, DAYS * DAY, || {
        assert!(house.is_running(), "the house residents run");
    })
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
    let finished = run("node", ["e2e/real-server.mjs"], &dir, &env, PATIENCE).await;
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
    // The world ran only while the agent waited: one frame for each
    // second of a wait. So the frame of the server is the time that the
    // script counted, and a run is the same each time.
    let frame = id(&world.public("/v1/world").await["frame"], "frame");
    assert_eq!(frame, id(&facts["frames"], "frames") + 1, "{finished}");
    // The default routine of the game put the tired resident to sleep at
    // its workplace. The agent woke it and sent it to its bed: the one
    // sleep that ended by itself was in the shelter.
    let nights: Vec<Value> = events(&world, "life.slept")
        .await
        .into_iter()
        .filter(|event| event["actor"] == resident)
        .map(|event| event["data"]["quality"].clone())
        .collect();
    assert_eq!(nights, ["fair"], "{finished}");
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
