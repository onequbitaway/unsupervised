//! The runtime through its handle: act, observe, wait, real time, shutdown.

use std::future::Future;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    ActionError, AttrDef, Begin, Builder, Holder, Input, Money, Plugin, ResidentId, Scope, Stage,
    VerbSpec, World, WorldConfig, ZoneId, ZoneTiming,
};
use unsup_protocol::{
    ActionRequest, ActionStatus, AdminCommand, AdminReply, CityId, FrameOutput, ObserveOpts,
    OwnerId, SimError, SpawnSpec, ViewQuery, WaitFilter, WorldView,
};
use unsup_runtime::{Journal, JournalError, Pacer, RuntimeConfig, SimHandle, start};

/// Arguments of `work`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WorkArgs {
    /// Duration in ticks.
    ticks: u32,
}

/// Arguments of `tell`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct TellArgs {
    /// The resident that gets the notice.
    to: ResidentId,
    /// The kind of the notice.
    kind: String,
}

/// Arguments of verbs that take none.
#[derive(Serialize, Deserialize, JsonSchema)]
struct Nothing {}

/// A small module: `work` pays after some ticks, `tell` writes a notice to
/// another resident, `crash` panics.
struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        let wages = b.faucet("town.wages");
        let energy = b.attr(AttrDef::new("energy").range(0, 1_000).default(1_000));
        if b.scope() == Scope::Zone {
            b.system(Stage::Update, 0, "town.tire", move |realm| {
                for value in realm.attrs_mut(energy).iter_mut() {
                    *value = (*value - 1).max(0);
                }
            });
        }
        b.verb::<WorkArgs>(
            VerbSpec {
                verb: "work",
                module: "town",
                summary: "Work for pay.",
                free: false,
                allowed_in: &[],
            },
            |_, _, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, _| {
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(500));
                Ok(json!({ "pay": 500 }))
            },
        );
        b.instant_verb::<TellArgs>(
            VerbSpec {
                verb: "tell",
                module: "town",
                summary: "Tell a resident something.",
                free: true,
                allowed_in: &[],
            },
            |realm, ctx, args| {
                if realm.idx_of(args.to).is_none() {
                    return Err(ActionError::not_found("Nobody with this id is here."));
                }
                realm.notify(args.to, &args.kind, Some(ctx.resident), json!({ "hello": true }));
                Ok(Value::Null)
            },
        );
        b.instant_verb::<Nothing>(
            VerbSpec {
                verb: "crash",
                module: "town",
                summary: "A bug.",
                free: true,
                allowed_in: &[],
            },
            |_, _, _| panic!("the town module has a bug"),
        );
    }
}

fn spec(name: &str) -> SpawnSpec {
    SpawnSpec { owner: OwnerId(1), name: name.to_string(), model: None, city: Some(CityId(0)) }
}

fn world() -> World {
    World::new(&[&Town], WorldConfig::with_seed(3))
}

fn act(verb: &str, args: Value) -> ActionRequest {
    ActionRequest::new(verb, args)
}

async fn cash(handle: &SimHandle, r: ResidentId) -> i64 {
    let observation = handle.observe(r, ObserveOpts::default()).await.expect("observe");
    observation.sections["self"]["cash"].as_i64().expect("cash is a number")
}

#[tokio::test]
async fn act_and_observe_through_the_handle() {
    let (handle, join) = start(world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    assert_eq!(ada, ResidentId(1));
    assert_eq!(handle.spawn(spec("")).await, Err(SimError::Invalid));

    let ack = handle.act(ada, act("work", json!({ "ticks": 3 }))).await.expect("act");
    assert!(matches!(ack.status, ActionStatus::Started { ticks: 3, .. }), "{:?}", ack.status);
    assert_eq!(
        handle.act(ResidentId(9), act("work", json!({ "ticks": 1 }))).await,
        Err(SimError::UnknownResident)
    );

    // Manual mode: the thread answers calls, but nothing moves until a step.
    assert_eq!(handle.stats().await.expect("stats").frames_run, 0);
    let observation = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
    assert_eq!(
        (observation.frame, observation.sections["self"]["activity"]["verb"].clone()),
        (0, json!("work"))
    );
    assert_eq!(cash(&handle, ada).await, 0);

    assert_eq!(handle.step(2).await, Ok(2));
    assert_eq!(cash(&handle, ada).await, 0);
    assert_eq!(handle.step(1).await, Ok(3));
    let observation = handle.observe(ada, ObserveOpts::default()).await.expect("observe");
    assert_eq!(observation.frame, 3);
    assert_eq!(observation.sections["self"]["cash"], json!(500));
    assert_eq!(observation.sections["self"]["attrs"]["energy"], json!(997));
    assert_eq!(observation.sections["reports"][0]["action_id"], json!(ack.action_id));
    assert_eq!(
        handle.observe(ResidentId(9), ObserveOpts::default()).await.map(|_| ()),
        Err(SimError::UnknownResident)
    );

    // Views, administration, the catalogue and the statistics.
    let view = handle.view(ViewQuery::Zone { zone: ZoneId(0) }).await.expect("view");
    let WorldView::Zone { residents, .. } = view else { panic!("a zone view") };
    assert_eq!(residents.iter().map(|r| r.id).collect::<Vec<_>>(), [ada]);
    assert_eq!(handle.view(ViewQuery::Zone { zone: ZoneId(77) }).await, Err(SimError::Invalid));
    assert_eq!(handle.admin(AdminCommand::Freeze { resident: ada }).await, Ok(AdminReply::Ok));
    let frozen = handle.act(ada, act("work", json!({ "ticks": 1 }))).await.expect("act");
    assert!(matches!(frozen.status, ActionStatus::Rejected { .. }));
    let catalogue = handle.catalogue();
    assert!(catalogue.verbs.iter().any(|v| v.verb == "work" && v.module == "town"));
    assert!(catalogue.verbs.iter().any(|v| v.verb == "wait" && v.module == "kernel"));

    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.frame, stats.frames_run, stats.residents, stats.waiting), (3, 3, 1, 0));
    // Spawn, two acts that reached the world, three frames, one admin
    // command. The refused spawn and the act of an unknown resident were
    // inputs too.
    assert_eq!(stats.inputs_applied, 9);
    assert!(stats.worst_frame_micros >= stats.mean_frame_micros);
    assert!(stats.worst_frame_micros >= stats.last_frame_micros);

    let world = join.shutdown().expect("a clean stop");
    assert_eq!(world.frame(), 3);
}

#[tokio::test]
async fn wait_wakes_when_a_notice_arrives() {
    let (handle, _join) = start(world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    handle.step(1).await.expect("step");

    // Ada waits for anything. The timeout is long: the test fails by time
    // if the wake does not work.
    let waiting = tokio::spawn({
        let handle = handle.clone();
        async move { handle.wait(ada, WaitFilter::Any, Duration::from_secs(30)).await }
    });
    // The wait is registered when the statistics show it.
    while handle.stats().await.expect("stats").waiting == 0 {
        tokio::task::yield_now().await;
    }
    handle.step(2).await.expect("step");
    assert!(!waiting.is_finished(), "frames without news do not wake the call");

    let started = Instant::now();
    handle.act(bob, act("tell", json!({ "to": ada, "kind": "town.greeting" }))).await.expect("act");
    handle.step(1).await.expect("step");
    let observation =
        waiting.await.expect("the task ran").expect("the wait returns an observation");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "the notice woke the call, not the timeout"
    );
    assert_eq!(observation.notices.len(), 1);
    assert_eq!(
        (observation.notices[0].kind.as_str(), observation.notices[0].from),
        ("town.greeting", Some(bob))
    );
    assert_eq!(handle.stats().await.expect("stats").waiting, 0);
}

#[tokio::test]
async fn wait_filters_by_kind_and_by_action_reports() {
    let (handle, _join) = start(world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    handle.step(1).await.expect("step");
    let registered = |n: u64| {
        let handle = handle.clone();
        async move {
            while handle.stats().await.expect("stats").waiting < n {
                tokio::task::yield_now().await;
            }
        }
    };

    // A call that waits for one kind sleeps through another kind.
    let kinds = WaitFilter::Kinds { kinds: vec!["town.alarm".into()] };
    let for_alarm = tokio::spawn({
        let handle = handle.clone();
        async move { handle.wait(ada, kinds, Duration::from_secs(30)).await }
    });
    // A call that waits for the end of an action sleeps through notices.
    let for_report = tokio::spawn({
        let handle = handle.clone();
        async move { handle.wait(ada, WaitFilter::ActionDone, Duration::from_secs(30)).await }
    });
    registered(2).await;
    handle.act(bob, act("tell", json!({ "to": ada, "kind": "town.greeting" }))).await.expect("act");
    handle.step(1).await.expect("step");
    assert_eq!(
        handle.stats().await.expect("stats").waiting,
        2,
        "a greeting is neither an alarm nor a report"
    );

    handle.act(bob, act("tell", json!({ "to": ada, "kind": "town.alarm" }))).await.expect("act");
    handle.step(1).await.expect("step");
    let observation = for_alarm.await.expect("the task ran").expect("an observation");
    let kinds: Vec<&str> = observation.notices.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(
        kinds,
        ["town.greeting", "town.alarm"],
        "everything since the call comes with the wake"
    );
    assert_eq!(handle.stats().await.expect("stats").waiting, 1);

    let ack = handle.act(ada, act("work", json!({ "ticks": 2 }))).await.expect("act");
    handle.step(1).await.expect("step");
    assert!(!for_report.is_finished());
    handle.step(1).await.expect("step");
    let observation = for_report.await.expect("the task ran").expect("an observation");
    assert_eq!(observation.sections["reports"][0]["action_id"], json!(ack.action_id));

    // With a cursor, news that is already there answers at once.
    let all = handle
        .wait_since(ada, WaitFilter::Any, Some(0), Duration::from_secs(30))
        .await
        .expect("an observation");
    assert_eq!(all.notices.len(), 2);
    let cursor = all.next_notice;
    // Without news the call returns at its timeout, with nothing in it.
    let started = Instant::now();
    let empty = handle
        .wait_since(ada, WaitFilter::Any, Some(cursor), Duration::from_millis(60))
        .await
        .expect("an observation");
    assert!(started.elapsed() >= Duration::from_millis(60));
    assert!(empty.notices.is_empty());
    assert_eq!(empty.sections["reports"], json!([]));
    assert_eq!(empty.next_notice, cursor);
    // A timeout of zero is a poll.
    let poll = handle.wait(ada, WaitFilter::Any, Duration::ZERO).await.expect("an observation");
    assert!(poll.notices.is_empty());
    assert_eq!(
        handle.wait(ResidentId(42), WaitFilter::Any, Duration::from_secs(1)).await.map(|_| ()),
        Err(SimError::UnknownResident)
    );
}

#[tokio::test]
async fn real_time_ticking_advances_frames() {
    let (handle, join) = start(world(), RuntimeConfig::real_time(Duration::from_millis(5)));
    let mut frames = handle.frames();
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.act(ada, act("work", json!({ "ticks": 4 }))).await.expect("act");

    // Frames arrive by themselves, in order, without a call to `step`.
    let mut numbers = Vec::new();
    while numbers.len() < 6 {
        let frame = tokio::time::timeout(Duration::from_secs(10), frames.recv())
            .await
            .expect("a frame arrives in time")
            .expect("the runtime is alive");
        numbers.push(frame.frame);
    }
    assert!(numbers.windows(2).all(|pair| pair[1] == pair[0] + 1), "{numbers:?}");

    // The action ended and woke a waiting call, all by the clock.
    let done = handle
        .wait_since(ada, WaitFilter::ActionDone, Some(0), Duration::from_secs(10))
        .await
        .expect("an observation");
    assert_eq!(done.sections["reports"][0]["verb"], json!("work"));
    assert_eq!(done.sections["self"]["cash"], json!(500));
    let stats = handle.stats().await.expect("stats");
    assert!(stats.frame >= 6 && stats.frames_run == stats.frame, "{stats:?}");
    assert!(stats.mean_frame_micros < 5_000_000);

    // The pace is the interval, not as fast as possible. A new subscription
    // gets only frames from now on, so eight of them span at least seven
    // intervals, less the time that the first one was already due.
    let mut paced = handle.frames();
    let started = Instant::now();
    let mut last = 0;
    for _ in 0..8 {
        last = tokio::time::timeout(Duration::from_secs(10), paced.recv())
            .await
            .expect("a frame arrives in time")
            .expect("the runtime is alive")
            .frame;
    }
    assert!(
        started.elapsed() >= Duration::from_millis(25),
        "eight frames in {:?}",
        started.elapsed()
    );
    let after = last;
    let world = join.shutdown().expect("a clean stop");
    assert!(world.frame() >= after);
}

#[tokio::test]
async fn shutdown_is_clean() {
    let (handle, join) = start(world(), RuntimeConfig::real_time(Duration::from_millis(2)));
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    handle.act(ada, act("work", json!({ "ticks": 1 }))).await.expect("act");
    let waiting = tokio::spawn({
        let handle = handle.clone();
        async move {
            handle
                .wait(
                    ada,
                    WaitFilter::Kinds { kinds: vec!["never".into()] },
                    Duration::from_secs(60),
                )
                .await
        }
    });
    while handle.stats().await.expect("stats").waiting == 0 {
        tokio::task::yield_now().await;
    }
    let mut frames = handle.frames();
    assert!(!join.is_finished());
    // At least one frame runs before the stop.
    tokio::time::timeout(Duration::from_secs(10), frames.recv())
        .await
        .expect("a frame arrives in time")
        .expect("the runtime is alive");

    // The world comes back with its state.
    let world = join.shutdown().expect("a clean stop");
    assert!(world.frame() > 0);
    assert_eq!(world.residents(), 1);
    assert_eq!(world.locate(ada), Some(ZoneId(0)));
    world.audit().expect("the world that comes back is balanced");

    // A call that waited is released, and later calls fail at once.
    assert_eq!(waiting.await.expect("the task ran").map(|_| ()), Err(SimError::Shutdown));
    assert_eq!(handle.spawn(spec("Bob")).await, Err(SimError::Shutdown));
    assert_eq!(
        handle.observe(ada, ObserveOpts::default()).await.map(|_| ()),
        Err(SimError::Shutdown)
    );
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
    assert_eq!(handle.stats().await.map(|_| ()), Err(SimError::Shutdown));
    // The frame stream ends after the frames that were already sent.
    let mut last = frames.recv().await;
    while !matches!(last, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
        last = frames.recv().await;
    }
    // A subscription that is made after the stop is closed from the start.
    assert!(matches!(
        handle.frames().recv().await,
        Err(tokio::sync::broadcast::error::RecvError::Closed)
    ));
    // The catalogue is still readable: it is a copy.
    assert!(!handle.catalogue().verbs.is_empty());
}

#[tokio::test]
async fn dropping_the_join_value_stops_the_thread() {
    let (handle, join) = start(world(), RuntimeConfig::manual());
    handle.step(1).await.expect("step");
    drop(join);
    assert_eq!(handle.step(1).await, Err(SimError::Shutdown));
}

/// A journal that writes every input down and counts the frames.
struct Recorder {
    inputs: Arc<Mutex<Vec<Input>>>,
    frames: Arc<Mutex<Vec<(u64, u64)>>>,
}

impl Journal for Recorder {
    fn record(&mut self, input: &Input) -> Result<(), JournalError> {
        self.inputs.lock().expect("lock").push(input.clone());
        Ok(())
    }

    fn frame_done(&mut self, world: &World, frame: &FrameOutput) {
        self.frames.lock().expect("lock").push((frame.frame, world.frame()));
    }
}

#[tokio::test]
async fn the_journal_sees_every_input_and_a_replay_gives_the_same_world() {
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let frames = Arc::new(Mutex::new(Vec::new()));
    let config = RuntimeConfig {
        journal: Box::new(Recorder { inputs: Arc::clone(&inputs), frames: Arc::clone(&frames) }),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(world(), config);
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    handle.act(ada, act("work", json!({ "ticks": 2 }))).await.expect("act");
    handle.step(1).await.expect("step");
    handle.act(bob, act("tell", json!({ "to": ada, "kind": "town.greeting" }))).await.expect("act");
    handle.admin(AdminCommand::Freeze { resident: bob }).await.expect("admin");
    handle.step(3).await.expect("step");
    // Reads are not inputs.
    handle.observe(ada, ObserveOpts::default()).await.expect("observe");
    handle.view(ViewQuery::Layout).await.expect("view");
    let live = join.shutdown().expect("a clean stop");

    let log = inputs.lock().expect("lock").clone();
    let kinds: Vec<&str> = log
        .iter()
        .map(|input| match input {
            Input::Spawn(_) => "spawn",
            Input::Act { .. } => "act",
            Input::Step { .. } => "step",
            Input::Admin(_) => "admin",
            Input::Deliver { .. } => "deliver",
        })
        .collect();
    assert_eq!(kinds, ["spawn", "spawn", "act", "step", "act", "admin", "step", "step", "step"]);
    assert_eq!(*frames.lock().expect("lock"), [(1, 1), (2, 2), (3, 3), (4, 4)]);

    // The log survives the wire, and a fresh world that is fed the log
    // reaches the state of the live world.
    let mut replayed = world();
    for input in &log {
        let text = serde_json::to_string(input).expect("an input is serializable");
        let input: Input = serde_json::from_str(&text).expect("and readable");
        replayed.apply(input);
    }
    assert_eq!(replayed.state_hash(), live.state_hash());
    assert_eq!(replayed.frame(), 4);
}

/// A journal that refuses everything.
struct Broken;

impl Journal for Broken {
    fn record(&mut self, _input: &Input) -> Result<(), JournalError> {
        Err(JournalError { message: "the disk is full".into() })
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

#[tokio::test]
async fn an_input_that_cannot_be_recorded_is_not_applied() {
    let config = RuntimeConfig { journal: Box::new(Broken), ..RuntimeConfig::manual() };
    let (handle, join) = start(world(), config);
    assert_eq!(handle.spawn(spec("Ada")).await, Err(SimError::Overloaded));
    assert_eq!(handle.step(1).await, Err(SimError::Overloaded));
    let stats = handle.stats().await.expect("stats");
    assert_eq!(
        (stats.frame, stats.residents, stats.inputs_applied, stats.journal_errors),
        (0, 0, 0, 2)
    );
    let world = join.shutdown().expect("a clean stop");
    assert_eq!((world.frame(), world.residents()), (0, 0));
}

/// A journal that holds the simulation thread until the test lets go.
struct Gate {
    entered: Sender<()>,
    release: Receiver<()>,
}

impl Journal for Gate {
    fn record(&mut self, _input: &Input) -> Result<(), JournalError> {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        Ok(())
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

#[tokio::test]
async fn a_full_mailbox_answers_overloaded() {
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let config = RuntimeConfig {
        mailbox: 1,
        journal: Box::new(Gate { entered: entered_tx, release: release_rx }),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(world(), config);

    // The first call is taken from the mailbox and is held in the journal.
    let first = tokio::spawn({
        let handle = handle.clone();
        async move { handle.spawn(spec("Ada")).await }
    });
    tokio::task::spawn_blocking(move || entered_rx.recv())
        .await
        .expect("the task ran")
        .expect("the journal was entered");

    // The second call fills the one slot of the mailbox.
    let mut second = std::pin::pin!(handle.stats());
    let pending =
        std::future::poll_fn(|cx| Poll::Ready(second.as_mut().poll(cx).is_pending())).await;
    assert!(pending, "the second call waits in the mailbox");
    // The third finds it full.
    assert_eq!(handle.stats().await.map(|_| ()), Err(SimError::Overloaded));

    // When the thread goes on, both earlier calls are answered.
    release_tx.send(()).expect("the journal waits");
    assert_eq!(first.await.expect("the task ran"), Ok(ResidentId(1)));
    assert_eq!(second.await.map(|s| s.residents), Ok(1));
    drop(release_tx);
    join.shutdown().expect("a clean stop");
}

/// A pacer that always skips zone 0 and remembers what it was shown.
struct SkipZero {
    seen: Arc<Mutex<Vec<Vec<ZoneTiming>>>>,
}

impl Pacer for SkipZero {
    fn skip(&mut self, timings: &[ZoneTiming], tick_interval: Duration) -> Vec<ZoneId> {
        assert_eq!(tick_interval, Duration::ZERO);
        self.seen.lock().expect("lock").push(timings.to_vec());
        vec![ZoneId(0)]
    }
}

#[tokio::test]
async fn the_pacer_decides_which_zones_skip_a_frame() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let config = RuntimeConfig {
        pacer: Box::new(SkipZero { seen: Arc::clone(&seen) }),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(world(), config);
    handle.step(3).await.expect("step");
    let world = join.shutdown().expect("a clean stop");
    assert_eq!(world.frame(), 3);
    let tick = |zone: u32| {
        world.realm(unsup_kernel::RealmId::Zone(ZoneId(zone))).expect("hosted").tick().0
    };
    assert_eq!((tick(0), tick(1), tick(3)), (0, 3, 3), "zone 0 never ticked");
    // The pacer got the zone timings of the frame before, from the second
    // frame on: one entry per zone, zero for the zone that was skipped.
    let seen = seen.lock().expect("lock");
    assert_eq!(seen.len(), 3);
    assert!(seen[0].is_empty());
    assert_eq!(
        seen[1].iter().map(|t| t.zone).collect::<Vec<_>>(),
        [ZoneId(0), ZoneId(1), ZoneId(2), ZoneId(3)]
    );
    assert_eq!(seen[2][0].elapsed, 0);
}

#[tokio::test]
async fn a_panic_in_the_simulation_is_reported() {
    let (handle, join) = start(world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    // The module panics. The call that hit the bug and every later call
    // fail; they do not hang.
    assert_eq!(
        handle.act(ada, act("crash", Value::Null)).await.map(|_| ()),
        Err(SimError::Shutdown)
    );
    assert_eq!(handle.stats().await.map(|_| ()), Err(SimError::Shutdown));
    while !join.is_finished() {
        tokio::task::yield_now().await;
    }
    let error = join.shutdown().expect_err("the thread panicked");
    assert_eq!(error.message, "the town module has a bug");
    assert_eq!(error.to_string(), "the simulation thread panicked: the town module has a bug");
}
