//! Bot swarm and benchmarks.

use std::time::Instant;

use clap::{Parser, Subcommand};
use unsup_kernel::Input;
use unsup_protocol::{ActionRequest, OwnerId, SpawnSpec};
use unsup_server::WorldShape;

/// Load tests and benchmarks for Unsupervised.
#[derive(Parser)]
#[command(name = "unsup-loadtest", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure frame time in one process, without the network.
    Bench {
        /// Number of residents.
        #[arg(long, default_value_t = 100_000)]
        residents: u32,
        /// Number of frames to measure.
        #[arg(long, default_value_t = 60)]
        frames: u32,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Bench { residents, frames } => bench(residents, frames),
    }
}

/// Builds a world with all modules, gives every resident a walk and prints
/// the frame times.
fn bench(residents: u32, frames: u32) -> anyhow::Result<()> {
    // About 2,000 residents for each zone.
    let zones = (residents / 2_000).max(1);
    let districts = zones.isqrt().clamp(1, 8);
    let cities = zones.div_ceil(districts * districts).max(1);
    let mut world =
        unsup_server::build_world(WorldShape { seed: 1, cities, districts }.to_config());

    let mut ids = Vec::with_capacity(residents as usize);
    for i in 0..residents {
        let spec =
            SpawnSpec { owner: OwnerId(1), name: format!("bot-{i}"), model: None, city: None };
        ids.push(world.spawn(spec).map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?);
    }
    let mut total = 0.0_f64;
    let mut worst = 0.0_f64;
    for frame in 0..frames {
        for (n, id) in ids.iter().enumerate() {
            if (n as u32 + frame).is_multiple_of(10) {
                let request = ActionRequest {
                    id: None,
                    verb: "wait".into(),
                    args: serde_json::json!({ "ticks": 5 }),
                };
                world.apply(Input::Act { resident: *id, request });
            }
        }
        let start = Instant::now();
        world.step();
        let ms = start.elapsed().as_secs_f64() * 1_000.0;
        total += ms;
        worst = worst.max(ms);
    }
    println!(
        "residents {residents}, zones {}, frames {frames}: mean {:.3} ms, worst {:.3} ms",
        world.layout().zones().len(),
        total / f64::from(frames.max(1)),
        worst
    );
    Ok(())
}
