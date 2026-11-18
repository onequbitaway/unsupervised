//! One benchmark run: one world size in this process.

use std::time::{Duration, Instant};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use unsup_kernel::Plugin;
use unsup_protocol::RealmId;

use super::memory::{Memory, Probe};
use crate::defaults::defaults;
use crate::local::{ActionCounts, Driver, DriverOptions, Placement, Shape, WorldSpec, populate};
use crate::machine;
use crate::profile::{Profile, Resolved};
use crate::report::thousands;
use crate::stats::Millis;

/// What to measure.
#[derive(Clone, Debug)]
pub struct BenchConfig {
    /// The world.
    pub world: WorldSpec,
    /// Frames that are measured.
    pub frames: u32,
    /// Frames that run before the measurement.
    pub warmup: u32,
    /// What the residents do.
    pub profile: Profile,
    /// True: every request carries an idempotency key.
    pub request_ids: bool,
    /// The run stops when the process holds more than this many bytes: the
    /// larger of its resident set size and its footprint. `None`: the
    /// limit of the data file
    /// ([`default_memory_limit`](super::default_memory_limit)).
    pub max_memory_bytes: Option<u64>,
}

/// How a run used its profile.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileUse {
    /// Name of the profile.
    pub name: String,
    /// The verbs that the residents used, each with its weight.
    pub verbs: Vec<(String, u32)>,
    /// Verbs of the profile that the world does not have.
    pub skipped: Vec<String>,
    /// Messages about the profile for the operator.
    pub warnings: Vec<String>,
}

impl ProfileUse {
    /// Describes a resolved profile.
    #[must_use]
    pub fn of(profile: &Resolved) -> ProfileUse {
        ProfileUse {
            name: profile.name.clone(),
            verbs: profile.verbs().into_iter().map(|(verb, w)| (verb.to_string(), w)).collect(),
            skipped: profile.skipped.clone(),
            warnings: profile.warnings.clone(),
        }
    }

    /// Returns one line for the summary.
    #[must_use]
    pub fn line(&self) -> String {
        let verbs: Vec<String> =
            self.verbs.iter().map(|(verb, weight)| format!("{verb} x{weight}")).collect();
        let skipped = if self.skipped.is_empty() {
            String::new()
        } else {
            format!("; skipped (not in this world): {}", self.skipped.join(", "))
        };
        format!("profile {}: {}{skipped}", self.name, verbs.join(", "))
    }
}

/// The result of one run.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchRun {
    /// Number of residents at the start.
    pub residents: u64,
    /// Number of residents after the last frame.
    pub residents_end: u64,
    /// Number of cities.
    pub cities: u32,
    /// Districts on one side of a city.
    pub districts: u32,
    /// Number of zones.
    pub zones: u64,
    /// Where the residents started.
    pub placement: Placement,
    /// Spawn sites that the game made. With none, every resident of a city
    /// starts on one tile unless the placement is `scatter`.
    pub spawn_sites: u64,
    /// Residents in the fullest zone after the last frame.
    pub fullest_zone: u64,
    /// The profile.
    pub profile: ProfileUse,
    /// True when every request carried an idempotency key.
    pub request_ids: bool,
    /// Seconds to build the empty world.
    pub build_secs: f64,
    /// Seconds to create and place the residents.
    pub populate_secs: f64,
    /// Frames before the measurement.
    pub warmup_frames: u32,
    /// Frames that were measured.
    pub frames: u32,
    /// Time of one frame (`World::step`): all zones tick, then the global
    /// realm, then the router.
    pub frame_ms: Millis,
    /// Time to apply the action requests of one frame (`World::apply`).
    pub inputs_ms: Millis,
    /// Time to build the observations of one frame (`World::observe`).
    pub observe_ms: Millis,
    /// `inputs + observe + frame`: the work of the simulation thread of a
    /// server for one frame.
    pub server_ms: Millis,
    /// Time that the load driver itself needed for one frame: it chooses
    /// actions and makes arguments. A server does not do this work.
    pub generator_ms: Millis,
    /// What the residents did in the measured frames.
    pub actions: ActionCounts,
    /// Accepted actions for each measured frame.
    pub actions_per_frame: f64,
    /// Accepted actions for each second of wall time of the measured loop,
    /// with the time of the load driver.
    pub actions_per_wall_second: f64,
    /// Accepted actions for each second of server work (`server_ms`).
    pub actions_per_server_second: f64,
    /// Position changes for each measured frame.
    pub moves_per_frame: f64,
    /// Residents that changed zone for each measured frame.
    pub migrations_per_frame: f64,
    /// Wall time of the measured loop in seconds.
    pub wall_secs: f64,
    /// Memory of the process.
    pub memory: Memory,
}

impl BenchRun {
    /// Returns the summary of the run.
    #[must_use]
    pub fn summary(&self) -> String {
        let line = |name: &str, t: &Millis| {
            format!(
                "  {name:<10} mean {:>9.3} ms   p50 {:>9.3}   p99 {:>9.3}   worst {:>9.3}\n",
                t.mean, t.p50, t.p99, t.max
            )
        };
        let shape = Shape { cities: self.cities, districts: self.districts };
        let mut out = format!(
            "{} residents in {} ({}), placement {}, fullest zone {}\n",
            thousands(self.residents),
            Shape::zones_text(self.zones),
            shape.describe(),
            self.placement,
            thousands(self.fullest_zone),
        );
        out.push_str(&format!("  {}\n", self.profile.line()));
        out.push_str(&format!(
            "  built in {:.2} s, {} residents created in {:.2} s; {} frames measured after {} warm-up frames\n",
            self.build_secs,
            thousands(self.residents),
            self.populate_secs,
            self.frames,
            self.warmup_frames
        ));
        out.push_str(&line("frame", &self.frame_ms));
        out.push_str(&line("inputs", &self.inputs_ms));
        out.push_str(&line("observe", &self.observe_ms));
        out.push_str(&line("server", &self.server_ms));
        out.push_str(&line("generator", &self.generator_ms));
        out.push_str(&format!(
            "  actions: {} accepted, {} rejected, {} observations; {} for each frame\n",
            thousands(self.actions.accepted()),
            thousands(self.actions.rejected),
            thousands(self.actions.observes),
            thousands(self.actions_per_frame.round() as u64),
        ));
        out.push_str(&format!(
            "  actions for each second: {} of wall time, {} of server work\n",
            thousands(self.actions_per_wall_second.round() as u64),
            thousands(self.actions_per_server_second.round() as u64),
        ));
        if !self.actions.rejected_by_code.is_empty() {
            out.push_str(&format!("  rejections by code: {:?}\n", self.actions.rejected_by_code));
        }
        out.push_str(&format!(
            "  moves for each frame {:.0}, zone changes for each frame {:.0}\n",
            self.moves_per_frame, self.migrations_per_frame
        ));
        out.push_str(&self.memory.lines());
        out.push_str(&format!(
            "  at one frame for each second, the server work of the slowest frame uses {:.1} % of the second\n",
            self.server_ms.max / 10.0
        ));
        for warning in &self.profile.warnings {
            out.push_str(&format!("  warning: {warning}\n"));
        }
        out
    }
}

/// Prints how far a long run is, at most once in each period.
struct Progress {
    every: Duration,
    total_frames: u64,
    last: Instant,
}

impl Progress {
    fn new(every: Duration, total_frames: u64) -> Progress {
        Progress { every, total_frames, last: Instant::now() }
    }

    /// Call it before each frame, with the number of frames that are done.
    fn frame(&mut self, frames_done: u64) {
        if self.last.elapsed() >= self.every {
            self.last = Instant::now();
            eprintln!("bench: {frames_done} of {} frames", self.total_frames);
        }
    }
}

fn nanos(d: Duration) -> u64 {
    u64::try_from(d.as_nanos()).unwrap_or(u64::MAX)
}

fn rate(count: u64, seconds: f64) -> f64 {
    if seconds > 0.0 { count as f64 / seconds } else { 0.0 }
}

/// Runs one benchmark. `extra` plugins are added to the game modules of the
/// server; tests use them.
///
/// It prints its progress to the standard error stream.
///
/// # Errors
///
/// Fails when the world cannot be built, when the world has none of the
/// verbs of the profile, or when the ledgers do not balance after the run.
pub fn run(config: &BenchConfig, extra: &[&dyn Plugin]) -> anyhow::Result<BenchRun> {
    let d = &defaults().bench;
    let mut probe = Probe::start(config.max_memory_bytes);
    let total_frames = u64::from(config.warmup) + u64::from(config.frames);
    let mut progress = Progress::new(Duration::from_secs(d.progress_secs.max(1)), total_frames);

    eprintln!(
        "bench: building a world for {} residents ({})",
        thousands(u64::from(config.world.residents)),
        config.world.shape.describe()
    );
    let population = populate(&config.world, extra)?;
    let (build_time, populate_time, spawn_sites) =
        (population.build_time, population.populate_time, population.spawn_sites);
    let mut world = population.world;
    let residents = population.ids.len() as u64;
    let profile = config.profile.resolve(&world.catalogue())?;
    for warning in &profile.warnings {
        eprintln!("bench: warning: {warning}");
    }
    let profile_use = ProfileUse::of(&profile);
    let options = DriverOptions { seed: config.world.seed, request_ids: config.request_ids };
    let mut driver = Driver::new(&world, population.ids, profile, options);
    probe.populated();
    eprintln!(
        "bench: {} residents ready after {:.2} s; {} warm-up frames, then {} measured frames{}",
        thousands(residents),
        (build_time + populate_time).as_secs_f64(),
        config.warmup,
        config.frames,
        probe.limit().map_or_else(String::new, |limit| format!(
            "; memory limit {:.1} GiB",
            limit as f64 / machine::GIB
        ))
    );

    for frame in 0..config.warmup {
        probe.check(u64::from(frame), total_frames)?;
        progress.frame(u64::from(frame));
        driver.turn(&mut world);
        world.step();
    }
    driver.take_counts();
    probe.warm();

    let frames = config.frames as usize;
    let mut frame_ns = Vec::with_capacity(frames);
    let mut inputs_ns = Vec::with_capacity(frames);
    let mut observe_ns = Vec::with_capacity(frames);
    let mut generator_ns = Vec::with_capacity(frames);
    let mut server_ns = Vec::with_capacity(frames);
    let (mut moves, mut migrations) = (0u64, 0u64);
    let loop_start = Instant::now();
    for frame in 0..frames {
        let frames_done = u64::from(config.warmup) + frame as u64;
        probe.check(frames_done, total_frames)?;
        progress.frame(frames_done);
        let t0 = Instant::now();
        driver.plan(&world);
        let t1 = Instant::now();
        driver.apply(&mut world);
        let t2 = Instant::now();
        driver.observe(&world);
        let t3 = Instant::now();
        let output = world.step();
        let t4 = Instant::now();
        generator_ns.push(nanos(t1 - t0));
        inputs_ns.push(nanos(t2 - t1));
        observe_ns.push(nanos(t3 - t2));
        frame_ns.push(nanos(t4 - t3));
        server_ns.push(nanos(t4 - t1));
        for zone in &output.zones {
            moves += zone.moved.len() as u64;
            migrations += zone.left.len() as u64;
        }
    }
    let wall = loop_start.elapsed().as_secs_f64();
    let actions = driver.take_counts();

    world.audit().map_err(|error| anyhow!("the world does not balance after the run: {error}"))?;
    let fullest_zone = world
        .layout()
        .zones()
        .iter()
        .filter_map(|zone| world.realm(RealmId::Zone(zone.id)))
        .map(|realm| realm.len() as u64)
        .max()
        .unwrap_or(0);
    let memory = probe.finish(&world, driver.residents());
    let server = Millis::from_nanos(&server_ns);
    let server_secs = server.mean * server.count as f64 / 1e3;
    let frames_f = f64::from(config.frames.max(1));
    Ok(BenchRun {
        residents,
        residents_end: world.residents() as u64,
        cities: config.world.shape.cities,
        districts: config.world.shape.districts,
        zones: world.layout().zones().len() as u64,
        placement: config.world.placement,
        spawn_sites,
        fullest_zone,
        profile: profile_use,
        request_ids: config.request_ids,
        build_secs: build_time.as_secs_f64(),
        populate_secs: populate_time.as_secs_f64(),
        warmup_frames: config.warmup,
        frames: config.frames,
        frame_ms: Millis::from_nanos(&frame_ns),
        inputs_ms: Millis::from_nanos(&inputs_ns),
        observe_ms: Millis::from_nanos(&observe_ns),
        server_ms: server,
        generator_ms: Millis::from_nanos(&generator_ns),
        actions_per_frame: actions.accepted() as f64 / frames_f,
        actions_per_wall_second: rate(actions.accepted(), wall),
        actions_per_server_second: rate(actions.accepted(), server_secs),
        moves_per_frame: moves as f64 / frames_f,
        migrations_per_frame: migrations as f64 / frames_f,
        actions,
        wall_secs: wall,
        memory,
    })
}
