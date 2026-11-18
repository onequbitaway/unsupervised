//! Missions of human sponsors over real HTTP: an owner publishes a
//! mission, a resident reads it, and the limits and the checks of the
//! route refuse what they must.

mod common;

use std::time::Duration;

use common::{ADMIN, Client, bread_mission, config, send};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use unsup_gateway::AdminToken;
use unsup_server::{Server, ServerConfig};

/// The configuration of these tests: the test world, an admin token, and
/// an audit of the whole world after every frame.
fn sponsors(per_day: u32) -> ServerConfig {
    let mut config = config();
    config.gateway.admin_token = Some(AdminToken::new(ADMIN).expect("a long token"));
    config.durability.audit_every = 1;
    config.sponsors.missions_per_day = per_day;
    config
}

/// Returns the code and the message of an error answer.
fn error(answer: &Value) -> (&str, &str) {
    let text = |field: &str| answer["error"][field].as_str().unwrap_or_else(|| panic!("{answer}"));
    (text("code"), text("message"))
}

/// Returns the money that the last audit of the whole world counted as
/// created, in cents, after a check that no audit failed.
async fn minted(client: &Client) -> i64 {
    let audit = client.dashboard().await["host"]["audit"].clone();
    assert_eq!(audit["failures"], 0, "{audit}");
    audit["money"]["minted"].as_i64().unwrap_or_else(|| panic!("an audit with money: {audit}"))
}

/// Returns the number of open missions that a resident can take.
async fn open_missions(server: &Server, client: &Client, key: &str) -> usize {
    let listed = client.mission_list(server, key, json!({})).await;
    listed["missions"].as_array().expect("missions").len()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owner_publishes_a_mission_and_a_resident_reads_it_and_the_limit_holds() {
    let server = unsup_server::start(sponsors(1)).await.expect("the server starts");
    let client = Client::of(&server);
    let ada = client.sign_up("Ada Mills").await;
    let (_, resident) = client.resident(&ada, "Baker").await;
    server.sim.step(1).await.expect("the world steps");
    assert_eq!(open_missions(&server, &client, &resident).await, 0);
    let before = minted(&client).await;

    // The owner publishes a mission. The world answers in the next frame.
    let (status, answer) = client.sponsor(&server, &ada, &bread_mission()).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    let mission = answer["mission"].as_u64().expect("the id of the mission");
    let frame = answer["frame"].as_u64().expect("the frame of the publication");
    assert_eq!(
        answer,
        json!({
            "mission": mission, "request": answer["request"], "frame": frame,
            "deadline": frame + 600, "reward": 5000, "takers": 2,
        })
    );
    assert!(answer["request"].as_u64().is_some_and(|request| request < 1 << 53), "{answer}");
    // The reward is new money: two places of 5,000 cents. The books of
    // the world balance.
    assert_eq!(minted(&client).await, before + 10_000);

    // A resident lists it with `mission_list`.
    let listed = client.mission_list(&server, &resident, json!({ "sponsored": true })).await;
    assert_eq!(
        listed,
        json!({ "missions": [{
            "mission": mission, "reward": 5000, "mode": "solo", "deadline": frame + 600,
            "taken": 0, "takers": 2, "sponsored": true, "objectives": 1,
            "title": { "untrusted": true, "from": 0, "text": "Bread for the fair" },
        }] })
    );
    // The sponsor is the owner of the token: the client did not name it.
    let one = client.mission_list(&server, &resident, json!({ "mission": mission })).await;
    assert_eq!(
        one["mission"]["sponsor"],
        json!({ "untrusted": true, "from": 0, "text": "Ada Mills" })
    );
    assert_eq!(
        one["mission"]["text"],
        json!({ "untrusted": true, "from": 0, "text": "The town needs bread." })
    );
    // The resident can take it.
    assert_eq!(
        client.accepted(&resident, "mission_accept", json!({ "mission": mission })).await,
        "pending"
    );
    server.sim.step(2).await.expect("the world steps");
    let mine = client.mission_list(&server, &resident, json!({ "scope": "accepted" })).await;
    assert_eq!(mine["missions"][0]["mission"], mission);

    // The limit of one owner: one mission in 24 hours. The next one is
    // refused, and nothing reaches the world.
    let request = client.post("/v1/missions", &bread_mission()).bearer_auth(&ada);
    let response = request.send().await.expect("the server answers");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let wait: u64 = response
        .headers()
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .expect("the seconds to wait");
    assert!((86_000..=86_400).contains(&wait), "{wait}");
    let refused: Value = response.json().await.expect("a JSON body");
    assert_eq!(
        error(&refused),
        (
            "rate_limited",
            format!(
                "An owner can publish 1 sponsor missions in 24 hours. Try again in {wait} \
                 seconds."
            )
            .as_str()
        )
    );
    server.sim.step(2).await.expect("the world steps");
    assert_eq!(minted(&client).await, before + 10_000, "no second reward was created");

    // The limit counts for each owner: another owner publishes one.
    let bob = client.sign_up("Bob").await;
    let (status, answer) = client.sponsor(&server, &bob, &bread_mission()).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_ne!(answer["mission"], mission);
    assert_eq!(minted(&client).await, before + 20_000);
    // Viewers see the publications as public events.
    let feed = common::viewer_shows(&client, "/v1/feed?kind=missions.mission_posted", |feed| {
        feed["events"].as_array().is_some_and(|events| events.len() == 2)
    })
    .await;
    let sponsors: Vec<&Value> = feed["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|event| &event["data"]["sponsor"]["text"])
        .collect();
    assert_eq!(sponsors, ["Ada Mills", "Bob"]);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_route_refuses_a_bad_request_before_it_reaches_the_world() {
    let server = unsup_server::start(sponsors(1)).await.expect("the server starts");
    let client = Client::of(&server);
    let ada = client.sign_up("Ada").await;
    let (_, resident) = client.resident(&ada, "Baker").await;
    server.sim.step(1).await.expect("the world steps");
    let before = minted(&client).await;
    let inputs = server.sim.stats().await.expect("stats").journal_records;
    let with = |change: fn(&mut Value)| {
        let mut mission = bread_mission();
        change(&mut mission);
        mission
    };

    let invalid: Vec<(Value, &str)> = vec![
        (json!([1, 2]), "The body must be an object: the definition of a mission."),
        (
            json!(["Bread", "", [{ "event": "economy.goods_produced" }], "solo", null, 500, 60, 1]),
            "The body must be an object: the definition of a mission.",
        ),
        (
            with(|m| m["sponsor"] = json!("Acme")),
            "The body is not a mission: unknown field `sponsor`, expected one of `title`, \
             `text`, `objectives`, `mode`, `org`, `reward`, `deadline_in`, `takers`.",
        ),
        (
            with(|m| {
                m.as_object_mut().expect("an object").remove("reward");
            }),
            "The body is not a mission: missing field `reward`.",
        ),
        (with(|m| m["title"] = json!("  ")), "`title` must not be empty."),
        (with(|m| m["title"] = json!("t".repeat(81))), "`title` has at most 80 bytes."),
        (with(|m| m["text"] = json!("word ".repeat(201))), "`text` has at most 1000 bytes."),
        (with(|m| m["objectives"] = json!([])), "`objectives` has from 1 to 4 objectives."),
        (
            with(|m| {
                let one = m["objectives"][0].clone();
                m["objectives"] = json!([one, one, one, one, one]);
            }),
            "`objectives` has from 1 to 4 objectives.",
        ),
        (
            with(|m| m["objectives"][0]["event"] = json!(format!("economy.{}", "e".repeat(60)))),
            "`objectives[0]`: `event` has at most 64 bytes.",
        ),
        (
            with(|m| {
                m["objectives"][0]["fields"] = json!({ "a": 1, "b": 2, "c": 3, "d": 4, "e": 5 })
            }),
            "`objectives[0]`: `fields` has at most 4 data fields.",
        ),
        (
            with(|m| m["objectives"][0]["fields"] = json!({ "n".repeat(33): 1 })),
            "`objectives[0]`: the name of a data field has at most 32 bytes.",
        ),
        (
            with(|m| m["objectives"][0]["fields"] = json!({ "item": "v".repeat(65) })),
            "`objectives[0]`: a text value of a data field has at most 64 bytes.",
        ),
        // 5,000 cents for 101 takers: more than the 500,000 cents that
        // one mission can create on this server.
        (
            with(|m| m["takers"] = json!(101)),
            "`reward` times `takers` is more than 500000 cents. That is the greatest sum for \
             one sponsor mission on this server.",
        ),
    ];
    for (body, message) in invalid {
        let (status, answer) = send(client.post("/v1/missions", &body).bearer_auth(&ada)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(error(&answer), ("invalid_request", message), "{body}");
    }

    // The text filters of the server: a link in a text, also deep in the
    // definition.
    let filtered = [
        (with(|m| m["title"] = json!("Bread from www.example.com")), "`title`"),
        (with(|m| m["text"] = json!("See https://example.org/bread")), "`text`"),
        (
            with(|m| m["objectives"][0]["fields"] = json!({ "item": "bread.example.com" })),
            "`objectives[0].fields.item`",
        ),
    ];
    for (body, place) in filtered {
        let (status, answer) = send(client.post("/v1/missions", &body).bearer_auth(&ada)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let message = format!("{place}: The text holds a link. This server takes no links.");
        assert_eq!(error(&answer), ("text_refused", message.as_str()));
    }

    // The door of the gateway: a resident key, no token, a query.
    let mission = bread_mission();
    let (status, answer) = send(client.post("/v1/missions", &mission).bearer_auth(&resident)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::FORBIDDEN, "wrong_token_kind"));
    let (status, answer) = send(client.post("/v1/missions", &mission)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::UNAUTHORIZED, "unauthorized"));
    let (status, answer) =
        send(client.post("/v1/missions?draft=true", &mission).bearer_auth(&ada)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::BAD_REQUEST, "invalid_request"));
    let (status, answer) = send(client.get("/v1/missions").bearer_auth(&ada)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed"));

    // None of these requests was an input of the world, and none took the
    // place of the owner in its limit: a good mission passes.
    assert_eq!(server.sim.stats().await.expect("stats").journal_records, inputs);
    let (status, answer) = client.sponsor(&server, &ada, &mission).await;
    assert_eq!(status, StatusCode::CREATED, "{answer}");
    assert_eq!(minted(&client).await, before + 10_000);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refusal_of_the_world_is_the_answer_and_takes_no_place_in_the_limit() {
    let mut config = sponsors(2);
    // The limit of the server for one mission is off: the limit of the
    // module for one game day decides.
    config.sponsors.max_mission_cents = 0;
    let server = unsup_server::start(config).await.expect("the server starts");
    let client = Client::of(&server);
    let ada = client.sign_up("Ada").await;
    let (_, resident) = client.resident(&ada, "Baker").await;
    server.sim.step(1).await.expect("the world steps");
    let before = minted(&client).await;
    let with = |change: fn(&mut Value)| {
        let mut mission = bread_mission();
        change(&mut mission);
        mission
    };

    // The module checks what the route does not know. Its refusal is the
    // answer: the code and the message are those of the module.
    let cases = [
        (
            with(|m| m["reward"] = json!(50)),
            StatusCode::BAD_REQUEST,
            "invalid_args",
            "`reward` must be from 100 to 10000000 cents.",
        ),
        (
            with(|m| m["objectives"][0]["event"] = json!("kernel.spawned")),
            StatusCode::BAD_REQUEST,
            "invalid_args",
            "`kernel.spawned` is not an event that a mission can count. Use a kind that starts \
             with one of: life., economy., finance., crime., politics., missions., society.org_, \
             society.contract_, society.married, society.divorced, society.household_, \
             society.opinion_.",
        ),
        // More than the module creates for all sponsors in one game day.
        (
            with(|m| m["reward"] = json!(2_600_000)),
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "The mission needs 5200000 cents. The sponsor faucet has 5000000 of 5000000 cents \
             left today.",
        ),
    ];
    for (body, status, code, message) in cases {
        let (got, answer) = client.sponsor(&server, &ada, &body).await;
        assert_eq!(got, status, "{answer}");
        assert_eq!(error(&answer), (code, message));
    }
    assert_eq!(open_missions(&server, &client, &resident).await, 0);
    assert_eq!(minted(&client).await, before, "a refusal creates no money");

    // The owner still has its two places: three refusals took none.
    let (status, first) = client.sponsor(&server, &ada, &bread_mission()).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    // The world has room for one open mission now: the next one is
    // refused by the module, with `conflict`.
    let body = json!({ "name": "missions.open_max", "value": 1 });
    let (status, _) = client.admin(Method::POST, "/admin/params", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, answer) = client.sponsor(&server, &ada, &bread_mission()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{answer}");
    assert_eq!(
        error(&answer),
        ("conflict", "The world has 1 open missions already. Try again later.")
    );
    let body = json!({ "name": "missions.open_max", "value": 2000 });
    let (status, _) = client.admin(Method::POST, "/admin/params", Some(body)).await;
    assert_eq!(status, StatusCode::OK);
    let (status, second) = client.sponsor(&server, &ada, &bread_mission()).await;
    assert_eq!(status, StatusCode::CREATED, "{second}");
    assert_ne!(first["request"], second["request"], "each request has its own number");
    // Two missions are published: the places of the owner are taken.
    let (status, answer) =
        send(client.post("/v1/missions", &bread_mission()).bearer_auth(&ada)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    assert_eq!(open_missions(&server, &client, &resident).await, 2);
    assert_eq!(minted(&client).await, before + 20_000);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_administrator_turns_sponsor_missions_off_in_the_configuration() {
    let mut config = sponsors(3);
    config.sponsors.enabled = false;
    let server = unsup_server::start(config).await.expect("the server starts");
    let client = Client::of(&server);
    let ada = client.sign_up("Ada").await;
    let (_, resident) = client.resident(&ada, "Baker").await;
    server.sim.step(1).await.expect("the world steps");
    let before = minted(&client).await;

    let (status, answer) =
        send(client.post("/v1/missions", &bread_mission()).bearer_auth(&ada)).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(
        error(&answer),
        ("missions_disabled", "The administrator of this server turned the sponsor missions off.")
    );
    // The gateway still asks for an owner token first.
    let (status, answer) =
        send(client.post("/v1/missions", &bread_mission()).bearer_auth(&resident)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::FORBIDDEN, "wrong_token_kind"));
    // Nothing reached the world. The missions of residents still work.
    server.sim.step(2).await.expect("the world steps");
    assert_eq!(open_missions(&server, &client, &resident).await, 0);
    assert_eq!(minted(&client).await, before);
    server.stop().await.expect("the server stops");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_world_that_does_not_answer_in_time_keeps_the_mission() {
    let mut config = sponsors(1);
    config.sponsors.answer_timeout_ms = 50;
    let server = unsup_server::start(config).await.expect("the server starts");
    let client = Client::of(&server);
    let ada = client.sign_up("Ada").await;
    let (_, resident) = client.resident(&ada, "Baker").await;
    server.sim.step(1).await.expect("the world steps");
    assert_eq!(open_missions(&server, &client, &resident).await, 0);

    // The world of the tests runs no frame by itself, so no answer comes.
    let started = std::time::Instant::now();
    let (status, answer) =
        send(client.post("/v1/missions", &bread_mission()).bearer_auth(&ada)).await;
    assert!(started.elapsed() >= Duration::from_millis(50), "the route waited for its time");
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{answer}");
    let (code, message) = error(&answer);
    assert_eq!(code, "answer_timeout");
    let request: u64 = message
        .rsplit_once("with the request ")
        .and_then(|(_, rest)| rest.strip_suffix('.'))
        .and_then(|number| number.parse().ok())
        .unwrap_or_else(|| panic!("the message names the request: {message}"));

    // The world has the mission: the next frame publishes it, with the
    // request number of the answer. So its place in the limit stays.
    server.sim.step(2).await.expect("the world steps");
    assert_eq!(open_missions(&server, &client, &resident).await, 1);
    let feed = common::viewer_shows(&client, "/v1/feed?kind=missions.mission_posted", |feed| {
        feed["events"].as_array().is_some_and(|events| events.len() == 1)
    })
    .await;
    assert_eq!(feed["events"][0]["data"]["request"], request);
    let (status, answer) =
        send(client.post("/v1/missions", &bread_mission()).bearer_auth(&ada)).await;
    assert_eq!((status, error(&answer).0), (StatusCode::TOO_MANY_REQUESTS, "rate_limited"));
    server.stop().await.expect("the server stops");
}
