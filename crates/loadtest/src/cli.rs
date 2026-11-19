//! The options of the command line.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use unsup_loadtest::bench::parse_size;
use unsup_loadtest::defaults::defaults;
use unsup_loadtest::local::Placement;
use unsup_loadtest::swarm::HttpConnections;
use unsup_loadtest::think::ThinkKind;

/// Load tests and benchmarks for Unsupervised.
///
/// See docs/performance.md for what each number means.
#[derive(Parser)]
#[command(name = "unsup-loadtest", version, about)]
pub struct Cli {
    /// The command.
    #[command(subcommand)]
    pub command: Command,
}

/// The commands.
#[derive(Subcommand)]
pub enum Command {
    /// Measure frame time, action rate and memory in one process, without
    /// the network.
    Bench(BenchArgs),
    /// Run for many ticks in one process and check conservation and
    /// determinism. The exit code is not zero after a violation.
    Soak(SoakArgs),
    /// Run many virtual agents over the network against a server and
    /// measure the acknowledgement latency, the throughput and the errors.
    Swarm(SwarmArgs),
}

/// Options that `bench` and `soak` share: the world and the behaviour.
#[derive(Args)]
pub struct WorldArgs {
    /// Seed of the world and of the behaviour of the residents.
    #[arg(long, default_value_t = defaults().world.seed)]
    pub seed: u64,
    /// Number of cities. Without it the tool chooses, for about 2,000
    /// residents in each zone.
    #[arg(long)]
    pub cities: Option<u32>,
    /// Districts on one side of a city. Without it the tool chooses.
    #[arg(long)]
    pub districts: Option<u32>,
    /// Where the residents start: `scatter` (random walkable tiles of
    /// their city, like a world that has run for a while) or `spawn` (the
    /// shelters and stations where the game puts new residents).
    #[arg(long, default_value_t = Placement::Scatter)]
    pub placement: Placement,
    /// Behaviour profile: a built-in name (walkers, rush, daily_life, still,
    /// sprint) or the path of a TOML file.
    #[arg(long, default_value = "walkers")]
    pub profile: String,
    /// Send an idempotency key with every request, as a careful client
    /// does. The kernel then remembers the last keys of each resident.
    #[arg(long)]
    pub request_ids: bool,
    /// A TOML file that replaces numbers of the game, in the format of a
    /// module's data/balance.toml. Use it to try another value of a
    /// parameter. It can be given more than once.
    #[arg(long)]
    pub data: Vec<PathBuf>,
}

impl WorldArgs {
    /// Returns the options as arguments for a child process.
    fn to_args(&self) -> Vec<String> {
        let mut args = vec![
            "--seed".to_string(),
            self.seed.to_string(),
            "--placement".to_string(),
            self.placement.to_string(),
            "--profile".to_string(),
            self.profile.clone(),
        ];
        if let Some(cities) = self.cities {
            args.extend(["--cities".to_string(), cities.to_string()]);
        }
        if let Some(districts) = self.districts {
            args.extend(["--districts".to_string(), districts.to_string()]);
        }
        if self.request_ids {
            args.push("--request-ids".to_string());
        }
        for path in &self.data {
            args.extend(["--data".to_string(), path.display().to_string()]);
        }
        args
    }
}

/// A list of world sizes, for `--scale`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scale(pub Vec<u32>);

impl std::str::FromStr for Scale {
    type Err = String;

    /// Reads a list such as `10000,100000,1000000` or `10k,100k,1M`.
    fn from_str(text: &str) -> Result<Scale, String> {
        let sizes: Vec<u32> = text
            .split(',')
            .filter(|part| !part.trim().is_empty())
            .map(parse_size)
            .collect::<Result<_, _>>()?;
        if sizes.is_empty() { Err("the scale list is empty".to_string()) } else { Ok(Scale(sizes)) }
    }
}

/// Options of `bench`.
#[derive(Args)]
pub struct BenchArgs {
    /// Number of residents.
    #[arg(long, default_value_t = defaults().bench.residents, value_parser = parse_size)]
    pub residents: u32,
    /// Several sizes in a row, for example `10000,100000,1000000` or
    /// `10k,100k,1M`. Each size runs in a process of its own.
    #[arg(long, conflicts_with = "residents")]
    pub scale: Option<Scale>,
    /// Frames that are measured.
    #[arg(long, default_value_t = defaults().bench.frames)]
    pub frames: u32,
    /// Frames that run before the measurement.
    #[arg(long, default_value_t = defaults().bench.warmup_frames)]
    pub warmup: u32,
    /// With `--scale`: stop a size that takes longer than this many
    /// seconds, and go on with the next size.
    #[arg(long)]
    pub time_limit_secs: Option<u64>,
    /// Stop the run when the process holds more than this many gibibytes
    /// (resident, compressed and swapped memory). Default: 80 % of the
    /// installed memory or 90 % of the free memory, the smaller of the two.
    #[arg(long)]
    pub max_memory_gib: Option<f64>,
    /// The sections that a resident asks for in an observation, with
    /// commas between them, for example `nearby`. Use it to measure the
    /// cost of one section. Without it, all sections.
    #[arg(long, value_delimiter = ',')]
    pub observe_sections: Vec<String>,
    /// Build the observations of one frame with `World::observe_batch` on
    /// all processor cores, at most this many requests in one call. The
    /// server does that for the `observe` calls of one batch of its
    /// mailbox, which holds at most 256 calls. 0: one `World::observe` for
    /// each observation, on one thread.
    #[arg(long, default_value_t = defaults().bench.observe_batch)]
    pub observe_batch: usize,
    /// Path of the report file. Default: var/loadtest/bench-<time>.json.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The world and the behaviour.
    #[command(flatten)]
    pub world: WorldArgs,
    /// This process is one size of a `--scale` run. It prints no summary.
    #[arg(long, hide = true)]
    pub child: bool,
}

impl BenchArgs {
    /// Returns the options that every size of a `--scale` run shares, as
    /// arguments for the child processes.
    pub fn shared_args(&self) -> Vec<String> {
        let mut args = vec![
            "--frames".to_string(),
            self.frames.to_string(),
            "--warmup".to_string(),
            self.warmup.to_string(),
        ];
        if let Some(gib) = self.max_memory_gib {
            args.extend(["--max-memory-gib".to_string(), gib.to_string()]);
        }
        if !self.observe_sections.is_empty() {
            args.extend(["--observe-sections".to_string(), self.observe_sections.join(",")]);
        }
        args.extend(["--observe-batch".to_string(), self.observe_batch.to_string()]);
        args.extend(self.world.to_args());
        args
    }
}

/// Options of `soak`.
#[derive(Args)]
pub struct SoakArgs {
    /// Number of residents.
    #[arg(long, default_value_t = defaults().soak.residents, value_parser = parse_size)]
    pub residents: u32,
    /// Number of ticks.
    #[arg(long, default_value_t = defaults().soak.ticks)]
    pub ticks: u64,
    /// Ticks between two audits of the whole world.
    #[arg(long, default_value_t = defaults().soak.audit_every)]
    pub audit_every: u64,
    /// Run a second world with the same seed and inputs, and compare the
    /// state hashes of the two worlds.
    #[arg(long)]
    pub verify_determinism: bool,
    /// Ticks between two comparisons of the state hash.
    #[arg(long, default_value_t = defaults().soak.hash_every)]
    pub hash_every: u64,
    /// Path of the report file. Default: var/loadtest/soak-<time>.json.
    #[arg(long)]
    pub out: Option<PathBuf>,
    /// The world and the behaviour.
    #[command(flatten)]
    pub world: WorldArgs,
}

/// Options of `swarm`.
#[derive(Args)]
pub struct SwarmArgs {
    /// Address of a server that runs, for example http://127.0.0.1:8080.
    #[arg(long, required_unless_present = "spawn_server", conflicts_with = "spawn_server")]
    pub url: Option<String>,
    /// Start the server of the project in this process on a free port and
    /// run against it.
    #[arg(long)]
    pub spawn_server: bool,
    /// Number of agents.
    #[arg(long, default_value_t = defaults().swarm.agents, value_parser = parse_size)]
    pub agents: u32,
    /// Seconds over which the agents start, one after the other.
    #[arg(long, default_value_t = defaults().swarm.ramp_up_secs)]
    pub ramp_up: f64,
    /// Seconds of the measurement window after the ramp-up.
    #[arg(long, default_value_t = defaults().swarm.duration_secs)]
    pub duration: f64,
    /// Distribution of the think time between two requests of an agent:
    /// constant, exponential or lognormal.
    #[arg(long, default_value = defaults().swarm.think.as_str())]
    pub think: ThinkKind,
    /// Mean think time in seconds.
    #[arg(long, default_value_t = defaults().swarm.think_mean_secs)]
    pub think_mean: f64,
    /// Shape of the log-normal think time: the standard deviation of the
    /// logarithm.
    #[arg(long, default_value_t = defaults().swarm.think_sigma)]
    pub think_sigma: f64,
    /// Share of the agents that use the WebSocket, from 0 to 1. The others
    /// send HTTP requests.
    #[arg(long, default_value_t = defaults().swarm.ws_share)]
    pub ws_share: f64,
    /// How the HTTP agents hold connections: `per-agent` (each agent keeps
    /// one connection open, like a real agent) or `shared` (one pool for
    /// all, for more agents than the machine has ports).
    #[arg(long, default_value_t = HttpConnections::PerAgent)]
    pub http_connections: HttpConnections,
    /// The HTTP agents send `POST /v1/act?observe=false`: the answer has
    /// only the acknowledgements, and the server builds no observation
    /// for it. An agent then looks at the world with the chance
    /// `observe_share` of the profile. Use it to find the greatest rate of
    /// actions.
    #[arg(long)]
    pub acks_only: bool,
    /// Behaviour profile: a built-in name (walkers, rush, daily_life, still,
    /// sprint) or the path of a TOML file.
    #[arg(long, default_value = "walkers")]
    pub profile: String,
    /// Seed of the choices of the agents, and of the world of a server
    /// that the tool starts.
    #[arg(long, default_value_t = defaults().world.seed)]
    pub seed: u64,
    /// File with the keys of the residents. Keys in the file are used
    /// again; the keys of this run are written to it. Keep it private.
    #[arg(long)]
    pub credentials: Option<PathBuf>,
    /// Seconds after which a request counts as failed.
    #[arg(long, default_value_t = defaults().swarm.request_timeout_secs)]
    pub request_timeout: f64,
    /// With `--spawn-server`: real milliseconds for one game minute.
    #[arg(long, default_value_t = defaults().swarm.tick_ms)]
    pub tick_ms: u64,
    /// With `--spawn-server`: number of cities. Without it the tool
    /// chooses, for about 2,000 residents in each zone.
    #[arg(long)]
    pub cities: Option<u32>,
    /// With `--spawn-server`: districts on one side of a city.
    #[arg(long)]
    pub districts: Option<u32>,
    /// The run is sustained when the 99th percentile of the
    /// acknowledgement latency is at or below this many milliseconds.
    #[arg(long, default_value_t = defaults().swarm.p99_target_ms)]
    pub p99_target_ms: f64,
    /// The run is sustained when at most this share of the requests
    /// failed.
    #[arg(long, default_value_t = defaults().swarm.max_error_share)]
    pub max_error_share: f64,
    /// Make the exit code not zero when the load was not sustained.
    #[arg(long)]
    pub require_sustained: bool,
    /// Path of a route with numbers of the server that needs no token.
    /// The tool reads it at the start and at the end of the window.
    #[arg(long, default_value = defaults().swarm.stats_path.as_str())]
    pub stats_path: String,
    /// Path of the report file. Default: var/loadtest/swarm-<time>.json.
    #[arg(long)]
    pub out: Option<PathBuf>,
}
