//! Time dilation: overloaded zones skip frames, and a replay repeats it.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use unsup_kernel::{Input, RealmId, World, WorldConfig, ZoneId, ZoneTiming};
use unsup_protocol::FrameOutput;
use unsup_runtime::{
    Dilation, DilationConfig, DurableConfig, DurableJournal, Journal, JournalError, MemStorage,
    Pacer, RuntimeConfig, SyncPolicy, recover, replay, start,
};

use common::{Town, hash_of, play, plugins};

const MS: u64 = 1_000_000;

/// A pacer for a budget of 10 ms per zone tick on `threads` threads.
fn pacer(threads: usize) -> Dilation {
    Dilation::new(DilationConfig {
        budget: Some(Duration::from_millis(10)),
        budget_percent: 50,
        relax_percent: 70,
        max_divisor: 10,
        smoothing: 1,
        calm_ticks: 2,
        threads,
    })
}

/// Feeds the pacer the way the kernel does: one timing per zone after
/// every frame, 0 for a zone that did not tick. `cost` is the tick time of
/// each zone in milliseconds. Returns the zones that skip, frame by frame.
struct Feed<P: Pacer> {
    pacer: P,
    last_skip: Vec<ZoneId>,
    first: bool,
}

impl<P: Pacer> Feed<P> {
    fn new(pacer: P) -> Feed<P> {
        Feed { pacer, last_skip: Vec::new(), first: true }
    }

    fn frame(&mut self, cost: &[u64]) -> Vec<u32> {
        let timings: Vec<ZoneTiming> = if self.first {
            // Before the first frame nothing was measured.
            Vec::new()
        } else {
            cost.iter()
                .enumerate()
                .map(|(zone, &ms)| {
                    let zone = ZoneId(zone as u32);
                    let elapsed = if self.last_skip.contains(&zone) { 0 } else { ms * MS };
                    ZoneTiming { zone, elapsed }
                })
                .collect()
        };
        self.first = false;
        self.last_skip = self.pacer.skip(&timings, Duration::from_secs(1));
        self.last_skip.iter().map(|zone| zone.0).collect()
    }

    fn frames(&mut self, cost: &[u64], n: usize) -> Vec<Vec<u32>> {
        (0..n).map(|_| self.frame(cost)).collect()
    }
}

/// Counts in how many of the frames a zone ticked.
fn ticks(frames: &[Vec<u32>], zone: u32) -> usize {
    frames.iter().filter(|skip| !skip.contains(&zone)).count()
}

#[test]
fn zones_within_the_budget_never_skip() {
    let mut feed = Feed::new(pacer(8));
    // Exactly the budget is not above it.
    let frames = feed.frames(&[1, 10, 9, 3], 50);
    assert!(frames.iter().all(Vec::is_empty), "{frames:?}");
    assert_eq!(feed.pacer.dilated_zones(), 0);
}

#[test]
fn only_the_heavy_zone_skips_and_just_enough() {
    let mut feed = Feed::new(pacer(8));
    // Zone 2 needs 35 ms per tick: three and a half budgets. One tick in
    // four frames brings it under the budget.
    let cost = [1, 2, 35, 1];
    assert_eq!(feed.frame(&cost), Vec::<u32>::new(), "nothing is measured yet");
    assert_eq!(feed.frame(&cost), [2], "the first measurement slows the zone at once");
    let frames = feed.frames(&cost, 40);
    assert!(frames.iter().all(|skip| skip.is_empty() || skip == &[2]), "{frames:?}");
    assert_eq!(ticks(&frames, 2), 10, "one tick in four frames");
    assert_eq!((feed.pacer.divisor(ZoneId(2)), feed.pacer.divisor(ZoneId(0))), (4, 1));
    assert_eq!(feed.pacer.dilated_zones(), 1);
    // Between two ticks the zone skips exactly three frames.
    let skips: Vec<bool> = frames.iter().map(|skip| skip == &[2]).collect();
    let longest = skips.split(|skipped| !skipped).map(<[bool]>::len).max();
    assert_eq!(longest, Some(3));
}

#[test]
fn no_zone_runs_slower_than_one_tick_in_ten_frames() {
    let mut feed = Feed::new(pacer(8));
    // One hundred budgets per tick. The floor holds anyway.
    let cost = [1, 1_000];
    feed.frames(&cost, 2);
    let frames = feed.frames(&cost, 200);
    assert_eq!(feed.pacer.divisor(ZoneId(1)), 10);
    assert_eq!(ticks(&frames, 1), 20);
    for window in frames.windows(10) {
        assert!(ticks(window, 1) >= 1, "ten frames in a row without a tick");
    }
    assert_eq!(ticks(&frames, 0), 200, "the light zone never skips");
}

#[test]
fn the_floor_holds_while_other_zones_change_their_pace() {
    // Two zones at the slowest pace take turns. Then the load of zone 0
    // falls, and it speeds up step by step. Each step moves the turn of
    // zone 1. Whenever that happens, zone 1 still ticks at least once in
    // every ten frames.
    for heavy_frames in 12..32 {
        let mut feed = Feed::new(pacer(8));
        let mut frames = feed.frames(&[1_000, 1_000], heavy_frames);
        assert_eq!((feed.pacer.divisor(ZoneId(0)), feed.pacer.divisor(ZoneId(1))), (10, 10));
        frames.extend(feed.frames(&[5, 1_000], 150));
        assert_eq!((feed.pacer.divisor(ZoneId(0)), feed.pacer.divisor(ZoneId(1))), (1, 10));
        // The first two frames come before any measurement.
        for (start, window) in frames[2..].windows(10).enumerate() {
            assert!(
                ticks(window, 1) >= 1,
                "zone 1 did not tick in the ten frames from frame {} on, after {heavy_frames} heavy frames",
                start + 2
            );
        }
    }
}

#[test]
fn a_zone_speeds_up_step_by_step_and_not_at_the_edge() {
    let mut feed = Feed::new(pacer(8));
    feed.frames(&[45], 3);
    assert_eq!(feed.pacer.divisor(ZoneId(0)), 5);

    // The load falls to 9 ms: under the budget, but above 70 % of it. The
    // zone speeds up one step for every two calm measurements, until one
    // more step would not leave the margin: at one tick in two frames.
    let mut divisors = Vec::new();
    for _ in 0..60 {
        feed.frame(&[9]);
        divisors.push(feed.pacer.divisor(ZoneId(0)));
    }
    let mut steps = divisors.clone();
    steps.dedup();
    assert_eq!(steps, [5, 4, 3, 2], "one step at a time, never up again");
    assert_eq!(divisors.last(), Some(&2), "9 ms is too close to the budget for full speed");

    // At 12 ms the zone is above the budget again when it runs every
    // frame, and at 9 ms it stays where it is: no flip between two paces.
    let frames = feed.frames(&[9], 40);
    assert_eq!(ticks(&frames, 0), 20);
    // Well below the margin it returns to the normal pace.
    feed.frames(&[6], 10);
    assert_eq!(feed.pacer.divisor(ZoneId(0)), 1);
    assert_eq!(feed.pacer.dilated_zones(), 0);
    assert!(feed.frames(&[6], 20).iter().all(Vec::is_empty));
}

#[test]
fn one_slow_measurement_does_not_slow_a_zone_with_smoothing() {
    let mut feed = Feed::new(Dilation::new(DilationConfig {
        budget: Some(Duration::from_millis(10)),
        smoothing: 4,
        threads: 8,
        ..DilationConfig::default()
    }));
    feed.frames(&[4], 5);
    // One tick of 20 ms among ticks of 4 ms: the average moves to 8 ms.
    feed.frame(&[20]);
    let frames = feed.frames(&[4], 10);
    assert!(frames.iter().all(Vec::is_empty), "{frames:?}");
    // A zone that stays slow is slowed after a few frames.
    let frames = feed.frames(&[40], 10);
    assert!(frames.iter().any(|skip| skip == &[0]), "{frames:?}");
}

#[test]
fn zones_that_fit_alone_share_the_threads() {
    // Eight zones of 9 ms each on two threads: each is under the budget,
    // but a frame needs 36 ms of the 20 ms that two threads give.
    let mut feed = Feed::new(pacer(2));
    let cost = [9; 8];
    feed.frames(&cost, 4);
    let frames = feed.frames(&cost, 40);
    // The heaviest zone gives way first, and among equals the one with the
    // lowest number, until the sum fits: 6 * 9/4 + 2 * 9/3 = 19.5 ms. One
    // step less for any zone would break the limit.
    let divisors: Vec<u32> = (0..8).map(|zone| feed.pacer.divisor(ZoneId(zone))).collect();
    assert_eq!(divisors, [4, 4, 4, 4, 4, 4, 3, 3]);
    assert_eq!(feed.pacer.dilated_zones(), 8);
    // The slow zones take turns. On average 2.17 zones tick per frame;
    // never more than three do, and never none.
    let ticking: Vec<usize> = frames.iter().map(|skip| 8 - skip.len()).collect();
    assert_eq!((ticking.iter().min(), ticking.iter().max()), (Some(&1), Some(&3)), "{ticking:?}");
    // Each zone keeps its own pace: one tick in four or in three frames.
    assert_eq!((ticks(&frames, 0), ticks(&frames[..39], 7)), (10, 13));
}

#[test]
fn manual_stepping_has_no_budget_and_never_skips() {
    let mut pacer = Dilation::new(DilationConfig::default());
    let slow = [ZoneTiming { zone: ZoneId(0), elapsed: 60_000 * MS }];
    for _ in 0..5 {
        assert_eq!(pacer.skip(&slow, Duration::ZERO), Vec::<ZoneId>::new());
    }
    // With a frame interval of one second the same zone is slowed: the
    // default budget is a share of the interval.
    assert_eq!(pacer.skip(&slow, Duration::from_secs(1)), [ZoneId(0)]);
}

/// A pacer for a runtime test: the real `Dilation`, fed with tick times
/// from a table instead of the clock. Zone 1 is heavy.
struct Scripted {
    feed: Feed<Dilation>,
    decisions: Arc<Mutex<Vec<Vec<u32>>>>,
}

impl Pacer for Scripted {
    fn skip(&mut self, timings: &[ZoneTiming], _tick_interval: Duration) -> Vec<ZoneId> {
        // The kernel reports one timing per zone from the second frame on.
        assert!(timings.is_empty() || timings.len() == 4);
        let skip = self.feed.frame(&[1, 30, 2, 1]);
        self.decisions.lock().expect("lock").push(skip.clone());
        skip.into_iter().map(ZoneId).collect()
    }

    fn dilated_zones(&self) -> usize {
        self.feed.pacer.dilated_zones()
    }
}

fn zone_ticks(world: &World) -> Vec<u64> {
    (0..4).map(|zone| world.realm(RealmId::Zone(ZoneId(zone))).expect("hosted").tick().0).collect()
}

#[tokio::test]
async fn dilation_slows_only_the_heavy_zone_and_a_replay_is_identical() {
    let storage = MemStorage::new();
    let durable = DurableConfig {
        sync: SyncPolicy::EveryBatch,
        snapshot_every: 5,
        ..DurableConfig::default()
    };
    let opened =
        DurableJournal::open(Arc::new(storage.clone()), plugins(), durable, common::new_world)
            .expect("the storage opens");
    let decisions = Arc::new(Mutex::new(Vec::new()));
    let config = RuntimeConfig {
        journal: Box::new(opened.journal),
        pacer: Box::new(Scripted { feed: Feed::new(pacer(8)), decisions: Arc::clone(&decisions) }),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(opened.world, config);
    play(&handle, 0, 20).await;
    let stats = handle.stats().await.expect("stats");
    let live = join.shutdown().expect("a clean stop");

    // Zone 1 costs three budgets: it ticks in one frame of three, after the
    // first frame, in which nothing was measured. The other zones tick in
    // every frame.
    assert_eq!(zone_ticks(&live), [20, 7, 20, 20]);
    assert_eq!((stats.dilated_zones, stats.zone_ticks_skipped), (1, 13));
    let decisions = decisions.lock().expect("lock").clone();
    assert!(decisions.iter().all(|skip| skip.is_empty() || skip == &[1]), "{decisions:?}");

    // The decisions are in the log, so a recovery and a replay skip the
    // same zone in the same frames without a pacer and without a clock.
    let recovery = recover(&storage, &[&Town::default()]).expect("the world recovers");
    assert_eq!(hash_of(&recovery.world), hash_of(&live));
    let report =
        replay(&storage, &[&Town::default()], &Default::default()).expect("the replay runs");
    assert_eq!((report.frames, report.frame), (20, 20));
    assert_eq!(common::hex(&report.state_hash), hash_of(&live));
    assert!(report.is_consistent() && report.checkpoints.len() == 4, "{report:?}");

    // A run with the same inputs and no dilation is a different world: the
    // skips are real.
    let (handle, join) = start(common::world(), RuntimeConfig::manual());
    play(&handle, 0, 20).await;
    let undilated = join.shutdown().expect("a clean stop");
    assert_eq!(zone_ticks(&undilated), [20, 20, 20, 20]);
    assert_ne!(hash_of(&undilated), hash_of(&live));
}

/// A journal that keeps the `skip` list of every frame.
struct Skips(Arc<Mutex<Vec<Vec<ZoneId>>>>);

impl Journal for Skips {
    fn record(&mut self, input: &Input) -> Result<(), JournalError> {
        if let Input::Step { skip } = input {
            self.0.lock().expect("lock").push(skip.clone());
        }
        Ok(())
    }

    fn frame_done(&mut self, _world: &World, _frame: &FrameOutput) {}
}

#[tokio::test]
async fn a_zone_that_is_really_slow_is_slowed_by_the_measured_time() {
    // Zone 0 burns processor time in every tick; the kernel measures it
    // with the clock of the runtime. The budget is far below that time and
    // far above the tick time of an empty zone, also on a busy machine.
    let data = r#"
        [params."town.heavy_zone"]
        value = 1
        [params."town.heavy_rounds"]
        value = 3000000
    "#;
    let config = WorldConfig { data: vec![data.to_string()], ..WorldConfig::with_seed(3) };
    let world = World::new(&[&Town::default()], config);
    let skips = Arc::new(Mutex::new(Vec::new()));
    let pacer = Dilation::new(DilationConfig {
        budget: Some(Duration::from_micros(50)),
        smoothing: 1,
        ..DilationConfig::default()
    });
    let config = RuntimeConfig {
        journal: Box::new(Skips(Arc::clone(&skips))),
        pacer: Box::new(pacer),
        ..RuntimeConfig::manual()
    };
    let (handle, join) = start(world, config);
    handle.step(40).await.expect("step");
    let stats = handle.stats().await.expect("stats");
    let world = join.shutdown().expect("a clean stop");

    // The heavy zone runs at the floor: one tick in ten frames, after the
    // first frames in which it was measured.
    let heavy = world.realm(RealmId::Zone(ZoneId(0))).expect("hosted").tick().0;
    assert!((4..=6).contains(&heavy), "zone 0 ticked {heavy} times in 40 frames");
    assert!(stats.dilated_zones >= 1 && stats.zone_ticks_skipped >= 34, "{stats:?}");
    // Every frame that skipped anything skipped zone 0.
    let skips = skips.lock().expect("lock");
    assert_eq!(skips.len(), 40);
    let with_zone_0 = skips.iter().filter(|skip| skip.contains(&ZoneId(0))).count() as u64;
    assert_eq!(with_zone_0, 40 - heavy);
}
