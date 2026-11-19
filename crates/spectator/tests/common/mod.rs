//! A small town and a server for the tests of the spectator service.
//!
//! The town is a kernel plugin with verbs that make exactly the output
//! that a test needs: a move, a speech event, an event with a chosen drama
//! score, an event of a chosen kind with chosen data, published state. The
//! simulation runs in manual stepping mode.

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, Audience, Begin, Builder, CityConfig, EventMeta, Plugin, Pos, Rect, ResidentId,
    Scope, Site, SiteId, UNCLAIMED, Untrusted, VerbSpec, World, WorldConfig, ZoneId,
};
use unsup_protocol::{
    ActionRequest, ActionStatus, CityId, OwnerId, ResidentView, SpawnSpec, WorldView,
};
use unsup_runtime::storage::{MemStorage, Storage};
use unsup_runtime::{RuntimeConfig, RuntimeJoin, SimHandle};
use unsup_spectator::wire::{Channel, ClientMessage, ServerMessage};
use unsup_spectator::{Config, router_with_storage};

/// Tiles on one side of a zone of the test world.
pub const ZONE_SIZE: u32 = 16;

/// The site where new residents appear: in zone 0.
pub const PLAZA: Rect = Rect::new(4, 4, 9, 9);

/// The tile where every new resident stands: the centre of the plaza.
pub const START: Pos = Pos::new(6, 6);

/// Arguments of verbs that name a tile.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct TileArgs {
    /// Column of the tile.
    pub x: i32,
    /// Row of the tile.
    pub y: i32,
}

/// Arguments of `say`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct SayArgs {
    /// The words.
    pub text: String,
}

/// Arguments of `stir` and `announce`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StirArgs {
    /// Drama score of the event.
    pub drama: u8,
    /// Make the event about this resident, without an actor.
    #[serde(default)]
    pub about: Option<ResidentId>,
}

/// Arguments of `build`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct BuildArgs {
    /// Name of the new site.
    pub name: String,
}

/// Arguments of `publish_stats`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct StatsArgs {
    /// Key of the series, if any.
    #[serde(default)]
    pub key: Option<String>,
    /// The money supply to publish.
    pub money_supply: i64,
}

/// Arguments of `publish_board`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct BoardArgs {
    /// Name of the board.
    pub board: String,
    /// The resident at the top of the board.
    pub top: ResidentId,
}

/// Arguments of `report`.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct ReportArgs {
    /// Kind of the event. It must be one of [`REPORT_KINDS`].
    pub kind: String,
    /// Drama score of the event.
    pub drama: u8,
    /// Data of the event.
    #[serde(default)]
    pub data: Value,
}

/// The kinds of events that `report` can make. The kernel takes the kind
/// of an event as static text.
pub const REPORT_KINDS: [&str; 12] = [
    "crime.theft_committed",
    "crime.arrest_made",
    "crime.suspect_charged",
    "economy.lease_started",
    "economy.worker_hired",
    "finance.loan_defaulted",
    "finance.loan_granted",
    "life.resident_died",
    "politics.election_decided",
    "society.contract_breached",
    "society.opinion_given",
    "town.news",
];

/// Arguments of verbs that take none.
#[derive(Serialize, Deserialize, JsonSchema)]
pub struct Nothing {}

fn spec(verb: &'static str, summary: &'static str) -> VerbSpec {
    VerbSpec { verb, module: "town", summary, free: false, allowed_in: &[] }
}

/// The test module.
pub struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        if b.scope() == Scope::Zone {
            b.on_worldgen(0, |realm, _| {
                let info = realm.zone_info().expect("zone hooks run in zones");
                // A road along the north side of every zone.
                let road = Rect::sized(info.bounds.x0, info.bounds.y0, ZONE_SIZE, 1);
                realm.tiles_mut().fill(road, 1);
                if info.id == ZoneId(0) {
                    let id = SiteId(realm.alloc_id());
                    realm
                        .sites_mut()
                        .insert(Site::new(id, "park", "Plaza", PLAZA).with_tag(Site::SPAWN_TAG));
                    realm.tiles_mut().fill(PLAZA, 2);
                }
            });
        }
        b.instant_verb::<TileArgs>(spec("hop", "Go to a tile at once."), |realm, ctx, args| {
            realm
                .set_pos(ctx.idx, Pos::new(args.x, args.y))
                .map_err(|e| ActionError::invalid_args(format!("You cannot go there: {e}.")))?;
            Ok(Value::Null)
        });
        b.verb::<TileArgs>(
            spec("walk", "Go to a tile in two ticks."),
            |_, _, _| Ok(Begin::Busy { ticks: 2 }),
            |realm, ctx, args| {
                realm
                    .set_pos(ctx.idx, Pos::new(args.x, args.y))
                    .map_err(|e| ActionError::invalid_args(format!("You cannot go there: {e}.")))?;
                Ok(Value::Null)
            },
        );
        b.instant_verb::<SayArgs>(spec("say", "Say something aloud."), |realm, ctx, args| {
            realm.emit(
                EventMeta::public("society.said", 3)
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx))
                    .audience(Audience::Nearby(5)),
                &json!({ "text": Untrusted::new(ctx.resident, args.text.clone()) }),
            );
            Ok(Value::Null)
        });
        b.instant_verb::<StirArgs>(spec("stir", "Make a public event."), |realm, ctx, args| {
            let meta = EventMeta::public("town.stirred", args.drama).at(realm.pos(ctx.idx));
            match args.about {
                Some(resident) => realm.emit(meta, &json!({ "resident": resident })),
                None => realm.emit(meta.actor(ctx.resident), &json!({})),
            }
            Ok(Value::Null)
        });
        b.instant_verb::<ReportArgs>(
            spec("report", "Make a public event of a kind, with data."),
            |realm, ctx, args| {
                let kind =
                    REPORT_KINDS.iter().find(|kind| **kind == args.kind).ok_or_else(|| {
                        ActionError::invalid_args("The test town does not know this kind.")
                    })?;
                let meta =
                    EventMeta::public(kind, args.drama).actor(ctx.resident).at(realm.pos(ctx.idx));
                realm.emit(meta, &args.data);
                Ok(Value::Null)
            },
        );
        b.instant_verb::<BuildArgs>(spec("build", "Build a shop here."), |realm, ctx, args| {
            let pos = realm.pos(ctx.idx);
            let rect = Rect::sized(pos.x, pos.y, 2, 2);
            let id = SiteId(realm.alloc_id());
            realm.sites_mut().insert(Site::new(id, "shop", args.name.clone(), rect));
            realm.tiles_mut().fill(rect, 4);
            Ok(Value::Null)
        });
        b.instant_verb::<Nothing>(spec("leave", "Leave the world."), |realm, ctx, _| {
            realm.despawn(ctx.idx, UNCLAIMED, "left");
            Ok(Value::Null)
        });
        b.global_verb::<StirArgs>(
            spec("announce", "Make a public event without a place."),
            |realm, ctx, args| {
                realm.emit(
                    EventMeta::public("town.announced", args.drama).actor(ctx.resident),
                    &json!({}),
                );
                Ok(Value::Null)
            },
        );
        b.global_verb::<StatsArgs>(
            spec("publish_stats", "Publish the statistics of the economy."),
            |realm, _, args| {
                let mut data = json!({ "money_supply": args.money_supply });
                if let Some(key) = &args.key {
                    data["key"] = json!(key);
                }
                realm.emit(EventMeta::public("finance.stats_published", 0), &data);
                Ok(Value::Null)
            },
        );
        b.global_verb::<BoardArgs>(
            spec("publish_board", "Publish a leaderboard."),
            |realm, _, args| {
                // The shape of the missions module
                // (`docs/design/modules/missions.md`, section 10.3).
                realm.emit(
                    EventMeta::public("missions.leaderboard_published", 0),
                    &board_data(&args.board, args.top),
                );
                Ok(Value::Null)
            },
        );
    }
}

/// Returns the data of a published board with one resident at the top, in
/// the shape of the missions module.
pub fn board_data(board: &str, top: ResidentId) -> Value {
    json!({
        "key": board, "season": 1, "unit": "cents", "residents": 1,
        "model_source": "self_reported",
        "top": [{
            "rank": 1, "resident": top, "name": "Ada", "owner": 1, "model": "test-model",
            "score": 100,
        }],
        "by_owner": [{ "rank": 1, "owner": 1, "score": 100, "residents": 1, "mean": 100 }],
        "by_org": [],
        "by_model": [{ "rank": 1, "model": "test-model", "score": 100, "residents": 1, "mean": 100 }],
    })
}

/// Makes the test world: one city of 2 by 2 zones, 16 tiles per zone.
/// Zone 0 is north-west, zone 1 north-east, zone 2 south-west, zone 3
/// south-east.
pub fn world() -> World {
    let config = WorldConfig {
        zone_size: ZONE_SIZE,
        cities: vec![CityConfig::new("Testville", Pos::new(0, 0), 2)],
        ..WorldConfig::with_seed(5)
    };
    World::new(&[&Town], config)
}

/// The limits that most tests use: the built-in ones.
pub fn limits() -> Config {
    Config::default()
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Real time after which a test gives up waiting for a message. A correct
/// run never waits this long.
const GIVE_UP: Duration = Duration::from_secs(20);

/// A WebSocket client of `/v1/spectate`.
pub struct Viewer {
    socket: Socket,
}

impl Viewer {
    /// Connects. The first message, `world`, is still unread.
    pub async fn connect(addr: SocketAddr) -> Viewer {
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/v1/spectate"))
            .await
            .expect("the viewer connects");
        Viewer { socket }
    }

    /// Sends a request.
    pub async fn send(&mut self, message: &ClientMessage) {
        let text = serde_json::to_string(message).expect("a request is JSON");
        self.send_raw(Message::Text(text.into())).await;
    }

    /// Sends any WebSocket message.
    pub async fn send_raw(&mut self, message: Message) {
        self.socket.send(message).await.expect("the request leaves");
    }

    /// Subscribes to zones.
    pub async fn subscribe(&mut self, zones: &[u32]) {
        let zones = zones.iter().copied().map(ZoneId).collect();
        self.send(&ClientMessage::Subscribe { zones }).await;
    }

    /// Chooses the channels of the show.
    pub async fn tune(&mut self, channels: &[Channel]) {
        self.send(&ClientMessage::Tune { channels: channels.to_vec() }).await;
    }

    /// Returns the next message of the feed, or `None` when the server
    /// closed the connection.
    pub async fn try_next(&mut self) -> Option<ServerMessage> {
        loop {
            let message = tokio::time::timeout(GIVE_UP, self.socket.next())
                .await
                .expect("the feed sends a message in time")?;
            match message {
                Ok(Message::Text(text)) => {
                    return Some(serde_json::from_str(&text).unwrap_or_else(|error| {
                        panic!("the feed sent something else than a message: {error}: {text}")
                    }));
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => {}
            }
        }
    }

    /// Returns the next message of the feed.
    pub async fn next(&mut self) -> ServerMessage {
        self.try_next().await.expect("the feed is open")
    }

    /// Returns every message up to the `frame` message of a frame, that
    /// message included.
    pub async fn until_frame(&mut self, frame: u64) -> Vec<ServerMessage> {
        let mut messages = Vec::new();
        loop {
            let message = self.next().await;
            let end = matches!(&message, ServerMessage::Frame { frame: f, .. } if *f == frame);
            messages.push(message);
            if end {
                return messages;
            }
        }
    }
}

/// A running simulation with the spectator service on a local port.
pub struct Harness {
    /// The door to the simulation.
    pub sim: SimHandle,
    /// The address of the service.
    pub addr: SocketAddr,
    http: reqwest::Client,
    /// A viewer without a subscription. It tells the harness when the
    /// cache has applied a frame.
    control: Viewer,
    server: JoinHandle<()>,
    join: Option<RuntimeJoin>,
    /// Frames that the service is behind the simulation.
    delay: u64,
}

impl Harness {
    /// Starts the test world in manual stepping mode with the built-in
    /// limits.
    pub async fn start() -> Harness {
        Harness::start_with(limits(), RuntimeConfig::manual()).await
    }

    /// Starts the test world with other limits.
    pub async fn start_with(config: Config, runtime: RuntimeConfig) -> Harness {
        Harness::start_world(world(), config, runtime, Arc::new(MemStorage::new())).await
    }

    /// Starts a world with a storage for the service. With a delay of the
    /// stream in the limits, it first runs that many frames: the service
    /// answers nobody before.
    pub async fn start_world(
        world: World,
        config: Config,
        runtime: RuntimeConfig,
        storage: Arc<dyn Storage>,
    ) -> Harness {
        let delay = config.stream.delay_frames;
        let (sim, join) = unsup_runtime::start(world, runtime);
        let app = router_with_storage(sim.clone(), config, storage).expect("the limits are valid");
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a local port");
        let addr = listener.local_addr().expect("the address of the port");
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("the server runs");
        });
        if delay > 0 {
            // The service answers nobody until the simulation is `delay`
            // frames past the frame at which the service read the world.
            // The harness runs frames until the service answers. A test
            // cannot know how many these are: it uses the frame numbers
            // that `step` returns.
            let client = reqwest::Client::new();
            let ready = client.get(format!("http://{addr}/v1/spectate/stats")).send();
            tokio::pin!(ready);
            loop {
                tokio::select! {
                    biased;
                    answer = &mut ready => {
                        answer.expect("the service answers after the delay");
                        break;
                    }
                    frame = sim.step(1) => {
                        frame.expect("step");
                    }
                }
            }
        }
        let mut control = Viewer::connect(addr).await;
        let first = control.next().await;
        assert!(matches!(first, ServerMessage::World { .. }), "{first:?}");
        Harness {
            sim,
            addr,
            http: reqwest::Client::new(),
            control,
            server,
            join: Some(join),
            delay,
        }
    }

    /// Connects one more viewer and reads its `world` message.
    pub async fn viewer(&self) -> (Viewer, ServerMessage) {
        let mut viewer = Viewer::connect(self.addr).await;
        let world = viewer.next().await;
        (viewer, world)
    }

    /// Creates a resident. It appears at [`START`] in zone 0.
    pub async fn spawn(&self, name: &str) -> ResidentId {
        let spec = SpawnSpec {
            owner: OwnerId(1),
            name: name.to_string(),
            model: Some("test-model".to_string()),
            city: Some(CityId(0)),
        };
        self.sim.spawn(spec).await.expect("spawn")
    }

    /// Sends an action that the simulation must accept.
    pub async fn act(&self, resident: ResidentId, verb: &str, args: Value) {
        let ack = self.sim.act(resident, ActionRequest::new(verb, args)).await.expect("act");
        assert!(!matches!(ack.status, ActionStatus::Rejected { .. }), "{verb}: {:?}", ack.status);
    }

    /// Runs one frame and waits until the cache has applied it. Returns
    /// the number of the frame.
    pub async fn step(&mut self) -> u64 {
        let frame = self.sim.step(1).await.expect("step");
        self.settle(frame).await;
        frame
    }

    /// Waits until the service has applied what the simulation gave it
    /// up to a frame: with a delay of the stream, the cache then holds the
    /// frame that is the delay older.
    pub async fn settle(&mut self, frame: u64) {
        self.control.until_frame(frame - self.delay).await;
    }

    /// Runs `frames` frames, one at a time.
    pub async fn steps(&mut self, frames: u32) -> u64 {
        let mut last = 0;
        for _ in 0..frames {
            last = self.step().await;
        }
        last
    }

    /// Sends a GET request and returns the status and the JSON body.
    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        let response =
            self.http.get(format!("http://{}{path}", self.addr)).send().await.expect("a response");
        let status = response.status();
        let body = response.json().await.expect("a JSON body");
        (status, body)
    }

    /// Sends a GET request that must succeed and returns the JSON body.
    pub async fn get_ok(&self, path: &str) -> Value {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        body
    }

    /// Returns the residents of a zone as `/v1/world/zones/{zone}` gives
    /// them.
    pub async fn residents(&self, zone: u32) -> Vec<ResidentView> {
        let body = self.get_ok(&format!("/v1/world/zones/{zone}")).await;
        match serde_json::from_value::<WorldView>(body).expect("a zone view") {
            WorldView::Zone { residents, .. } => residents,
            other => panic!("not a zone view: {other:?}"),
        }
    }

    /// Sends a POST request without a body and returns the status and
    /// the JSON body.
    pub async fn post(&self, path: &str) -> (StatusCode, Value) {
        let response =
            self.http.post(format!("http://{}{path}", self.addr)).send().await.expect("a response");
        let status = response.status();
        let body = response.json().await.expect("a JSON body");
        (status, body)
    }

    /// Stops the simulation. The server stays.
    pub fn stop_simulation(&mut self) {
        self.take_world();
    }

    /// Stops the simulation and returns its world. The server stays.
    pub fn take_world(&mut self) -> World {
        let join = self.join.take().expect("the simulation runs");
        join.shutdown().expect("the simulation stops without a panic")
    }

    /// Waits until the service has written everything to its storage and
    /// closed the feed. Call it after the simulation stopped.
    pub async fn feed_closed(&mut self) {
        while self.control.try_next().await.is_some() {}
    }

    /// Stops the simulation, when it still runs, and the server.
    pub async fn stop(mut self) {
        if self.join.is_some() {
            self.stop_simulation();
        }
        self.server.abort();
    }
}

/// Makes the view of a resident of the test world.
pub fn resident(id: ResidentId, name: &str, pos: Pos, activity: Option<&str>) -> ResidentView {
    ResidentView {
        id,
        name: name.to_string(),
        pos,
        activity: activity.map(str::to_string),
        model: Some("test-model".to_string()),
    }
}

/// What a viewer knows about one zone after it applied the messages of
/// the feed, by the rules of `docs/design/protocol.md`.
#[derive(Default, Debug)]
pub struct Model {
    /// Residents by id.
    pub residents: std::collections::BTreeMap<ResidentId, ResidentView>,
}

impl Model {
    /// Applies one message for `zone`. Messages for other zones and
    /// messages without residents change nothing.
    pub fn apply(&mut self, zone: ZoneId, message: &ServerMessage) {
        match message {
            ServerMessage::Snapshot { zone: z, residents, .. } if *z == zone => {
                self.residents = residents.iter().map(|r| (r.id, r.clone())).collect();
            }
            ServerMessage::Delta { zone: z, moved, entered, left, activity, .. } if *z == zone => {
                for id in left {
                    self.residents.remove(id);
                }
                for view in entered {
                    self.residents.insert(view.id, view.clone());
                }
                for (id, x, y) in moved {
                    if let Some(resident) = self.residents.get_mut(id) {
                        resident.pos = Pos::new(*x, *y);
                    }
                }
                for (id, verb) in activity {
                    if let Some(resident) = self.residents.get_mut(id) {
                        resident.activity.clone_from(verb);
                    }
                }
            }
            _ => {}
        }
    }

    /// Returns the residents in ascending id order.
    pub fn list(&self) -> Vec<ResidentView> {
        self.residents.values().cloned().collect()
    }
}
