//! Replays the input log of a stored world and reports what it found: the
//! frames, the inputs and the final state hash.
//!
//! The replay starts at a snapshot and applies every later input of the
//! log. At each later snapshot it compares the state hash of the replayed
//! world with the hash that the live run wrote. It does not change the
//! directory.
//!
//! ```text
//! cargo run -p unsup-runtime --example replay
//! cargo run -p unsup-runtime --example replay -- record DIR [ROUNDS]
//! cargo run -p unsup-runtime --example replay -- check DIR [FROM]
//! ```
//!
//! Without arguments the example records a small world in a scratch
//! directory, checks it and removes the directory. `record` writes a world
//! of the town module (the module of the tests of this crate) to `DIR`.
//! `check` replays the world in `DIR`. `FROM` is `oldest` (the default),
//! `newest`, or a frame number: the replay starts at the newest snapshot at
//! or before that frame.
//!
//! A replay needs the plugins that made the world. This example knows only
//! the town module, so it checks only worlds that `record` made. The server
//! has every game module; its replay tool calls the same
//! `unsup_runtime::replay`.
//!
//! The exit code is 0 when the replay checked every later snapshot and
//! every state hash matches, 1 when it did not, and 2 when the replay did
//! not run.

#[path = "../tests/common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use unsup_runtime::{
    DurableConfig, DurableJournal, FsStorage, ReplayFrom, ReplayOptions, ReplayReport,
    RuntimeConfig, SyncPolicy, replay, start,
};

use common::{Town, hex, new_world, play, plugins};

/// Rounds of the script that `record` plays when none are given.
const ROUNDS: u64 = 24;

/// Frames between two snapshots of a recorded world.
const SNAPSHOT_EVERY: u64 = 6;

const USAGE: &str = "usage:
  replay                        record a small world in a scratch directory and check it
  replay record DIR [ROUNDS]    write a world of the town module to DIR
  replay check DIR [FROM]       replay the world in DIR; FROM is oldest, newest or a frame";

/// Writes a world of the town module to a directory: `rounds` rounds of the
/// script, each with one frame, and a snapshot every few frames.
fn record(dir: &Path, rounds: u64) -> Result<(), String> {
    let storage = FsStorage::open(dir).map_err(|e| e.to_string())?;
    let durable = DurableConfig {
        sync: SyncPolicy::EveryBatch,
        snapshot_every: SNAPSHOT_EVERY,
        // Every snapshot stays, so that a check can pass all of them.
        keep_snapshots: usize::MAX,
        ..DurableConfig::default()
    };
    let opened = DurableJournal::open(Arc::new(storage), plugins(), durable, new_world)
        .map_err(|e| e.to_string())?;
    let first = opened.world.frame();
    let config = RuntimeConfig { journal: Box::new(opened.journal), ..RuntimeConfig::manual() };
    let (handle, join) = start(opened.world, config);
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("the tokio runtime did not start: {e}"))?;
    tokio.block_on(play(&handle, first, first + rounds));
    let world = join.shutdown().map_err(|e| e.to_string())?;
    println!(
        "recorded {} in {}: frames {first} to {}, state hash {}",
        if first == 0 { "a new world" } else { "more of the world" },
        dir.display(),
        world.frame(),
        hex(&world.state_hash()),
    );
    Ok(())
}

/// Reads the `FROM` argument.
fn parse_from(text: &str) -> Result<ReplayFrom, String> {
    match text {
        "oldest" => Ok(ReplayFrom::Oldest),
        "newest" => Ok(ReplayFrom::Newest),
        frame => frame
            .parse()
            .map(ReplayFrom::Frame)
            .map_err(|_| format!("`{frame}` is not `oldest`, `newest` or a frame number")),
    }
}

/// Prints what a replay found.
fn print(dir: &Path, report: &ReplayReport) {
    println!("replay of {}", dir.display());
    println!(
        "  start:      the snapshot of frame {} (after {} inputs); its state hash is {}",
        report.snapshot_frame,
        report.snapshot_seq,
        if report.snapshot_hash_ok { "right" } else { "WRONG" },
    );
    println!("  inputs:     {}", report.inputs);
    println!("  frames:     {}", report.frames);
    println!("  end:        frame {}, after {} inputs", report.frame, report.next_seq);
    println!("  state hash: {}", hex(&report.state_hash));
    let matching = report.checkpoints.iter().filter(|checkpoint| checkpoint.matches()).count();
    println!(
        "  later snapshots: {} passed, {matching} with the same state hash",
        report.checkpoints.len()
    );
    for checkpoint in &report.checkpoints {
        if checkpoint.matches() {
            println!("    frame {:>6}: same", checkpoint.frame);
        } else {
            println!(
                "    frame {:>6}: DIFFERENT; the snapshot says {}, the replay gives {}",
                checkpoint.frame,
                hex(&checkpoint.expected),
                hex(&checkpoint.found),
            );
        }
    }
    if let Some(torn) = &report.torn_tail {
        println!(
            "  torn end of the log: record {} at byte {} of `{}`, {} bytes ignored ({})",
            torn.seq, torn.offset, torn.segment, torn.ignored_bytes, torn.reason
        );
    }
    for snapshot in &report.skipped {
        println!("  snapshot not used: `{}`: {}", snapshot.name, snapshot.reason);
    }
}

/// Replays the world in a directory. Returns whether every snapshot was
/// checked and every state hash matched.
fn check(dir: &Path, from: ReplayFrom) -> Result<bool, String> {
    if !dir.is_dir() {
        return Err(format!("`{}` is not a directory", dir.display()));
    }
    let storage = FsStorage::open(dir).map_err(|e| e.to_string())?;
    let report = replay(&storage, &[&Town::default()], &ReplayOptions { from })
        .map_err(|e| e.to_string())?;
    print(dir, &report);
    let result = match (report.is_consistent(), report.skipped.len()) {
        (true, 0) => "the log gives the recorded world".to_string(),
        (true, unchecked) => format!(
            "the log gives the recorded world as far as it was checked; snapshots not checked: {unchecked}"
        ),
        (false, _) => "the log does NOT give the recorded world".to_string(),
    };
    println!("result: {result}");
    Ok(report.is_consistent() && report.skipped.is_empty())
}

/// Records a small world in a scratch directory, checks it and removes it.
fn demonstration() -> Result<bool, String> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("unsup-replay-example-{}", std::process::id()));
    let result = record(&dir, ROUNDS).and_then(|()| check(&dir, ReplayFrom::Oldest));
    std::fs::remove_dir_all(&dir)
        .map_err(|e| format!("cannot remove the scratch directory `{}`: {e}", dir.display()))?;
    result
}

fn run(args: &[String]) -> Result<bool, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] => demonstration(),
        ["record", dir] => record(Path::new(dir), ROUNDS).map(|()| true),
        ["record", dir, rounds] => {
            let rounds =
                rounds.parse().map_err(|_| format!("`{rounds}` is not a number of rounds"))?;
            record(Path::new(dir), rounds).map(|()| true)
        }
        ["check", dir] => check(Path::new(dir), ReplayFrom::Oldest),
        ["check", dir, from] => check(Path::new(dir), parse_from(from)?),
        _ => Err(USAGE.to_string()),
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(2)
        }
    }
}
