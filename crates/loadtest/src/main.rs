//! The `unsup-loadtest` command: `bench`, `soak` and `swarm`.

mod cli;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Parser;
use serde::Serialize;
use unsup_loadtest::bench::{self, BenchConfig, BenchReport, ScaleFailure, ScaleOptions};
use unsup_loadtest::defaults::defaults;
use unsup_loadtest::local::{Shape, WorldSpec};
use unsup_loadtest::machine::GIB;
use unsup_loadtest::profile::Profile;
use unsup_loadtest::report::{DEFAULT_DIR, Head};
use unsup_loadtest::soak::{self, SoakConfig};
use unsup_loadtest::swarm::{self, SpawnServer, SwarmConfig, Target, Timing};
use unsup_loadtest::think::ThinkTime;

use crate::cli::{BenchArgs, Cli, Command, SoakArgs, SwarmArgs, WorldArgs};

fn main() -> anyhow::Result<()> {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match Cli::parse().command {
        Command::Bench(args) => bench_command(&args, argv),
        Command::Soak(args) => soak_command(&args, argv),
        Command::Swarm(args) => swarm_command(&args, argv),
    }
}

/// Ends a command: prints the summary, writes the report file and says
/// where the file is. Without a summary it only writes the file.
fn finish<T: Serialize>(
    head: Head,
    out: Option<&PathBuf>,
    result: T,
    summary: Option<String>,
) -> anyhow::Result<()> {
    let path = out.cloned().unwrap_or_else(|| head.default_path());
    let report = head.with(result);
    if let Some(summary) = &summary {
        print!("\n{}{summary}", report.head_lines());
    }
    report.write(&path)?;
    if summary.is_some() {
        println!("report: {}", path.display());
    }
    Ok(())
}

fn world_spec(args: &WorldArgs, residents: u32) -> anyhow::Result<WorldSpec> {
    let data = args
        .data
        .iter()
        .map(|path| {
            std::fs::read_to_string(path)
                .with_context(|| format!("cannot read the data file {}", path.display()))
        })
        .collect::<anyhow::Result<Vec<String>>>()?;
    Ok(WorldSpec {
        seed: args.seed,
        residents,
        shape: Shape::choose(residents, args.cities, args.districts),
        placement: args.placement,
        data,
    })
}

fn bench_command(args: &BenchArgs, argv: Vec<String>) -> anyhow::Result<()> {
    let head = Head::now("bench", argv);
    let report = match &args.scale {
        Some(sizes) => {
            let options = ScaleOptions {
                shared_args: args.shared_args(),
                time_limit: args.time_limit_secs.map(Duration::from_secs),
                work_dir: args
                    .out
                    .as_ref()
                    .and_then(|path| path.parent().map(PathBuf::from))
                    .filter(|dir| !dir.as_os_str().is_empty())
                    .unwrap_or_else(|| PathBuf::from(DEFAULT_DIR)),
            };
            bench::run_scale(&sizes.0, &options)?
        }
        None => {
            let config = BenchConfig {
                world: world_spec(&args.world, args.residents)?,
                frames: args.frames,
                warmup: args.warmup,
                profile: Profile::load(&args.world.profile)?,
                request_ids: args.world.request_ids,
                observe_sections: args.observe_sections.clone(),
                observe_batch: args.observe_batch,
                // A limit that is not a usable number is no limit of the
                // user; the default applies.
                max_memory_bytes: args
                    .max_memory_gib
                    .filter(|gib| gib.is_finite() && *gib > 0.0)
                    .map(|gib| (gib * GIB) as u64),
            };
            // A run that stops is a result too: the report names the size
            // and the reason.
            match bench::run(&config, &[]) {
                Ok(run) => BenchReport { runs: vec![run], failures: Vec::new() },
                Err(error) => {
                    eprintln!("bench: {error:#}");
                    BenchReport {
                        runs: Vec::new(),
                        failures: vec![ScaleFailure {
                            residents: u64::from(args.residents),
                            reason: format!("{error:#}"),
                        }],
                    }
                }
            }
        }
    };
    let failed = report.failures.len();
    // A child prints nothing: its file is a hand-over to its parent, which
    // prints the summary of all sizes and removes the file.
    let summary = (!args.child).then(|| report.summary());
    finish(head, args.out.as_ref(), report, summary)?;
    if failed > 0 {
        bail!("{failed} of the sizes did not finish; the report names them");
    }
    Ok(())
}

fn soak_command(args: &SoakArgs, argv: Vec<String>) -> anyhow::Result<()> {
    let head = Head::now("soak", argv);
    let config = SoakConfig {
        world: world_spec(&args.world, args.residents)?,
        ticks: args.ticks,
        profile: Profile::load(&args.world.profile)?,
        audit_every: args.audit_every,
        verify_determinism: args.verify_determinism,
        hash_every: args.hash_every,
        request_ids: args.world.request_ids,
    };
    eprintln!(
        "soak: {} ticks with {} residents{}",
        config.ticks,
        config.world.residents,
        if config.verify_determinism { ", two identical runs side by side" } else { "" }
    );
    let run = soak::run(&config, &[])?;
    let (verdict, summary) = (run.verdict(), run.summary());
    finish(head, args.out.as_ref(), run, Some(summary))?;
    verdict.context("the soak run found a violation")
}

/// Turns a number of seconds from the command line into a duration.
fn seconds(name: &str, value: f64) -> anyhow::Result<Duration> {
    Duration::try_from_secs_f64(value)
        .map_err(|_| anyhow::anyhow!("--{name} is {value}; it must be a number of seconds from 0"))
}

fn swarm_command(args: &SwarmArgs, argv: Vec<String>) -> anyhow::Result<()> {
    let d = &defaults().swarm;
    let head = Head::now("swarm", argv);
    if !(0.0..=1.0).contains(&args.ws_share) {
        bail!("--ws-share is {}; it must be from 0 to 1", args.ws_share);
    }
    let target = match &args.url {
        Some(url) => Target::Url(url.clone()),
        None => Target::Spawn(SpawnServer {
            seed: args.seed,
            shape: Shape::choose(args.agents, args.cities, args.districts),
            tick_ms: args.tick_ms,
        }),
    };
    let config = SwarmConfig {
        target,
        agents: args.agents,
        ramp_up: seconds("ramp-up", args.ramp_up)?,
        duration: seconds("duration", args.duration)?,
        think: ThinkTime::new(args.think, args.think_mean, args.think_sigma)
            .map_err(|message| anyhow::anyhow!(message))?,
        ws_share: args.ws_share,
        http_connections: args.http_connections,
        acks_only: args.acks_only,
        profile: Profile::load(&args.profile)?,
        seed: args.seed,
        credentials: args.credentials.clone(),
        timing: Timing {
            request_timeout: seconds("request-timeout", args.request_timeout)?,
            retry: seconds("retry", d.retry_secs)?,
            max_failures: d.max_failures_in_row,
            wait_timeout: seconds("wait-timeout", d.wait_timeout_secs)?,
        },
        p99_target: seconds("p99-target-ms", args.p99_target_ms / 1e3)?,
        max_error_share: args.max_error_share,
        stats_path: args.stats_path.clone(),
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("cannot start the async runtime")?;
    let run = runtime.block_on(swarm::run(&config))?;
    let (sustained, summary) = (run.sustained, run.summary());
    let machine_limit = run.machine_limit.clone();
    finish(head, args.out.as_ref(), run, Some(summary))?;
    if let Some(limit) = machine_limit {
        bail!(limit);
    }
    if args.require_sustained && !sustained {
        bail!("the server did not sustain the load; the summary gives the reasons");
    }
    Ok(())
}
