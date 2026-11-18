//! The tunable numbers of the runtime, read from `data/runtime.toml`.
//!
//! The file is part of the build. The default value of every configuration
//! type of this crate comes from it, so no tunable number lives in code.

use std::sync::OnceLock;

use serde::Deserialize;

/// The content of `data/runtime.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub runtime: RuntimeSettings,
    pub log: LogSettings,
    pub snapshots: SnapshotSettings,
    pub dilation: DilationSettings,
    pub cluster: ClusterSettings,
}

/// The table `[runtime]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RuntimeSettings {
    pub tick_interval_ms: u64,
    pub mailbox: usize,
    pub batch: usize,
    pub frame_buffer: usize,
    pub max_waiters: usize,
    pub max_waiters_per_resident: usize,
    pub stats_window_frames: usize,
    pub rate_window_ms: u64,
    pub reply_handoff: usize,
    pub reply_queue: usize,
}

/// The table `[log]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LogSettings {
    pub segment_bytes: u64,
    pub max_record_bytes: u32,
    pub sync: SyncSetting,
    pub sync_interval_ms: u64,
}

/// The values of `log.sync`.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SyncSetting {
    Record,
    Batch,
    Interval,
}

/// The table `[snapshots]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SnapshotSettings {
    pub every_frames: u64,
    pub keep: usize,
}

/// The table `[dilation]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DilationSettings {
    pub budget_percent: u32,
    pub relax_percent: u32,
    pub max_divisor: u32,
    pub smoothing: u32,
    pub calm_ticks: u32,
}

/// The table `[cluster]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClusterSettings {
    pub join_timeout_ms: u64,
    pub connect_timeout_ms: u64,
    pub retry_ms: u64,
    pub reply_timeout_ms: u64,
    pub max_frame_bytes: u32,
    pub seals_keep: usize,
}

/// Returns the numbers of `data/runtime.toml`.
///
/// # Panics
///
/// Panics when the file does not parse. The file is compiled into the
/// crate, so that is a mistake in the source tree, and a test reads it.
pub(crate) fn settings() -> &'static Settings {
    static SETTINGS: OnceLock<Settings> = OnceLock::new();
    SETTINGS.get_or_init(|| {
        toml::from_str(include_str!("../data/runtime.toml"))
            .expect("data/runtime.toml is a valid settings file")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_file_parses_and_its_numbers_are_usable() {
        let s = settings();
        assert!(s.runtime.mailbox > 0 && s.runtime.batch > 0 && s.runtime.frame_buffer > 0);
        assert!(s.log.segment_bytes > u64::from(s.log.max_record_bytes));
        assert!(s.snapshots.every_frames > 0 && s.snapshots.keep > 0);
        assert!(s.dilation.max_divisor >= 1 && s.dilation.smoothing >= 1);
        assert!(s.dilation.relax_percent < 100 && s.dilation.budget_percent <= 100);
        assert!(s.cluster.join_timeout_ms > 0 && s.cluster.connect_timeout_ms > 0);
        assert!(s.cluster.retry_ms > 0 && s.cluster.reply_timeout_ms > 0);
        assert!(s.cluster.max_frame_bytes >= s.log.max_record_bytes && s.cluster.seals_keep > 0);
    }
}
