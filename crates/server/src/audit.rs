//! The check of a data directory: replay the input log with all game
//! modules, compare the state hashes, and audit the money and the goods.

use std::path::Path;

use serde::Serialize;
use unsup_kernel::Plugin;
use unsup_runtime::{FsStorage, RecoverError, ReplayFrom, ReplayOptions, recover, replay};

use crate::durable::hash_text;

/// A later snapshot that the replay passed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CheckedSnapshot {
    /// Frame of the snapshot.
    pub frame: u64,
    /// True when the replayed world had the state hash of the snapshot.
    pub matches: bool,
}

/// The money of the world, in cents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MoneyTotals {
    /// Sum of all balances.
    pub held: i64,
    /// Created through faucets.
    pub minted: i64,
    /// Destroyed through sinks.
    pub burned: i64,
    /// On the way between two realms.
    pub in_flight: i64,
}

/// What [`audit_world`] found in a world directory.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorldAudit {
    /// True when every check passed: the snapshots have the state hashes
    /// that the replay gives, the replay and the recovery end in the same
    /// state, and the money and the goods add up.
    pub consistent: bool,
    /// Frame of the snapshot that the replay started from.
    pub snapshot_frame: u64,
    /// True when the world of that snapshot has the state hash that the
    /// snapshot names.
    pub snapshot_hash_ok: bool,
    /// Inputs of the log that the replay applied.
    pub inputs: u64,
    /// How many of them were frames.
    pub frames: u64,
    /// Number of the last frame of the world.
    pub frame: u64,
    /// The state hash of the world at the end, as hexadecimal text.
    pub state_hash: String,
    /// The later snapshots that the replay compared.
    pub snapshots: Vec<CheckedSnapshot>,
    /// Snapshots that could not be compared, with the reason.
    pub skipped: Vec<String>,
    /// True when the log ended with a record that a crash cut off.
    pub torn_tail: bool,
    /// True when a recovery, which starts from the newest snapshot, ends
    /// in the same state as the replay.
    pub recovery_matches: bool,
    /// Number of residents of the world.
    pub residents: usize,
    /// The money of the world. Absent when the audit failed.
    pub money: Option<MoneyTotals>,
    /// What the audit of money and goods said when it failed.
    pub audit_error: Option<String>,
}

/// Replays the input log of a world directory with all game modules and
/// checks it.
///
/// The replay starts at the snapshot that `from` names and compares the
/// state hash with every later snapshot. Then a recovery, as a start of
/// the server does it, must end in the same state. Then the money and
/// the goods of that world must add up. The directory is not changed.
///
/// # Errors
///
/// Fails when the directory cannot be opened (a server that runs holds
/// it), holds no world, or holds a log that cannot be read.
pub fn audit_world(dir: &Path, from: ReplayFrom) -> anyhow::Result<WorldAudit> {
    anyhow::ensure!(dir.is_dir(), "the world directory {} does not exist", dir.display());
    let storage = FsStorage::open(dir).map_err(|error| {
        anyhow::anyhow!(
            "cannot open the world directory {}: {error}. A server that runs holds its \
             directory: stop it, or copy the directory first",
            dir.display()
        )
    })?;
    let plugins = crate::plugins();
    let refs: Vec<&dyn Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    let explain = |error: RecoverError| match error {
        RecoverError::Empty => anyhow::anyhow!("{} holds no world", dir.display()),
        other => anyhow::anyhow!("cannot replay {}: {other}", dir.display()),
    };

    let report = replay(&storage, &refs, &ReplayOptions { from }).map_err(explain)?;
    let recovery = recover(&storage, &refs).map_err(explain)?;
    let recovery_matches = recovery.report.state_hash == report.state_hash;
    let (money, audit_error) = match recovery.world.audit() {
        Ok(totals) => {
            let money = totals.money;
            let money = MoneyTotals {
                held: money.held,
                minted: money.minted,
                burned: money.burned,
                in_flight: money.in_flight,
            };
            (Some(money), None)
        }
        Err(error) => (None, Some(error.to_string())),
    };
    Ok(WorldAudit {
        consistent: report.is_consistent() && recovery_matches && audit_error.is_none(),
        snapshot_frame: report.snapshot_frame,
        snapshot_hash_ok: report.snapshot_hash_ok,
        inputs: report.inputs,
        frames: report.frames,
        frame: report.frame,
        state_hash: hash_text(&report.state_hash),
        snapshots: report
            .checkpoints
            .iter()
            .map(|point| CheckedSnapshot { frame: point.frame, matches: point.matches() })
            .collect(),
        skipped: report
            .skipped
            .iter()
            .map(|skipped| format!("{}: {}", skipped.name, skipped.reason))
            .collect(),
        torn_tail: report.torn_tail.is_some(),
        recovery_matches,
        residents: recovery.world.residents(),
        money,
        audit_error,
    })
}
