//! The `replay` command: checks the world of a data directory.

use std::path::PathBuf;

use anyhow::Context;
use clap::Args;
use unsup_runtime::ReplayFrom;
use unsup_server::{
    Overrides, ServerConfig, StoreKind, WorldAudit, WorldStore, WorldStoreKind, audit_world,
};

/// The options of `replay`.
#[derive(Args)]
pub struct ReplayArgs {
    /// Configuration file in TOML. The command reads `data_dir` from it.
    #[arg(long, env = "UNSUPERVISED_CONFIG")]
    config: Option<PathBuf>,
    /// The data directory of the server. Its directory `world` holds the
    /// input log and the snapshots.
    #[arg(long, env = "UNSUPERVISED_DATA_DIR")]
    data_dir: Option<PathBuf>,
    /// The snapshot to start from: `oldest` (the longest check), `newest`
    /// (what a start of the server does), or a frame number (the newest
    /// snapshot at or before that frame).
    #[arg(long, default_value = "oldest", value_parser = parse_from)]
    from: ReplayFrom,
    /// Print the result as JSON.
    #[arg(long)]
    json: bool,
}

fn parse_from(text: &str) -> Result<ReplayFrom, String> {
    match text {
        "oldest" => Ok(ReplayFrom::Oldest),
        "newest" => Ok(ReplayFrom::Newest),
        frame => frame
            .parse()
            .map(ReplayFrom::Frame)
            .map_err(|_| "give `oldest`, `newest` or a frame number".to_string()),
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

/// Prints the result for a person.
fn print(dir: &std::path::Path, audit: &WorldAudit) {
    println!("World directory: {}", dir.display());
    println!(
        "Replay from the snapshot of frame {}: {} inputs, {} frames.",
        audit.snapshot_frame, audit.inputs, audit.frames
    );
    println!("The snapshot has the state hash that it names: {}.", yes_no(audit.snapshot_hash_ok));
    println!("The world is at frame {}. State hash: {}", audit.frame, audit.state_hash);
    let matching = audit.snapshots.iter().filter(|snapshot| snapshot.matches).count();
    println!(
        "Later snapshots compared: {}. With the same state hash: {matching}.",
        audit.snapshots.len()
    );
    for snapshot in audit.snapshots.iter().filter(|snapshot| !snapshot.matches) {
        println!("  The snapshot of frame {} has another state hash.", snapshot.frame);
    }
    for skipped in &audit.skipped {
        println!("  Not compared: {skipped}");
    }
    if audit.torn_tail {
        println!("The log ends with a record that a crash cut off. It was ignored.");
    }
    println!(
        "A recovery from the newest snapshot gives the same state: {}.",
        yes_no(audit.recovery_matches)
    );
    println!("Residents: {}.", audit.residents);
    match (&audit.money, &audit.audit_error) {
        (Some(money), _) => println!(
            "Money in cents: {} held, {} created, {} destroyed, {} between realms. It adds up.",
            money.held, money.minted, money.burned, money.in_flight
        ),
        (None, Some(error)) => println!("The audit of money and goods failed: {error}"),
        (None, None) => {}
    }
    println!("Result: {}.", if audit.consistent { "consistent" } else { "NOT consistent" });
}

/// Runs the command. It fails, and the process ends with an error, when
/// the directory cannot be read or a check does not pass.
pub fn run(args: &ReplayArgs) -> anyhow::Result<()> {
    // The same rules as `run`: the world is in `<data_dir>/world`.
    let overrides = Overrides {
        data_dir: args.data_dir.clone(),
        store: Some(StoreKind::Sqlite),
        world_store: Some(WorldStoreKind::Disk),
        ..Overrides::default()
    };
    let config = ServerConfig::load(args.config.as_deref(), overrides)?;
    let WorldStore::Dir(dir) = config.world_store else {
        anyhow::bail!("the configuration keeps the world in memory: there is nothing to replay");
    };
    let audit = audit_world(&dir, args.from)?;
    if args.json {
        let text = serde_json::to_string_pretty(&audit).context("cannot write the result")?;
        println!("{text}");
    } else {
        print(&dir, &audit);
    }
    anyhow::ensure!(audit.consistent, "the world in {} is not consistent", dir.display());
    Ok(())
}
