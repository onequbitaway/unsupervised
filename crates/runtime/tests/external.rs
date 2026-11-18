//! Messages from a service outside the simulation: they reach the global
//! realm, they are in the input log, and a replay delivers them again.

mod common;

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize, Serializer};
use serde_json::json;
use unsup_kernel::{Message, RealmId, ResidentId, World, WorldConfig, ZoneId};
use unsup_protocol::{ObserveOpts, SimError, WaitFilter};
use unsup_runtime::{
    DurableConfig, DurableJournal, Fault, MemStorage, RuntimeConfig, RuntimeJoin, SimHandle,
    SimStats, SyncPolicy, recover, replay, start,
};

use common::{Board, Decree, Posted, Town, hash_of, new_world, play, plugins, spec};

/// Opens the durable world of a storage and starts a runtime with manual
/// stepping for it. Only the first snapshot is taken, so a recovery replays
/// the whole log.
fn open(storage: &MemStorage) -> (SimHandle, RuntimeJoin) {
    let durable = DurableConfig {
        sync: SyncPolicy::EveryBatch,
        snapshot_every: 0,
        ..DurableConfig::default()
    };
    let opened = DurableJournal::open(Arc::new(storage.clone()), plugins(), durable, new_world)
        .expect("the storage opens");
    let runtime = RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
    start(opened.world, runtime)
}

/// Runs a script against a world that has no journal and is never
/// interrupted, and returns the world.
async fn twin<F: Future<Output = ()>>(script: impl FnOnce(SimHandle) -> F) -> World {
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    script(handle).await;
    join.shutdown().expect("a clean stop")
}

async fn until(handle: &SimHandle, condition: impl Fn(&SimStats) -> bool) -> SimStats {
    loop {
        let stats = handle.stats().await.expect("stats");
        if condition(&stats) {
            return stats;
        }
        tokio::task::yield_now().await;
    }
}

fn decree(title: &str, to: u64, bonus: i64) -> Decree {
    Decree { title: title.to_string(), to: ResidentId(to), bonus }
}

/// Returns what the global realm wrote on its board.
fn board(world: &World) -> Vec<Posted> {
    world.realm(RealmId::Global).expect("the global realm is hosted").res::<Board>().posted.clone()
}

fn posted(frame: u64, title: &str) -> Posted {
    Posted { frame, from: RealmId::Global, title: title.to_string() }
}

async fn cash(handle: &SimHandle, resident: ResidentId) -> i64 {
    let observation = handle.observe(resident, ObserveOpts::default()).await.expect("observe");
    observation.sections["self"]["cash"].as_i64().expect("cash is a number")
}

#[tokio::test]
async fn a_message_from_outside_reaches_the_global_realm() {
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    assert_eq!(cash(&handle, ada).await, 100, "the start money of the town");
    let waiting = tokio::spawn({
        let handle = handle.clone();
        let filter = WaitFilter::Kinds { kinds: vec!["town.decree".into()] };
        async move { handle.wait(ada, filter, Duration::from_secs(60)).await }
    });
    until(&handle, |s| s.waiting == 1).await;
    let before = handle.stats().await.expect("stats");

    // The service sends the message. It is one input of the world, and
    // nothing runs before the next frame.
    assert_eq!(handle.send_global(&decree("Founders' day", ada.0, 250)).await, Ok(()));
    let after = handle.stats().await.expect("stats");
    assert_eq!((after.inputs_applied, after.frame), (before.inputs_applied + 1, 0));
    assert_eq!((cash(&handle, ada).await, after.waiting), (100, 1));

    // Frame 1: the global realm takes the message and wires the bonus.
    // Frame 2: the zone of the resident gets the money and the notice.
    handle.step(1).await.expect("step");
    assert_eq!(cash(&handle, ada).await, 100, "the bonus is on its way");
    handle.step(1).await.expect("step");
    assert_eq!(cash(&handle, ada).await, 350);
    let observation = waiting.await.expect("the task ran").expect("an observation");
    let notices: Vec<_> =
        observation.notices.iter().map(|n| (n.kind.as_str(), n.from, n.data.clone())).collect();
    assert_eq!(notices, [("town.decree", None, json!({ "title": "Founders' day", "paid": true }))]);

    // The handler ran once, in frame 1, and saw the global realm as sender.
    let world = join.shutdown().expect("a clean stop");
    assert_eq!(board(&world), [posted(1, "Founders' day")]);
    // The bonus was made and moved through the ledgers: nothing is lost.
    let totals = world.audit().expect("money and goods balance");
    assert_eq!((totals.money.minted, totals.money.held, totals.money.in_flight), (350, 350, 0));
}

#[tokio::test]
async fn a_message_from_outside_is_in_the_log_and_a_replay_delivers_it_again() {
    let storage = MemStorage::new();
    let (handle, join) = open(&storage);
    play(&handle, 0, 3).await;
    let before = handle.stats().await.expect("stats").journal_records;
    assert_eq!(handle.send_global(&decree("Harvest", 1, 40)).await, Ok(()));
    assert_eq!(handle.stats().await.expect("stats").journal_records, before + 1, "one record");
    play(&handle, 3, 5).await;
    // This one is still in the inbox of the global realm when the process
    // dies: no frame ran after it.
    assert_eq!(handle.send_global(&decree("Winter", 2, 60)).await, Ok(()));
    let records = handle.stats().await.expect("stats").journal_records;

    // The process dies here.
    let disk = storage.fork();
    let live = join.shutdown().expect("a clean stop");
    assert_eq!(board(&live), [posted(4, "Harvest")]);

    // A recovery applies the whole log, the two messages with it, and
    // gives the same world: the board, the money and the inbox.
    let recovery = recover(&disk, &[&Town::default()]).expect("the world recovers");
    assert_eq!((recovery.report.snapshot_seq, recovery.report.inputs), (0, records));
    assert_eq!(hash_of(&recovery.world), hash_of(&live));
    assert_eq!(board(&recovery.world), [posted(4, "Harvest")]);
    recovery.world.audit().expect("money and goods balance after the recovery");

    // The messages are state. A run with the same script and without them
    // is another world.
    let without = twin(|handle| async move { play(&handle, 0, 5).await }).await;
    assert_eq!(board(&without), []);
    assert_ne!(hash_of(&without), hash_of(&live));

    // The server starts again on the same disk. The message that waited is
    // delivered once, in the first frame after the start.
    let (handle, join) = open(&disk);
    handle.step(2).await.expect("step");
    let after = join.shutdown().expect("a clean stop");
    assert_eq!(board(&after), [posted(4, "Harvest"), posted(6, "Winter")]);
    let uninterrupted = twin(|handle| async move {
        play(&handle, 0, 3).await;
        handle.send_global(&decree("Harvest", 1, 40)).await.expect("send");
        play(&handle, 3, 5).await;
        handle.send_global(&decree("Winter", 2, 60)).await.expect("send");
        handle.step(2).await.expect("step");
    })
    .await;
    assert_eq!(hash_of(&after), hash_of(&uninterrupted));
    after.audit().expect("money and goods balance");

    // The replay check passes over both messages and ends in the same
    // state, with the state hash of the final snapshot on the way.
    let report = replay(&disk, &[&Town::default()], &Default::default()).expect("the replay runs");
    assert_eq!((report.snapshot_frame, report.frame, report.inputs), (0, 7, records + 2));
    assert_eq!(common::hex(&report.state_hash), hash_of(&after));
    assert_eq!(report.checkpoints.len(), 1, "the final snapshot");
    assert!(report.is_consistent(), "{report:?}");
}

/// Writes a number as text.
fn as_text<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

/// A message that does not read back what it wrote: the writer makes text
/// of the bonus, the reader wants a number. It claims the kind of
/// [`Decree`].
#[derive(Serialize, Deserialize)]
struct OneWay {
    title: String,
    to: ResidentId,
    #[serde(serialize_with = "as_text")]
    bonus: i64,
}

impl Message for OneWay {
    const KIND: &'static str = "town.decree";
}

/// A message that no module takes.
#[derive(Serialize, Deserialize)]
struct Unheard {
    text: String,
}

impl Message for Unheard {
    const KIND: &'static str = "town.unheard";
}

#[tokio::test]
async fn a_message_that_cannot_be_delivered_is_refused_and_changes_nothing() {
    let storage = MemStorage::new();
    let (handle, join) = open(&storage);
    handle.spawn(spec("Ada")).await.expect("spawn");
    let before = handle.stats().await.expect("stats");
    let unchanged = |stats: &SimStats| {
        (stats.inputs_applied, stats.journal_records)
            == (before.inputs_applied, before.journal_records)
    };

    // The handler of the global realm could not read this message, and a
    // handler that cannot read its message stops the simulation. The
    // message never gets into the mailbox.
    let one_way = OneWay { title: "Trap".into(), to: ResidentId(1), bonus: 5 };
    assert_eq!(handle.send_global(&one_way).await, Err(SimError::Invalid));
    assert!(unchanged(&handle.stats().await.expect("stats")));

    // The disk fails: the message cannot be recorded, so the world does
    // not get it.
    storage.set_fault(Some(Fault::Append));
    assert_eq!(handle.send_global(&decree("Lost", 1, 5)).await, Err(SimError::Overloaded));
    storage.set_fault(None);
    let stats = handle.stats().await.expect("stats");
    assert!(unchanged(&stats), "{stats:?}");
    assert_eq!(stats.journal_errors, 1);

    // A message that no module takes is an input like any other. The
    // global realm drops it.
    assert_eq!(handle.send_global(&Unheard { text: "anybody?".into() }).await, Ok(()));
    handle.step(2).await.expect("step");
    let stats = handle.stats().await.expect("stats");
    assert_eq!(stats.journal_records, before.journal_records + 3, "the message and two frames");

    // None of the three reached a handler.
    let world = join.shutdown().expect("a clean stop");
    assert_eq!(board(&world), []);
    let expected = twin(|handle| async move {
        handle.spawn(spec("Ada")).await.expect("spawn");
        handle.send_global(&Unheard { text: "anybody?".into() }).await.expect("send");
        handle.step(2).await.expect("step");
    })
    .await;
    assert_eq!(hash_of(&world), hash_of(&expected));
    // After the stop the call says so.
    assert_eq!(handle.send_global(&decree("Late", 1, 5)).await, Err(SimError::Shutdown));
}

#[tokio::test]
async fn a_world_without_the_global_realm_takes_no_message_from_outside() {
    // This process hosts the four zones. Another process has the global
    // realm.
    let zones = (0..4).map(|zone| RealmId::Zone(ZoneId(zone))).collect();
    let config = WorldConfig { hosted: Some(zones), ..WorldConfig::with_seed(3) };
    let world = World::new(&[&Town::default()], config);
    let (handle, join) = start(world, RuntimeConfig::manual());
    handle.spawn(spec("Ada")).await.expect("spawn");

    assert_eq!(handle.send_global(&decree("Nowhere", 1, 5)).await, Err(SimError::NotHosted));
    let stats = handle.stats().await.expect("stats");
    assert_eq!(stats.inputs_applied, 1, "the spawn only");
    let mut world = join.shutdown().expect("a clean stop");
    // Nothing for the global realm went to the outbox for other processes.
    let kinds: Vec<String> =
        world.take_external().iter().map(|envelope| envelope.kind().to_string()).collect();
    assert!(!kinds.iter().any(|kind| kind == "town.decree"), "{kinds:?}");
}
