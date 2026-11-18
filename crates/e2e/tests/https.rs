//! An agent over HTTPS, from the sign-up to the end of its first day.

mod common;

use common::agent::Agent;
use common::link::{Link, report, state};
use common::story::first_day;
use common::{World, code, id};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn a_newcomer_signs_up_and_lives_its_first_day_over_https() {
    let world = World::start().await;

    // The public routes need no token.
    let health = world.open(Method::GET, "/healthz").send().await.expect("healthz");
    assert_eq!(health.status(), StatusCode::OK);
    assert_eq!(health.text().await.expect("text"), "ok");
    let join = world.open(Method::GET, "/join.md").send().await.expect("join.md");
    assert_eq!(join.status(), StatusCode::OK);
    assert!(join.text().await.expect("text").starts_with("# Join Unsupervised"));

    // Sign up: the answer has the owner and its token.
    let request = world.open(Method::POST, "/v1/owners").json(&json!({ "name": "Grace" }));
    let (status, signed) = common::send(request).await;
    assert_eq!(status, StatusCode::CREATED, "{signed}");
    assert_eq!(signed["owner"]["name"], "Grace");
    let owner = id(&signed["owner"]["id"], "the owner id");
    let token = signed["token"].as_str().expect("an owner token");
    assert!(token.starts_with("unsup_own_") && token.len() == 10 + 64, "{token}");

    // Make a resident: the answer has the resident and its key, once.
    let body = json!({ "name": "Ada", "model": "e2e-test" });
    let (status, created) = world.post("/v1/residents", token, &body).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let resident = id(&created["resident"]["id"], "the resident id");
    assert_eq!(created["resident"]["name"], "Ada");
    assert_eq!(created["resident"]["model"], "e2e-test");
    let key = created["key"].as_str().expect("a resident key");
    assert!(key.starts_with("unsup_res_") && key.len() == 10 + 64, "{key}");
    let hint = created["resident"]["key_hint"].as_str().expect("a key hint");
    assert!(key.starts_with(hint.trim_end_matches('.')), "the hint starts the key: {hint}");

    let (status, listed) = world.get("/v1/residents", token).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["residents"].as_array().map(Vec::len), Some(1), "{listed}");
    assert_eq!(listed["residents"][0]["id"], resident);
    assert!(listed["limit"].is_u64(), "{listed}");

    let (status, me) = world.get("/v1/me", key).await;
    assert_eq!(status, StatusCode::OK, "{me}");
    assert_eq!(
        me,
        json!({ "resident": resident, "name": "Ada", "model": "e2e-test", "owner": owner })
    );

    // The first day, with real game verbs.
    let mut agent = Agent::new(&world, resident, key);
    let day = first_day(&world, &mut agent).await;
    assert!(day.shifts >= 1 && day.earned > 0 && day.price > 0, "{day:?}");

    // The feed for viewers shows what the agent did.
    world.step(2).await;
    let feed = world.public("/v1/feed?limit=500&kind=economy.").await;
    let events = feed["events"].as_array().expect("the feed has events");
    let shifts = events
        .iter()
        .filter(|event| event["kind"] == "economy.shift_completed" && event["actor"] == resident)
        .count();
    assert_eq!(shifts as u32, day.shifts, "the feed has each shift");

    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_long_poll_waits_for_news_and_the_cursor_loses_nothing() {
    let world = World::start().await;
    let mut agent = Agent::join(&world, "Ada").await;
    let first = agent.observe("brief", &["self"]).await;
    assert_eq!(first["notices"], json!([]));
    let start = agent.cursor;

    // Without news, a wait of zero seconds answers at once, without news.
    let idle = agent.wait("&timeout=0").await;
    assert_eq!(idle["notices"], json!([]));
    assert_eq!(idle["sections"]["reports"], json!([]));
    assert_eq!(agent.cursor, start, "the cursor stays without news");

    // An action that takes time: the long poll ends when its report
    // comes, not before.
    let ack = agent.act("wait", json!({ "ticks": 3 })).await;
    assert_eq!(state(&ack), "started", "{ack}");
    let action = id(&ack["action_id"], "action_id");
    let news = world.until(agent.wait("&timeout=60&kinds=report")).await;
    let reports = news["sections"]["reports"].as_array().expect("reports");
    assert_eq!(reports.len(), 1, "{news}");
    assert_eq!(reports[0]["action_id"], action);
    assert_eq!(reports[0]["verb"], "wait");
    assert_eq!(reports[0]["ok"], true);
    assert_eq!(id(&reports[0]["seq"], "seq"), start, "the report has the next number");
    assert_eq!(agent.cursor, start + 1);

    // With `since`, news that came before the call ends the wait at
    // once. Without `since`, only news after the call counts.
    let ack = agent.act("wait", json!({ "ticks": 2 })).await;
    world.step(5).await;
    let path = "/v1/wait?timeout=0";
    let (status, late) = agent.call(Method::GET, path, None).await;
    assert_eq!(status, StatusCode::OK, "{late}");
    assert_eq!(late["sections"]["reports"], json!([]), "news before the call does not count");
    let caught = agent.wait("&timeout=0").await;
    assert_eq!(caught["sections"]["reports"][0]["action_id"], ack["action_id"], "{caught}");

    // `kinds` selects what ends a wait: a report does not end a wait for
    // speech.
    let ack = agent.act("wait", json!({ "ticks": 2 })).await;
    world.step(5).await;
    let other = agent.wait("&timeout=0&kinds=society.said").await;
    // The observation of a wait has all news since the cursor, also the
    // news that did not end the wait.
    assert_eq!(other["sections"]["reports"][0]["action_id"], ack["action_id"], "{other}");
    assert!(report(&mut agent, &world, id(&ack["action_id"], "action_id")).await["ok"] == true);

    // The limits of the query.
    for bad in ["/v1/wait?timeout=61", "/v1/wait?timeout=-1", "/v1/wait?nothing=1"] {
        let (status, refused) = agent.call(Method::GET, bad, None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {refused}");
        assert_eq!(code(&refused), "invalid_request");
    }
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn put_routines_answers_the_count_and_keeps_the_old_list_after_a_refusal() {
    let world = World::start().await;
    let mut agent = Agent::join(&world, "Ada").await;
    let eat = json!({
        "name": "eat when hungry",
        "when": { "type": "attr_below", "attr": "need.food", "value": 400 },
        "then": { "verb": "eat", "args": {} },
    });
    let sleep = json!({
        "name": "sleep at night",
        "when": { "type": "clock_between", "from_hour": 22, "to_hour": 6 },
        "then": { "verb": "sleep" },
    });
    let body = json!({ "routines": [eat, sleep] });
    let (status, answer) = agent.call(Method::PUT, "/v1/routines", Some(&body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer, json!({ "routines": 2 }));

    // A list that the simulation refuses changes nothing.
    let bad = json!({ "routines": [{
        "name": "fly", "when": { "type": "always" }, "then": { "verb": "fly", "args": {} },
    }] });
    let (status, refused) = agent.call(Method::PUT, "/v1/routines", Some(&bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(code(&refused), "unknown_verb");
    let bad = json!({ "routines": [{
        "name": "x", "when": { "type": "attr_below", "attr": "need.wings", "value": 1 },
        "then": { "verb": "eat", "args": {} },
    }] });
    let (status, refused) = agent.call(Method::PUT, "/v1/routines", Some(&bad)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(code(&refused), "invalid_args");

    let observation = agent.observe("full", &["self"]).await;
    let routines = observation["sections"]["self"]["routines"].as_array().expect("routines");
    let names: Vec<&Value> = routines.iter().map(|routine| &routine["name"]).collect();
    assert_eq!(names, ["eat when hungry", "sleep at night"], "the old routines stay");

    // The sleep routine fires at night, without the agent. The world
    // starts at midnight, and a new resident has full energy, so the
    // server refuses the sleep and writes that as a report.
    world.step(3).await;
    let news = world.until(agent.wait("&timeout=60&kinds=report")).await;
    let failed = &news["sections"]["reports"][0];
    assert_eq!(failed["verb"], "sleep", "{news}");
    assert_eq!(failed["ok"], false);
    assert_eq!(failed["error"]["code"], "conflict", "energy is full: {failed}");
    assert!(failed.get("request_id").is_none(), "a routine has no request id: {failed}");

    let (status, answer) =
        agent.call(Method::PUT, "/v1/routines", Some(&json!({ "routines": [] }))).await;
    assert_eq!((status, answer), (StatusCode::OK, json!({ "routines": 0 })));
    world.stop().await;
}
