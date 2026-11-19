//! The delay of the stream, for every route and every message of the feed.
//!
//! The rule: with a delay of N frames, when the simulation is at frame F,
//! nothing that a viewer gets is newer than frame F - N.
//!
//! The test gives everything that changes a mark: the number of the frame
//! that reports the change. A resident goes to a tile of that number, a
//! shop gets the number as its name, an event and the published state
//! carry it. The test then reads every route and every message at each
//! frame, also between an action and the frame that reports it, and looks
//! for a mark that is too new.

mod common;

use std::collections::BTreeMap;

use common::*;
use serde_json::{Value, json};
use unsup_protocol::{Pos, ResidentId};
use unsup_runtime::RuntimeConfig;
use unsup_spectator::Config;
use unsup_spectator::wire::{Channel, ServerMessage};

/// Frames that the stream is behind the simulation.
const DELAY: u64 = 3;

/// The limits of this test: a delay, a map that is read again as often as
/// a request asks, and a director that moves fast.
fn delayed() -> Config {
    let mut config = limits();
    config.stream.delay_frames = DELAY;
    config.map.refresh_frames = 1;
    config.director.wide_min_frames = 1;
    config.director.min_hold_frames = 2;
    config.director.breaking_hold_frames = 1;
    config
}

/// The tile of a mark. Each mark below 96 has a tile of its own in zone 0,
/// in the rows below the plaza, where no new resident appears.
fn tile(mark: u64) -> Pos {
    assert!(mark < 96, "the test has tiles for 96 marks");
    let tile = Pos::new((mark % 16) as i32, 10 + (mark / 16) as i32);
    assert!(!PLAZA.contains(tile));
    tile
}

/// What an answer shows: the newest mark in it, and where the answer has
/// it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Newest {
    mark: u64,
    place: String,
}

impl Newest {
    /// Keeps a mark when it is newer than what was seen.
    fn see(&mut self, mark: u64, place: String) {
        if mark > self.mark {
            *self = Newest { mark, place };
        }
    }
}

/// Finds marks in JSON.
struct Reader {
    /// The mark of each tile that a resident stood on.
    tiles: BTreeMap<(i64, i64), u64>,
}

impl Reader {
    fn new() -> Reader {
        // Where every resident starts. That is no news.
        Reader { tiles: BTreeMap::from([((i64::from(START.x), i64::from(START.y)), 0)]) }
    }

    /// Says that the tile of a mark is in use.
    fn know(&mut self, mark: u64) {
        let pos = tile(mark);
        self.tiles.insert((i64::from(pos.x), i64::from(pos.y)), mark);
    }

    fn tile_mark(&self, x: &Value, y: &Value) -> Option<u64> {
        self.tiles.get(&(x.as_i64()?, y.as_i64()?)).copied()
    }

    /// Walks a JSON value and keeps the newest mark.
    ///
    /// A mark is: a number with the name `frame`, `marker` or
    /// `money_supply`; a position (`x`, `y`) on the tile of a mark; a move
    /// `[id, x, y]` to such a tile; and the name `Shop <mark>`.
    fn walk(&self, value: &Value, path: &str, newest: &mut Newest) {
        match value {
            Value::Object(map) => {
                if let (Some(x), Some(y)) = (map.get("x"), map.get("y"))
                    && let Some(mark) = self.tile_mark(x, y)
                {
                    newest.see(mark, format!("{path} (a position)"));
                }
                for (name, inner) in map {
                    let here = format!("{path}.{name}");
                    if matches!(name.as_str(), "frame" | "marker" | "money_supply")
                        && let Some(mark) = inner.as_u64()
                    {
                        newest.see(mark, here.clone());
                    }
                    if name == "moved"
                        && let Some(moves) = inner.as_array()
                    {
                        for step in moves {
                            if let Some(mark) = self.tile_mark(&step[1], &step[2]) {
                                newest.see(mark, format!("{here} (a move)"));
                            }
                        }
                    }
                    self.walk(inner, &here, newest);
                }
            }
            Value::Array(list) => {
                for (at, inner) in list.iter().enumerate() {
                    self.walk(inner, &format!("{path}[{at}]"), newest);
                }
            }
            Value::String(text) => {
                if let Some(mark) = text.strip_prefix("Shop ").and_then(|n| n.parse().ok()) {
                    newest.see(mark, format!("{path} (`{text}`)"));
                }
            }
            _ => {}
        }
    }

    /// Returns the newest mark of a JSON value.
    fn newest(&self, value: &Value) -> Newest {
        let mut newest = Newest::default();
        self.walk(value, "", &mut newest);
        newest
    }
}

/// The routes that the test reads at each point. `{ada}` stands for the id
/// of the resident.
const ROUTES: [&str; 13] = [
    "/v1/spectate/stats",
    "/v1/world",
    "/v1/world/zones/0",
    "/v1/world/zones/0?residents=false",
    "/v1/feed?limit=500",
    "/v1/residents/{ada}/profile",
    "/v1/director",
    "/v1/stories",
    "/v1/clips",
    "/v1/published",
    "/v1/published/finance.stats_published",
    "/v1/stats/economy",
    "/v1/leaderboards/wealth",
];

/// Reads every route and returns the newest mark of each. A route that
/// has nothing yet answers with an error, which has no mark.
async fn read_routes(harness: &Harness, reader: &Reader, ada: ResidentId) -> Vec<(String, Newest)> {
    let mut found = Vec::new();
    for route in ROUTES {
        let path = route.replace("{ada}", &ada.to_string());
        let (_, body) = harness.get(&path).await;
        found.push((path, reader.newest(&body)));
    }
    found
}

/// Checks that no route shows a mark above `live - DELAY`. Returns the
/// newest mark of each route.
async fn check_routes(
    harness: &Harness,
    reader: &Reader,
    ada: ResidentId,
    live: u64,
    when: &str,
) -> BTreeMap<String, u64> {
    let mut marks = BTreeMap::new();
    for (path, newest) in read_routes(harness, reader, ada).await {
        assert!(
            newest.mark <= live - DELAY,
            "{when}: the simulation is at frame {live}, and `{path}` shows frame {} at {}",
            newest.mark,
            newest.place
        );
        marks.insert(path, newest.mark);
    }
    marks
}

/// Checks the messages that the feed sent up to the simulation frame
/// `live`. Returns the newest mark in them.
fn check_messages(reader: &Reader, messages: &[ServerMessage], live: u64) -> u64 {
    let mut newest = 0;
    for message in messages {
        let found = reader.newest(&serde_json::to_value(message).expect("a message is JSON"));
        assert!(
            found.mark <= live - DELAY,
            "the simulation is at frame {live}, and the feed sent frame {} at {}: {message:?}",
            found.mark,
            found.place
        );
        newest = newest.max(found.mark);
    }
    newest
}

#[tokio::test]
async fn no_route_and_no_message_is_newer_than_the_delayed_frame() {
    let mut harness = Harness::start_with(delayed(), RuntimeConfig::manual()).await;
    let ada = harness.spawn("Ada").await;
    let bea = harness.spawn("Bea").await;
    let mut live = harness.steps(2).await;
    let (mut viewer, _) = harness.viewer().await;
    viewer.subscribe(&[0]).await;
    viewer.tune(&[Channel::Director, Channel::Stories]).await;
    let mut reader = Reader::new();
    let first = live + 1;
    let last = first + 11;
    let mut published = 0;
    let mut seen_on_feed = 0;

    for mark in first..=last {
        // Everything that Ada does now is reported in frame `mark`: she
        // goes to the tile of the mark, makes news, and builds a shop
        // there. Bea publishes numbers in every second round, because the
        // global realm keeps her busy for a frame.
        reader.know(mark);
        let place = tile(mark);
        harness.act(ada, "hop", json!({ "x": place.x, "y": place.y })).await;
        let news = json!({ "kind": "town.news", "drama": 90, "data": { "marker": mark } });
        harness.act(ada, "report", news).await;
        harness.act(ada, "build", json!({ "name": format!("Shop {mark}") })).await;
        if mark % 2 == 0 {
            harness.act(bea, "publish_stats", json!({ "money_supply": mark })).await;
            published = mark;
        }

        // The simulation has all of it, and no frame has reported it. A
        // read of the simulation at this moment sees the new shop.
        check_routes(&harness, &reader, ada, live, "after the actions").await;

        assert_eq!(harness.step().await, mark);
        live = mark;
        let messages = viewer.until_frame(live - DELAY).await;
        seen_on_feed = seen_on_feed.max(check_messages(&reader, &messages, live));
        check_routes(&harness, &reader, ada, live, "after the frame").await;
    }

    // The delayed state does arrive: each part of the stream shows the
    // frame that is exactly the delay old, and nothing of it is missing.
    let marks = check_routes(&harness, &reader, ada, live, "at the end").await;
    let shown = live - DELAY;
    assert!(shown >= first, "the marks of the test are on the stream");
    assert_eq!(seen_on_feed, shown, "the feed");
    for route in ["/v1/spectate/stats", "/v1/world", "/v1/feed?limit=500"] {
        assert_eq!(marks[route], shown, "{route}");
    }
    // Ada stands on the tile of that frame.
    assert_eq!(marks["/v1/world/zones/0"], shown);
    assert_eq!(marks[&format!("/v1/residents/{ada}/profile")], shown);
    // Bea published numbers in each frame with an even number. The stream
    // has the newest of them that are old enough.
    assert!(published > shown, "numbers that are too new exist");
    assert_eq!(marks["/v1/stats/economy"], shown - shown % 2);
    assert_eq!(marks["/v1/published/finance.stats_published"], shown - shown % 2);

    // The map of the zone comes through the same delay. The last shop is
    // on it when a request asked for the map after the shop was built and
    // the delay is over; it is never there before.
    let mut on_map = marks["/v1/world/zones/0?residents=false"];
    assert!(on_map <= shown);
    for _ in 0..2 * DELAY + 2 {
        live = harness.step().await;
        let marks = check_routes(&harness, &reader, ada, live, "while the map catches up").await;
        on_map = marks["/v1/world/zones/0?residents=false"];
    }
    assert_eq!(on_map, last, "the newest shop is on the map in the end");
    let messages = viewer.until_frame(live - DELAY).await;
    check_messages(&reader, &messages, live);
    harness.stop().await;
}
