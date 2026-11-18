//! A viewer: the feed shows what an agent did, the profile of the
//! resident has its life story, and the published statistics and
//! leaderboards answer after the game published them.

mod common;

use common::agent::Agent;
use common::game::Catalogue;
use common::link::{Link, accepted, myself, state};
use common::story::first_day;
use common::{World, code, id, int, soon};
use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

/// Frames of one game day.
const DAY: u64 = 1_440;

type Feed =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads the next message of the viewer socket.
async fn next(feed: &mut Feed) -> Value {
    loop {
        let message = feed.next().await.expect("the feed is open").expect("a message");
        if let Message::Text(text) = message {
            return serde_json::from_str(&text).expect("a feed message is JSON");
        }
    }
}

/// Reads messages of the viewer socket until one has the tag `op`.
async fn next_of(feed: &mut Feed, op: &str) -> Value {
    loop {
        let message = next(feed).await;
        if message["op"] == op {
            return message;
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_viewer_sees_the_day_of_an_agent_in_the_feed_the_profile_and_the_published_state() {
    let world = World::start().await;
    let catalogue = Catalogue::read(&world).await;
    let mut ada = Agent::join(&world, "Ada").await;
    let bo = Agent::join(&world, "Bo").await;

    // Nothing is published before the game published it.
    for path in
        ["/v1/stats/economy", "/v1/leaderboards/wealth", "/v1/published/finance.stats_published"]
    {
        let (status, body) = world.read(path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
        assert_eq!(code(&body), "not_published");
    }

    // The agent lives its first day and writes a will.
    let day = first_day(&world, &mut ada).await;
    let ack = ada.act("write_will", json!({ "heir": bo.id })).await;
    assert_eq!(state(&ack), "done", "{ack}");
    let cash = int(&myself(&mut ada).await["cash"], "cash");
    assert_eq!(cash, day.cash);

    // The feed has what the agent did, oldest first, with a cursor.
    world.step(2).await;
    let mine = |event: &&Value| event["actor"] == ada.id;
    let feed = world.public("/v1/feed?limit=500").await;
    let events: Vec<&Value> =
        feed["events"].as_array().expect("events").iter().filter(mine).collect();
    let kinds: Vec<&str> =
        events.iter().map(|event| event["kind"].as_str().expect("a kind")).collect();
    let position = |kind: &str| {
        kinds
            .iter()
            .position(|seen| *seen == kind)
            .unwrap_or_else(|| panic!("`{kind}` in {kinds:?}"))
    };
    let order = [
        position("kernel.spawned"),
        position("economy.shift_completed"),
        position("economy.goods_sold"),
        position("life.meal_eaten"),
        position("life.slept"),
        position("life.will_written"),
    ];
    assert!(order.is_sorted(), "the feed has the day in its order: {kinds:?}");
    let shift = events[position("economy.shift_completed")];
    assert_eq!(shift["actor_name"], "Ada");
    assert_eq!(int(&shift["data"]["wage"], "wage"), day.wage, "{shift}");
    let sale = events[position("economy.goods_sold")];
    assert_eq!(sale["data"]["item"], day.food.as_str(), "{sale}");
    assert_eq!(int(&sale["data"]["unit_price"], "unit_price"), day.price);
    assert_eq!(sale["data"]["buyer"], ada.id);

    // The cursor: the events after the first page, with no event twice.
    let page = world.public("/v1/feed?limit=3&after=0-0").await;
    let first: Vec<&str> = page["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["id"].as_str().expect("id"))
        .collect();
    assert_eq!(first.len(), 3);
    assert_eq!(page["more"], true);
    let rest = world
        .public(&format!("/v1/feed?limit=500&after={}", page["next"].as_str().expect("next")))
        .await;
    let rest: Vec<&str> = rest["events"]
        .as_array()
        .expect("events")
        .iter()
        .map(|e| e["id"].as_str().expect("id"))
        .collect();
    assert!(!rest.is_empty() && !rest.iter().any(|id| first.contains(id)), "{first:?} {rest:?}");
    let only = world.public("/v1/feed?kind=life.will_written&min_drama=11").await;
    assert_eq!(only["events"].as_array().map(Vec::len), Some(1), "{only}");

    // The profile has the resident and its life story.
    let profile = world.public(&format!("/v1/residents/{}/profile", ada.id)).await;
    assert_eq!(profile["name"], "Ada");
    assert_eq!(profile["model"], "e2e-test");
    assert_eq!(profile["alive"], true);
    assert_eq!(profile["pos"], myself(&mut ada).await["pos"]);
    let recent = profile["events"].as_array().expect("recent events");
    assert_eq!(recent.last().expect("an event")["kind"], "life.will_written", "{profile}");
    let life = &profile["life"];
    assert_eq!(life["born"]["kind"], "kernel.spawned", "{life}");
    assert!(
        life["born"]["line"].as_str().is_some_and(|line| line.starts_with("Ada arrived in ")),
        "{life}"
    );
    let deals = life["deals"].as_array().expect("the chapter of deals");
    assert_eq!(deals.len(), 1, "{life}");
    assert_eq!(deals[0]["kind"], "life.will_written");
    assert!(deals[0]["line"].as_str().is_some_and(|line| line.contains("Ada")), "{life}");
    assert!(life.get("death").is_none(), "the resident lives: {life}");
    let (status, nobody) = world.read("/v1/residents/999999/profile").await;
    assert_eq!((status, code(&nobody)), (StatusCode::NOT_FOUND, "unknown_resident"));

    // The leaderboards: the game publishes them at an interval.
    world.step(u32::try_from(catalogue.param("missions.board_period")).expect("a period")).await;
    let board = world.public("/v1/leaderboards/wealth").await;
    assert_eq!(board["kind"], "missions.leaderboard_published");
    assert_eq!(board["key"], "wealth");
    let top = board["data"]["top"].as_array().expect("the best residents");
    assert_eq!(top[0]["resident"], ada.id, "Ada has money, Bo has none: {board}");
    assert_eq!(int(&top[0]["score"], "score"), cash, "the wealth is the cash: {board}");
    assert_eq!(top[0]["model"], "e2e-test");
    let (status, none) = world.read("/v1/leaderboards/beauty").await;
    assert_eq!((status, code(&none)), (StatusCode::NOT_FOUND, "not_published"));

    // The statistics of the economy: the game publishes them at midnight.
    let frame = id(&world.public("/v1/spectate/stats").await["frame"], "frame");
    assert!(frame < DAY, "the first day is not over at frame {frame}");
    world.step(u32::try_from(DAY - frame + 2).expect("frames")).await;
    let stats = world.public("/v1/stats/economy").await;
    assert_eq!(stats["kind"], "finance.stats_published");
    let series = stats["series"].as_array().expect("series");
    assert_eq!(series.len(), 1, "{stats}");
    assert_eq!(id(&series[0]["frame"], "frame"), DAY);
    let numbers = &series[0]["data"];
    assert_eq!(numbers["day"], 1, "{numbers}");
    assert_eq!(
        int(&numbers["faucets"]["gov.public_works"]["total"], "the wages of starter shifts"),
        day.wage * i64::from(day.shifts),
        "the money that came into the world is the wages of the shifts: {numbers}"
    );
    assert_eq!(
        int(&numbers["sinks"]["import.purchase"]["total"], "imports"),
        day.price,
        "{numbers}"
    );
    assert_eq!(
        int(&numbers["money_supply"], "money supply"),
        day.wage * i64::from(day.shifts) - day.price,
        "{numbers}"
    );
    assert_eq!(int(&numbers["residents"], "residents"), 2, "{numbers}");

    // The list of everything that is published.
    let index = world.public("/v1/published").await;
    let entries = index["published"].as_array().expect("published");
    for (kind, key) in [
        ("finance.stats_published", Value::Null),
        ("missions.leaderboard_published", json!("wealth")),
    ] {
        let found = entries
            .iter()
            .any(|entry| entry["kind"] == kind && entry.get("key").unwrap_or(&Value::Null) == &key);
        assert!(found, "{kind} {key} in {index}");
    }

    // The show: the director and the stories answer.
    let director = world.public("/v1/director").await;
    assert!(director["shot"]["seq"].is_u64(), "{director}");
    assert!(world.public("/v1/stories").await["stories"].is_array());
    world.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_viewer_socket_shows_a_resident_that_walks() {
    let world = World::start().await;
    let mut ada = Agent::join(&world, "Ada").await;
    world.step(1).await;
    let me = myself(&mut ada).await;
    let zone = id(&me["zone"], "zone");

    let address = format!("ws://{}/v1/spectate", world.addr);
    let (mut feed, _) =
        tokio_tungstenite::connect_async(address).await.expect("the viewer connects");
    let hello = soon(next(&mut feed)).await;
    assert_eq!(hello["op"], "world", "the first message is the world: {hello}");
    assert_eq!(hello["protocol"], 1);
    let subscribe = json!({ "op": "subscribe", "zones": [zone] });
    feed.send(Message::Text(subscribe.to_string().into())).await.expect("subscribe");
    let snapshot = soon(next_of(&mut feed, "snapshot")).await;
    assert_eq!(snapshot["zone"], zone);
    let residents = snapshot["residents"].as_array().expect("residents");
    assert_eq!(residents.len(), 1, "{snapshot}");
    assert_eq!(residents[0]["id"], ada.id);
    assert_eq!(residents[0]["name"], "Ada");
    assert_eq!(residents[0]["model"], "e2e-test");
    assert_eq!(residents[0]["pos"], me["pos"]);

    // The agent rests. The viewer sees the activity, and that it ends.
    let ack = accepted(&mut ada, "rest", json!({ "ticks": 3 })).await;
    assert_eq!(state(&ack), "started");
    let mut activity = Vec::new();
    while activity.len() < 2 {
        let delta = world.until(next_of(&mut feed, "delta")).await;
        assert_eq!(delta["zone"], zone);
        for change in delta["activity"].as_array().into_iter().flatten() {
            assert_eq!(change[0], ada.id);
            activity.push(change[1].clone());
        }
    }
    assert_eq!(activity, [json!("rest"), Value::Null]);
    drop(feed);
    world.stop().await;
}
