//! The `unsupervised` command.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use unsup_server::{ServerConfig, WorldShape};

/// Unsupervised: a persistent society played by AI agents.
#[derive(Parser)]
#[command(name = "unsupervised", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the simulation and the server.
    Run {
        /// Address to listen on.
        #[arg(long, env = "UNSUPERVISED_BIND", default_value = "127.0.0.1:8080")]
        bind: SocketAddr,
        /// Real milliseconds for one game minute.
        #[arg(long, env = "UNSUPERVISED_TICK_MS", default_value_t = 1000)]
        tick_ms: u64,
        /// Seed of the world.
        #[arg(long, default_value_t = 1)]
        seed: u64,
        /// Number of cities.
        #[arg(long, default_value_t = 2)]
        cities: u32,
        /// Districts on one side of a city.
        #[arg(long, default_value_t = 3)]
        districts: u32,
        /// Directory with the built viewer.
        #[arg(long, default_value = "web/dist")]
        web_dir: PathBuf,
    },
    /// Write the protocol schema and the action catalogue to a directory.
    ExportSchema {
        /// Target directory.
        #[arg(long, default_value = "schema")]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::Run { bind, tick_ms, seed, cities, districts, web_dir } => {
            let config = ServerConfig {
                bind,
                tick_interval: Duration::from_millis(tick_ms),
                world: WorldShape { seed, cities, districts },
                web_dir: Some(web_dir),
            };
            let server = unsup_server::start(config).await?;
            println!("Unsupervised listens on http://{}", server.addr);
            tokio::signal::ctrl_c().await.context("cannot wait for Ctrl-C")?;
            server.stop().await
        }
        Command::ExportSchema { out } => export_schema(&out),
    }
}

/// Writes `protocol.schema.json` and `catalogue.json`.
fn export_schema(out: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
    let world =
        unsup_server::build_world(WorldShape { cities: 1, districts: 1, seed: 1 }.to_config());
    let files = [
        ("protocol.schema.json", unsup_protocol::schema::wire_schema()),
        ("catalogue.json", serde_json::to_value(world.catalogue())?),
    ];
    for (name, value) in files {
        let path = out.join(name);
        let mut text = serde_json::to_string_pretty(&value)?;
        text.push('\n');
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}
