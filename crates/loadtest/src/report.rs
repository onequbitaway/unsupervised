//! The report format that all commands share: one JSON file and a summary
//! that a person can read.
//!
//! Every report has the same head: the command and its arguments, the time,
//! the machine and the commit. The `result` is the part of one command.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::machine::{self, Commit, Machine};

/// The directory for reports when the command line names none. The
/// repository ignores `var/`.
pub const DEFAULT_DIR: &str = "var/loadtest";

/// A report of one command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report<T> {
    /// Name of the tool.
    pub tool: String,
    /// Version of the tool.
    pub version: String,
    /// The command: `bench`, `soak` or `swarm`.
    pub command: String,
    /// The arguments of the command line, without the program name.
    pub argv: Vec<String>,
    /// When the command started, in UTC.
    pub started_at: String,
    /// The machine that ran the command.
    pub machine: Machine,
    /// Load average of the last minute when the command ended. The value
    /// at the start is in `machine`. A value near or above the number of
    /// logical cores means that other programs used the machine too.
    pub load_average_end: Option<f64>,
    /// The commit of the source tree, when the command ran inside one.
    pub commit: Option<Commit>,
    /// What the command measured.
    pub result: T,
}

/// The head of a report, made when a command starts.
#[derive(Clone, Debug)]
pub struct Head {
    command: String,
    argv: Vec<String>,
    started_unix: u64,
    machine: Machine,
    commit: Option<Commit>,
}

impl Head {
    /// Reads the machine, the commit and the time for a command that starts
    /// now.
    #[must_use]
    pub fn now(command: &str, argv: Vec<String>) -> Head {
        Head {
            command: command.to_string(),
            argv,
            started_unix: machine::unix_now(),
            machine: Machine::detect(),
            commit: machine::commit(),
        }
    }

    /// Returns the path of the report file when the command line names
    /// none: `var/loadtest/<command>-<time>.json`.
    #[must_use]
    pub fn default_path(&self) -> PathBuf {
        let stamp = machine::utc_timestamp(self.started_unix).replace([':', '-'], "");
        Path::new(DEFAULT_DIR).join(format!("{}-{stamp}.json", self.command))
    }

    /// Joins the head with the result of the command. Call it when the
    /// command ends: it reads the load of the machine once more.
    #[must_use]
    pub fn with<T>(self, result: T) -> Report<T> {
        Report {
            tool: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            command: self.command,
            argv: self.argv,
            started_at: machine::utc_timestamp(self.started_unix),
            machine: self.machine,
            load_average_end: machine::load_average(),
            commit: self.commit,
            result,
        }
    }
}

impl<T> Report<T> {
    /// Returns the lines that open the readable summary: the command, the
    /// time, the machine with its load, and the commit.
    #[must_use]
    pub fn head_lines(&self) -> String {
        let load = |value: Option<f64>| {
            value.map_or_else(|| "unknown".to_string(), |load| format!("{load:.1}"))
        };
        format!(
            "{} at {}\nmachine: {}\nload average: {} at the start, {} at the end\ncommit: {}\n",
            self.command,
            self.started_at,
            self.machine.line(),
            load(self.machine.load_average),
            load(self.load_average_end),
            self.commit.as_ref().map_or_else(|| "unknown".to_string(), Commit::to_string),
        )
    }
}

impl<T: Serialize> Report<T> {
    /// Writes the report as JSON. It makes the directory when needed.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be written.
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot make the directory {}", dir.display()))?;
        }
        let mut text = serde_json::to_string_pretty(self).context("cannot write the report")?;
        text.push('\n');
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))
    }
}

impl<T: DeserializeOwned> Report<T> {
    /// Reads a report from a JSON file.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not a report of this kind.
    pub fn read(path: &Path) -> anyhow::Result<Report<T>> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        serde_json::from_str(&text)
            .with_context(|| format!("{} is not a report of this command", path.display()))
    }
}

/// Writes a number with a comma between each group of three digits.
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Writes a number of bytes in mebibytes with one decimal.
#[must_use]
pub fn mib(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_grouped_by_three_digits() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(mib(3 * 1024 * 1024 + 512 * 1024), "3.5 MiB");
    }

    #[test]
    fn a_report_survives_a_round_trip_through_a_file() {
        let head = Head::now("bench", vec!["bench".into(), "--frames".into(), "3".into()]);
        let path = head.default_path();
        assert!(path.starts_with(DEFAULT_DIR));
        let report = head.with(vec![1u32, 2, 3]);
        let dir = std::env::temp_dir().join(format!("unsup-report-{}", std::process::id()));
        let file = dir.join("nested").join("report.json");
        report.write(&file).expect("the report is written");
        let back: Report<Vec<u32>> = Report::read(&file).expect("the report is read");
        std::fs::remove_dir_all(&dir).expect("the temporary directory is removed");
        assert_eq!(back, report);
        assert_eq!(back.command, "bench");
        assert_eq!(back.tool, "unsup-loadtest");
    }

    #[test]
    fn the_head_of_the_summary_names_the_command_the_machine_and_the_load() {
        let mut report = Head::now("soak", Vec::new()).with(());
        report.started_at = "2026-11-18T09:52:50Z".to_string();
        report.machine = Machine {
            cpu: Some("Test CPU".into()),
            logical_cores: Some(8),
            sim_threads: 8,
            memory_bytes: Some(16 << 30),
            os: Some("TestOS 1".into()),
            arch: "aarch64".into(),
            debug_build: false,
            load_average: Some(1.3),
        };
        report.load_average_end = Some(9.5);
        report.commit = Some(Commit { hash: "abc1234".into(), dirty: true });
        assert_eq!(
            report.head_lines(),
            "soak at 2026-11-18T09:52:50Z\n\
             machine: Test CPU, 8 logical cores, 16 GiB memory, TestOS 1, 8 zone threads\n\
             load average: 1.3 at the start, 9.5 at the end\n\
             commit: abc1234 (with local changes)\n"
        );
        report.machine.debug_build = true;
        report.load_average_end = None;
        report.commit = None;
        let lines = report.head_lines();
        assert!(lines.contains("DEBUG BUILD (numbers are not valid)"), "{lines}");
        assert!(lines.contains("1.3 at the start, unknown at the end"), "{lines}");
        assert!(lines.ends_with("commit: unknown\n"), "{lines}");
    }
}
