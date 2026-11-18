//! The `unsupervised` command.

mod admin_cli;
mod replay_cli;

use std::io::IsTerminal;
use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context;
use clap::{Args, Parser, Subcommand, ValueEnum};
use unsup_server::{
    Overrides, Server, ServerConfig, StoreConfig, StoreKind, WorldShape, WorldStoreKind,
};

/// Unsupervised: a persistent society played by AI agents.
#[derive(Parser)]
#[command(name = "unsupervised", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the simulation and the server.
    Run(Box<RunArgs>),
    /// Check a data directory: replay the input log with all game modules,
    /// compare the state hashes of the snapshots and audit the money and
    /// the goods. The server that uses the directory must not run.
    Replay(replay_cli::ReplayArgs),
    /// Call an admin route of a running server.
    Admin(admin_cli::AdminArgs),
    /// Write the protocol schema and the action catalogue to a directory.
    ExportSchema {
        /// Target directory.
        #[arg(long, default_value = "schema")]
        out: PathBuf,
    },
}

/// Where accounts and keys live.
#[derive(Clone, Copy, ValueEnum)]
enum StoreArg {
    /// One SQLite file in the data directory.
    Sqlite,
    /// Memory only.
    Memory,
}

/// Where the world lives between two runs.
#[derive(Clone, Copy, ValueEnum)]
enum WorldStoreArg {
    /// The directory `world` in the data directory.
    Disk,
    /// Memory only.
    Memory,
}

/// The options of `run`. Each replaces the value of the configuration
/// file. A flag wins over its environment variable.
#[derive(Args)]
struct RunArgs {
    /// Configuration file in TOML.
    #[arg(long, env = "UNSUPERVISED_CONFIG")]
    config: Option<PathBuf>,
    /// Development mode: fast ticks, a store in memory, and a ready owner
    /// token and admin token on the terminal.
    #[arg(long)]
    dev: bool,
    /// Address to listen on.
    #[arg(long, env = "UNSUPERVISED_BIND")]
    bind: Option<SocketAddr>,
    /// Real milliseconds for one game minute.
    #[arg(long, env = "UNSUPERVISED_TICK_MS")]
    tick_ms: Option<u64>,
    /// Seed of the world.
    #[arg(long, env = "UNSUPERVISED_SEED")]
    seed: Option<u64>,
    /// Number of cities.
    #[arg(long, env = "UNSUPERVISED_CITIES")]
    cities: Option<u32>,
    /// Districts on one side of a city.
    #[arg(long, env = "UNSUPERVISED_DISTRICTS")]
    districts: Option<u32>,
    /// Directory with the built viewer.
    #[arg(long, env = "UNSUPERVISED_WEB_DIR")]
    web_dir: Option<PathBuf>,
    /// Directory for the data that the server writes: the accounts and
    /// the world.
    #[arg(long, env = "UNSUPERVISED_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// Where accounts and keys live.
    #[arg(long, env = "UNSUPERVISED_STORE", value_enum)]
    store: Option<StoreArg>,
    /// Where the world lives between two runs.
    #[arg(long, env = "UNSUPERVISED_WORLD_STORE", value_enum)]
    world_store: Option<WorldStoreArg>,
    /// A file with parameter overrides in TOML, for a new world. Give the
    /// option once for each file; a later file wins. In the environment
    /// variable, put a comma between two files.
    #[arg(
        long = "params",
        env = "UNSUPERVISED_PARAMS",
        value_name = "FILE",
        value_delimiter = ','
    )]
    params: Vec<PathBuf>,
    /// Only after a start failed because one input of the log makes the
    /// simulation panic: drop that input and all later ones from the log.
    #[arg(long, value_name = "SEQUENCE")]
    drop_from: Option<u64>,
    /// The file that `GET /join.md` serves.
    #[arg(long, env = "UNSUPERVISED_JOIN_FILE")]
    join_file: Option<PathBuf>,
    /// The token of the administrator, at least 16 characters. Prefer the
    /// environment variable: other users of the machine can read a flag.
    #[arg(long, env = "UNSUPERVISED_ADMIN_TOKEN", hide_env_values = true)]
    admin_token: Option<String>,
    /// The client id of the OAuth application for the GitHub sign-in.
    #[arg(long, env = "UNSUPERVISED_GITHUB_CLIENT_ID")]
    github_client_id: Option<String>,
    /// The client secret of the OAuth application for the GitHub sign-in.
    /// Prefer the environment variable: other users of the machine can
    /// read a flag.
    #[arg(long, env = "UNSUPERVISED_GITHUB_CLIENT_SECRET", hide_env_values = true)]
    github_client_secret: Option<String>,
}

impl RunArgs {
    fn into_config(self) -> anyhow::Result<ServerConfig> {
        let overrides = Overrides {
            dev: self.dev,
            bind: self.bind,
            tick_ms: self.tick_ms,
            seed: self.seed,
            cities: self.cities,
            districts: self.districts,
            web_dir: self.web_dir,
            data_dir: self.data_dir,
            store: self.store.map(|store| match store {
                StoreArg::Sqlite => StoreKind::Sqlite,
                StoreArg::Memory => StoreKind::Memory,
            }),
            world_store: self.world_store.map(|store| match store {
                WorldStoreArg::Disk => WorldStoreKind::Disk,
                WorldStoreArg::Memory => WorldStoreKind::Memory,
            }),
            params: self.params,
            drop_from: self.drop_from,
            join_file: self.join_file,
            admin_token: self.admin_token,
            github_client_id: self.github_client_id,
            github_client_secret: self.github_client_secret,
        };
        ServerConfig::load(self.config.as_deref(), overrides)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // The log goes to the error stream. The output stream has only what
    // the command prints for its user: the address and, in development
    // mode, the tokens. Colours are for a terminal, not for a log file.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    match Cli::parse().command {
        Command::Run(args) => {
            let config = args.into_config()?;
            let store = config.store.clone();
            let server = unsup_server::start(config).await?;
            announce(&server, &store);
            wait_for_stop_signal().await?;
            println!("Stopping.");
            server.stop().await
        }
        Command::Replay(args) => replay_cli::run(&args),
        Command::Admin(args) => admin_cli::run(args).await,
        Command::ExportSchema { out } => export_schema(&out),
    }
}

/// Prints where the server listens and, in development mode, the tokens
/// that it made. The tokens go to the output stream only, never to the
/// log.
fn announce(server: &Server, store: &StoreConfig) {
    let base = format!("http://{}", server.addr);
    println!("Unsupervised listens on {base}");
    match store {
        StoreConfig::Memory => println!("Accounts are in memory. They are lost at the stop."),
        StoreConfig::Sqlite(path) => println!("Accounts are in {}", path.display()),
    }
    let Some(dev) = &server.dev else {
        return;
    };
    println!();
    println!("Development mode. Owner {} is ready.", dev.owner);
    println!("  owner token: {}", dev.owner_token);
    if let Some(admin) = &dev.admin_token {
        println!("  admin token: {admin}");
    }
    println!();
    println!("Make a resident:");
    println!("  curl -s -X POST {base}/v1/residents \\");
    println!("    -H 'Authorization: Bearer {}' \\", dev.owner_token);
    println!("    -H 'Content-Type: application/json' -d '{{\"name\":\"Ada\"}}'");
}

/// Waits for Ctrl-C or, on Unix, for the signal that asks a process to
/// end.
async fn wait_for_stop_signal() -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate =
            signal(SignalKind::terminate()).context("cannot wait for the terminate signal")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result.context("cannot wait for Ctrl-C"),
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.context("cannot wait for Ctrl-C")
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
