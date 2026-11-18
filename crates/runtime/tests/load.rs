//! Behaviour under load: back-pressure, many waiting calls, statistics.

mod common;

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::task::Poll;
use std::time::{Duration, Instant};

use serde_json::json;
use unsup_kernel::{Input, ResidentId, World, ZoneId, ZoneTiming};
use unsup_protocol::{FrameOutput, Observation, SimError, WaitFilter};
use unsup_runtime::{
    DurableConfig, DurableJournal, Journal, JournalError, MemStorage, Pacer, RuntimeConfig,
    SimHandle, SimStats, SyncPolicy, start,
};

use common::{act, new_world, play, plugins, spec};

type Call<T> = Pin<Box<dyn Future<Output = Result<T, SimError>> + Send>>;

/// Polls a call once. `None` means that it waits for the simulation.
async fn poll_once<T>(call: &mut Call<T>) -> Option<Result<T, SimError>> {
    std::future::poll_fn(|cx| match call.as_mut().poll(cx) {
        Poll::Ready(answer) => Poll::Ready(Some(answer)),
        Poll::Pending => Poll::Ready(None),
    })
    .await
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

/// A journal that holds the simulation thread in `commit` until the test
/// lets go: a disk that is slow.
struct SlowDisk {
    entered: Sender<()>,
    release: Receiver<()>,
}

impl Journal for SlowDisk {
    fn record(&mut self, _input: &Input) -> Result<(), JournalError> {
        Ok(())
    }

    fn commit(&mut self) -> Result<(), JournalError> {
        let _ = self.entered.send(());
        let _ = self.release.recv();
        Ok(())
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

#[tokio::test]
async fn a_flood_gets_overloaded_at_once_and_nothing_blocks() {
    let (entered_tx, entered_rx) = channel();
    let (release_tx, release_rx) = channel();
    let config = RuntimeConfig {
        mailbox: 8,
        journal: Box::new(SlowDisk { entered: entered_tx, release: release_rx }),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(common::world(), config);
    // This binding is younger than the runtime. When an assertion fails it
    // is dropped first: the disk answers, and the runtime can stop.
    let release_tx = release_tx;

    // The first call is taken from the mailbox. The simulation thread then
    // waits for the disk.
    let mut first: Call<ResidentId> = Box::pin({
        let handle = handle.clone();
        async move { handle.spawn(spec("Ada")).await }
    });
    assert!(poll_once(&mut first).await.is_none());
    tokio::task::spawn_blocking(move || entered_rx.recv())
        .await
        .expect("the task ran")
        .expect("the journal was entered");

    // Twenty callers arrive while the thread is stuck. Eight fit in the
    // mailbox. The other twelve get their answer in the same poll: the
    // runtime sheds load, it does not make them wait.
    let started = Instant::now();
    let mut waiting = Vec::new();
    let mut refused = 0;
    for n in 0..20 {
        let mut call: Call<ResidentId> = Box::pin({
            let handle = handle.clone();
            async move { handle.spawn(spec(&format!("R{n}"))).await }
        });
        match poll_once(&mut call).await {
            None => waiting.push(call),
            Some(answer) => {
                assert_eq!(answer, Err(SimError::Overloaded));
                refused += 1;
            }
        }
    }
    assert_eq!((waiting.len(), refused), (8, 12));
    // Reads are shed the same way.
    assert_eq!(handle.stats().await.map(|_| ()), Err(SimError::Overloaded));
    assert!(started.elapsed() < Duration::from_secs(5), "the refusals did not wait for the disk");

    // The disk answers. The calls that were accepted are all served, in
    // order, and the refused ones left no trace.
    drop(release_tx);
    assert_eq!(first.await, Ok(ResidentId(1)));
    for (n, call) in waiting.into_iter().enumerate() {
        assert_eq!(call.await, Ok(ResidentId(n as u64 + 2)));
    }
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.residents, stats.inputs_applied), (9, 9));
    assert_eq!((stats.overloaded, stats.mailbox_capacity, stats.mailbox_depth), (13, 8, 0));
    assert_eq!(join.shutdown().expect("a clean stop").residents(), 9);
}

#[tokio::test]
async fn wait_wakes_the_right_callers_among_thousands() {
    const RESIDENTS: u64 = 3_000;
    let (handle, _join) = start(common::world(), RuntimeConfig::manual());
    for n in 0..RESIDENTS {
        handle.spawn(spec(&format!("R{n}"))).await.expect("spawn");
    }
    handle.step(1).await.expect("step");

    // Every resident has a call that waits, with a long timeout.
    let mut calls = Vec::new();
    for id in 1..=RESIDENTS {
        let handle = handle.clone();
        calls.push(tokio::spawn(async move {
            handle.wait(ResidentId(id), WaitFilter::Any, Duration::from_secs(120)).await
        }));
    }
    until(&handle, |s| s.waiting == RESIDENTS).await;
    // A frame without news wakes nobody.
    handle.step(1).await.expect("step");
    assert_eq!(handle.stats().await.expect("stats").waiting, RESIDENTS);

    // Every seventh resident gets a notice from its neighbour.
    let chosen: Vec<u64> = (1..=RESIDENTS).filter(|id| id.is_multiple_of(7)).collect();
    for &id in &chosen {
        let tell = act("tell", json!({ "to": id, "kind": "town.greeting" }));
        handle.act(ResidentId(id - 1), tell).await.expect("act");
    }
    handle.step(1).await.expect("step");
    let stats = handle.stats().await.expect("stats");
    assert_eq!(stats.waiting, RESIDENTS - chosen.len() as u64);

    // Exactly the chosen calls returned, each with the notice for its own
    // resident. The others still wait.
    for (index, call) in calls.iter().enumerate() {
        let id = index as u64 + 1;
        assert_eq!(call.is_finished(), id.is_multiple_of(7), "resident {id}");
    }
    for &id in &chosen {
        let call = std::mem::replace(
            &mut calls[id as usize - 1],
            tokio::spawn(async { Err(SimError::Invalid) }),
        );
        let observation: Observation = call.await.expect("the task ran").expect("an observation");
        assert_eq!(observation.resident, ResidentId(id));
        let from: Vec<_> = observation.notices.iter().map(|n| (n.kind.as_str(), n.from)).collect();
        assert_eq!(from, [("town.greeting", Some(ResidentId(id - 1)))]);
    }
}

#[tokio::test]
async fn a_wait_with_news_behind_its_cursor_answers_at_once() {
    let (handle, _join) = start(common::world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    let tell = act("tell", json!({ "to": bob, "kind": "town.greeting" }));
    handle.act(ada, tell).await.expect("act");
    handle.step(1).await.expect("step");
    let hour = Duration::from_secs(3_600);
    let wait = |since: Option<u64>| -> Call<Observation> {
        let handle = handle.clone();
        Box::pin(async move { handle.wait_since(bob, WaitFilter::Any, since, hour).await })
    };

    // The notice arrived before the call. A cursor before it finds it: the
    // call takes no place among the waiting calls. The statistics call is
    // behind it in the mailbox, so the runtime has handled it by then.
    let mut behind = wait(Some(0));
    assert!(poll_once(&mut behind).await.is_none(), "the call is in the mailbox");
    assert_eq!(handle.stats().await.expect("stats").waiting, 0);
    let observation = behind.await.expect("an observation");
    let kinds: Vec<&str> = observation.notices.iter().map(|n| n.kind.as_str()).collect();
    assert_eq!(kinds, ["town.greeting"]);

    // A cursor at the end of the inbox, and a call without a cursor, have
    // nothing to find: they wait.
    let mut at_end = wait(Some(observation.next_notice));
    assert!(poll_once(&mut at_end).await.is_none());
    assert_eq!(handle.stats().await.expect("stats").waiting, 1);
    let mut from_now = wait(None);
    assert!(poll_once(&mut from_now).await.is_none());
    assert_eq!(handle.stats().await.expect("stats").waiting, 2);
    assert!(poll_once(&mut at_end).await.is_none() && poll_once(&mut from_now).await.is_none());
}

#[tokio::test]
async fn waiting_calls_end_at_their_own_deadline() {
    let (handle, _join) = start(common::world(), RuntimeConfig::manual());
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    let bob = handle.spawn(spec("Bob")).await.expect("spawn");
    let wait = |resident, millis| {
        let handle = handle.clone();
        tokio::spawn(async move {
            let started = Instant::now();
            let answer =
                handle.wait(resident, WaitFilter::Any, Duration::from_millis(millis)).await;
            (answer, started.elapsed())
        })
    };
    // The call with the later deadline is made first.
    let long = wait(ada, 120_000);
    let short = wait(bob, 40);
    let (answer, elapsed) = short.await.expect("the task ran");
    assert!(answer.expect("an observation").notices.is_empty());
    assert!(elapsed >= Duration::from_millis(40), "{elapsed:?}");
    // The other call is not touched by the deadline of the first.
    let stats = handle.stats().await.expect("stats");
    assert_eq!(stats.waiting, 1);
    assert!(!long.is_finished());
}

#[tokio::test]
async fn the_number_of_waiting_calls_is_bounded() {
    let config =
        RuntimeConfig { max_waiters: 3, max_waiters_per_resident: 2, ..RuntimeConfig::manual() };
    let (handle, _join) = start(common::world(), config);
    let mut ids = Vec::new();
    for name in ["Ada", "Bob", "Cy", "Di"] {
        ids.push(handle.spawn(spec(name)).await.expect("spawn"));
    }
    handle.step(1).await.expect("step");
    let wait = |resident: ResidentId| {
        let handle = handle.clone();
        tokio::spawn(async move {
            handle.wait(resident, WaitFilter::Any, Duration::from_secs(120)).await
        })
    };

    // One resident: the third call ends the first one early, with an
    // observation that holds no news, like a timeout.
    let first = wait(ids[0]);
    until(&handle, |s| s.waiting == 1).await;
    let second = wait(ids[0]);
    until(&handle, |s| s.waiting == 2).await;
    let third = wait(ids[0]);
    let early = first.await.expect("the task ran").expect("an observation");
    assert!(early.notices.is_empty());
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.waiting, stats.overloaded), (2, 0));
    assert!(!second.is_finished() && !third.is_finished());

    // The whole runtime: three calls wait, the next one is refused.
    let other = wait(ids[1]);
    until(&handle, |s| s.waiting == 3).await;
    assert_eq!(
        handle.wait(ids[2], WaitFilter::Any, Duration::from_secs(120)).await.map(|_| ()),
        Err(SimError::Overloaded)
    );
    // A poll (timeout zero) needs no place and still works.
    handle.wait(ids[2], WaitFilter::Any, Duration::ZERO).await.expect("a poll");
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.waiting, stats.overloaded), (3, 1));

    // A call whose caller went away frees its place at the next call for
    // the same resident. The mailbox keeps the order of the two calls.
    other.abort();
    assert!(other.await.expect_err("aborted").is_cancelled());
    let mut again: Call<Observation> = Box::pin({
        let (handle, bob) = (handle.clone(), ids[1]);
        async move { handle.wait(bob, WaitFilter::Any, Duration::from_secs(120)).await }
    });
    assert!(poll_once(&mut again).await.is_none());
    assert_eq!(
        handle.wait(ids[3], WaitFilter::Any, Duration::from_secs(120)).await.map(|_| ()),
        Err(SimError::Overloaded),
        "the new call of the same resident took the place of the caller that went away"
    );
    assert!(poll_once(&mut again).await.is_none(), "the new call waits");
    let stats = handle.stats().await.expect("stats");
    assert_eq!((stats.waiting, stats.overloaded), (3, 2));
}

/// A pacer that makes zone 3 skip every second frame.
struct EverySecond(u64);

impl Pacer for EverySecond {
    fn skip(&mut self, _timings: &[ZoneTiming], _tick_interval: Duration) -> Vec<ZoneId> {
        self.0 += 1;
        if self.0.is_multiple_of(2) { vec![ZoneId(3)] } else { Vec::new() }
    }

    fn dilated_zones(&self) -> usize {
        1
    }
}

#[tokio::test]
async fn the_statistics_show_the_load() {
    let storage = MemStorage::new();
    let durable = DurableConfig {
        sync: SyncPolicy::Interval(Duration::from_secs(3_600)),
        snapshot_every: 4,
        ..DurableConfig::default()
    };
    let opened = DurableJournal::open(Arc::new(storage.clone()), plugins(), durable, new_world)
        .expect("the storage opens");
    let config = RuntimeConfig {
        journal: Box::new(opened.journal),
        pacer: Box::new(EverySecond(0)),
        mailbox: 100,
        stats_window: 4,
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(opened.world, config);
    let empty = handle.stats().await.expect("stats");
    assert_eq!(
        (empty.frames_run, empty.p50_frame_micros, empty.p99_frame_micros, empty.inputs_per_second),
        (0, 0, 0, 0)
    );
    assert_eq!((empty.journal_records, empty.journal_lag, empty.snapshots), (0, 0, 0));

    play(&handle, 0, 6).await;
    let stats = until(&handle, |s| s.snapshots == 1).await;

    // Frames: 6 ran; the percentiles look at the last 4 and are ordered.
    assert_eq!((stats.frame, stats.frames_run), (6, 6));
    assert!(stats.p50_frame_micros <= stats.p95_frame_micros);
    assert!(stats.p95_frame_micros <= stats.p99_frame_micros);
    assert!(stats.p99_frame_micros <= stats.worst_frame_micros);
    assert!(stats.p50_frame_micros > 0, "a frame takes more than no time");
    // Time dilation: the pacer skipped one zone in frames 2, 4 and 6.
    assert_eq!((stats.dilated_zones, stats.zone_ticks_skipped), (1, 3));
    // The journal: every input is a record. The snapshot of frame 4 forced
    // the log to the disk; what came later is not forced yet.
    assert_eq!(stats.journal_records, stats.inputs_applied);
    assert_eq!((stats.snapshots, stats.last_snapshot_frame, stats.snapshot_errors), (1, 4, 0));
    assert!(stats.journal_lag > 0 && stats.journal_lag < stats.journal_records, "{stats:?}");
    assert!(stats.last_snapshot_bytes > 112, "a header and a payload");
    assert!(stats.last_snapshot_micros > 0 && stats.last_snapshot_write_micros > 0);
    // The mailbox: this call was the only one in it, and it is taken out
    // before it is answered.
    assert_eq!((stats.mailbox_depth, stats.mailbox_capacity, stats.overloaded), (0, 100, 0));
    assert_eq!((stats.journal_errors, stats.rollbacks, stats.waiting), (0, 0, 0));
    // The simulation thread did work in every phase of a batch.
    assert!(stats.intake_micros > 0 && stats.apply_micros > 0, "{stats:?}");

    // The numbers survive the wire.
    let text = serde_json::to_string(&stats).expect("the statistics are serializable");
    assert_eq!(serde_json::from_str::<SimStats>(&text).expect("and readable"), stats);
    // An older reader's document, without the newer fields, still reads.
    let old: SimStats = serde_json::from_str(r#"{"frame":7,"residents":2}"#).expect("readable");
    assert_eq!((old.frame, old.residents, old.journal_lag), (7, 2, 0));
    join.shutdown().expect("a clean stop");
}

#[tokio::test]
async fn the_input_rate_is_measured_over_a_window() {
    let config =
        RuntimeConfig { rate_window: Duration::from_millis(20), ..RuntimeConfig::manual() };
    let (handle, _join) = start(common::world(), config);
    let ada = handle.spawn(spec("Ada")).await.expect("spawn");
    // Inputs flow until a window that saw them is over.
    let started = Instant::now();
    let mut sent = 1u64;
    let stats = loop {
        handle.act(ada, act("chop", json!({}))).await.expect("act");
        sent += 1;
        let stats = handle.stats().await.expect("stats");
        if stats.inputs_per_second > 0 {
            break stats;
        }
        assert!(started.elapsed() < Duration::from_secs(20), "no window ever ended");
    };
    // A window is at least 20 ms long, so the rate of a window is at most
    // everything that was sent, 50 times per second.
    assert!(stats.inputs_per_second <= sent * 50, "{sent} inputs, {stats:?}");
    assert_eq!(stats.inputs_applied, sent);
}
