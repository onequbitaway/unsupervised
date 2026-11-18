//! Facts about the machine: processor, memory, load, limits, and the
//! commit of the source tree. The facts about this process (its memory and
//! its open files) are in [`crate::process`].
//!
//! The workspace forbids `unsafe` code, so this module asks the operating
//! system through files (`/proc`) and small standard commands (`sysctl`,
//! `vm_stat`, `sh -c ulimit`). Every fact is optional: a fact that cannot
//! be read is `None` and the report says so.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// The machine that ran a command.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    /// Name of the processor.
    pub cpu: Option<String>,
    /// Number of logical processors that the process can use.
    pub logical_cores: Option<u64>,
    /// Threads of the pool that ticks the zones.
    pub sim_threads: u64,
    /// Installed memory in bytes.
    pub memory_bytes: Option<u64>,
    /// Name and version of the operating system.
    pub os: Option<String>,
    /// Processor architecture.
    pub arch: String,
    /// True for a build without optimization. Its numbers mean nothing.
    pub debug_build: bool,
    /// Load average of the last minute when the command started: the mean
    /// number of threads that wanted a processor. A value near or above
    /// the number of logical cores means that other programs used the
    /// machine too, and that the measured times are too high.
    pub load_average: Option<f64>,
}

/// The commit of the source tree in the working directory.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    /// Short hash of `HEAD`.
    pub hash: String,
    /// True when the tree has changes that are not committed.
    pub dirty: bool,
}

impl std::fmt::Display for Commit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.hash, if self.dirty { " (with local changes)" } else { "" })
    }
}

/// Runs a command and returns its trimmed standard output, or `None` when
/// the command is missing, fails or prints nothing.
pub(crate) fn output_of(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Returns the value of a `key: value` line of a `/proc` file.
pub(crate) fn proc_field(path: &str, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == key).then(|| value.trim().to_string())
    })
}

/// Reads a number of kibibytes such as `123456 kB` and returns bytes.
pub(crate) fn kib(text: &str) -> Option<u64> {
    text.split_whitespace().next()?.parse::<u64>().ok()?.checked_mul(1024)
}

impl Machine {
    /// Reads the facts of this machine.
    #[must_use]
    pub fn detect() -> Machine {
        let cpu = proc_field("/proc/cpuinfo", "model name")
            .or_else(|| output_of("sysctl", &["-n", "machdep.cpu.brand_string"]));
        let memory_bytes = proc_field("/proc/meminfo", "MemTotal")
            .and_then(|text| kib(&text))
            .or_else(|| output_of("sysctl", &["-n", "hw.memsize"])?.parse().ok());
        let os = match (
            output_of("sw_vers", &["-productName"]),
            output_of("sw_vers", &["-productVersion"]),
        ) {
            (Some(name), Some(version)) => Some(format!("{name} {version}")),
            _ => output_of("uname", &["-sr"]),
        };
        Machine {
            cpu,
            logical_cores: std::thread::available_parallelism().ok().map(|n| n.get() as u64),
            sim_threads: rayon::current_num_threads() as u64,
            memory_bytes,
            os,
            arch: std::env::consts::ARCH.to_string(),
            debug_build: cfg!(debug_assertions),
            load_average: load_average(),
        }
    }

    /// Returns one line that names the machine.
    #[must_use]
    pub fn line(&self) -> String {
        let gib = self
            .memory_bytes
            .map_or_else(|| "unknown".to_string(), |b| format!("{:.0} GiB", b as f64 / GIB));
        format!(
            "{}, {} logical cores, {} memory, {}, {} zone threads{}",
            self.cpu.as_deref().unwrap_or("unknown processor"),
            self.logical_cores.map_or_else(|| "?".to_string(), |n| n.to_string()),
            gib,
            self.os.as_deref().unwrap_or("unknown system"),
            self.sim_threads,
            if self.debug_build { ", DEBUG BUILD (numbers are not valid)" } else { "" }
        )
    }
}

/// Bytes in one gibibyte.
pub const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// Returns the load average of the last minute, or `None` when the system
/// does not tell.
#[must_use]
pub fn load_average() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .or_else(|| output_of("sysctl", &["-n", "vm.loadavg"]))?;
    // Linux writes `0.52 0.40 0.30 1/123 4567`, macOS `{ 0.52 0.40 0.30 }`.
    text.split_whitespace().find_map(|part| part.parse().ok())
}

/// Reads the page counts of `vm_stat` (macOS) that can be given to a
/// process without swapping: free, inactive, speculative and purgeable
/// pages. Returns bytes.
fn vm_stat_available(text: &str) -> Option<u64> {
    let page_size: u64 = text
        .lines()
        .next()?
        .split("page size of ")
        .nth(1)?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let pages: u64 = text
        .lines()
        .filter_map(|line| {
            let (name, count) = line.split_once(':')?;
            let wanted = ["Pages free", "Pages inactive", "Pages speculative", "Pages purgeable"];
            wanted
                .contains(&name.trim())
                .then(|| count.trim().trim_end_matches('.').parse::<u64>().ok())?
        })
        .sum();
    pages.checked_mul(page_size)
}

/// Returns an estimate of the memory that a new process can take before
/// the system starts to swap, in bytes. `None` when the system does not
/// tell.
#[must_use]
pub fn available_memory_bytes() -> Option<u64> {
    proc_field("/proc/meminfo", "MemAvailable")
        .and_then(|text| kib(&text))
        .or_else(|| vm_stat_available(&output_of("vm_stat", &[])?))
}

/// Returns the commit of the source tree in the working directory, or
/// `None` outside a git repository. It only reads: `--no-optional-locks`
/// stops `git status` from writing a refreshed index.
#[must_use]
pub fn commit() -> Option<Commit> {
    let hash = output_of("git", &["rev-parse", "--short", "HEAD"])?;
    let status =
        Command::new("git").args(["--no-optional-locks", "status", "--porcelain"]).output().ok()?;
    Some(Commit { hash, dirty: !status.stdout.is_empty() })
}

/// A limit of the operating system on a resource.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    /// The system sets no limit.
    Unlimited,
    /// At most this many.
    At(u64),
}

impl Limit {
    /// Returns true when `needed` fits.
    #[must_use]
    pub fn allows(self, needed: u64) -> bool {
        match self {
            Limit::Unlimited => true,
            Limit::At(max) => needed <= max,
        }
    }
}

impl std::fmt::Display for Limit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Limit::Unlimited => f.write_str("unlimited"),
            Limit::At(n) => write!(f, "{n}"),
        }
    }
}

fn parse_limit(text: &str) -> Option<Limit> {
    match text.trim() {
        "unlimited" | "infinity" => Some(Limit::Unlimited),
        number => number.parse().ok().map(Limit::At),
    }
}

/// The limits on open files and connections of this process.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileLimits {
    /// Greatest number of open files of this process now (the soft limit).
    pub soft: Option<Limit>,
    /// The value up to which `ulimit -n` can raise the soft limit.
    pub hard: Option<Limit>,
    /// Greatest number of open files of one process that the kernel allows
    /// (macOS: `kern.maxfilesperproc`). It caps the soft limit.
    pub per_process_cap: Option<u64>,
    /// Number of local ports for outgoing connections. One client address
    /// can hold at most this many connections to one server address.
    pub ephemeral_ports: Option<u64>,
}

impl FileLimits {
    /// Reads the limits. A child shell inherits the limits of this process,
    /// so `ulimit -n` in it gives the right numbers.
    #[must_use]
    pub fn detect() -> FileLimits {
        let ulimit = |flag: &str| {
            output_of("sh", &["-c", &format!("ulimit {flag}")]).and_then(|t| parse_limit(&t))
        };
        let sysctl_number =
            |name: &str| output_of("sysctl", &["-n", name]).and_then(|t| t.parse::<u64>().ok());
        let ephemeral_ports = std::fs::read_to_string("/proc/sys/net/ipv4/ip_local_port_range")
            .ok()
            .and_then(|text| {
                let mut parts = text.split_whitespace().map(str::parse::<u64>);
                let (first, last) = (parts.next()?.ok()?, parts.next()?.ok()?);
                Some(last.checked_sub(first)? + 1)
            })
            .or_else(|| {
                let first = sysctl_number("net.inet.ip.portrange.first")?;
                let last = sysctl_number("net.inet.ip.portrange.last")?;
                Some(last.checked_sub(first)? + 1)
            });
        FileLimits {
            soft: ulimit("-n"),
            hard: ulimit("-Hn"),
            per_process_cap: sysctl_number("kern.maxfilesperproc"),
            ephemeral_ports,
        }
    }

    /// Returns the number of files that this process can really open: the
    /// soft limit, cut by the cap of the kernel. `None` when neither is
    /// known or both are without limit.
    #[must_use]
    pub fn usable(&self) -> Option<u64> {
        let soft = match self.soft {
            Some(Limit::At(n)) => Some(n),
            _ => None,
        };
        match (soft, self.per_process_cap) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Returns the time now as seconds since 1970.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Writes seconds since 1970 as a date and time in UTC, for example
/// `2026-11-18T09:52:50Z`.
#[must_use]
pub fn utc_timestamp(unix_secs: u64) -> String {
    let days = unix_secs / 86_400;
    let secs = unix_secs % 86_400;
    // Civil date from a day count (the algorithm of Howard Hinnant), with
    // the era that starts on 0000-03-01.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_calendar_dates_in_utc() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(utc_timestamp(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(utc_timestamp(1_795_168_370), "2026-11-20T09:52:50Z");
        assert_eq!(utc_timestamp(4_102_444_799), "2099-12-31T23:59:59Z");
    }

    #[test]
    fn limits_parse_and_compare() {
        assert_eq!(parse_limit("unlimited\n"), Some(Limit::Unlimited));
        assert_eq!(parse_limit(" 256 "), Some(Limit::At(256)));
        assert_eq!(parse_limit("many"), None);
        assert!(Limit::At(10).allows(10));
        assert!(!Limit::At(10).allows(11));
        assert!(Limit::Unlimited.allows(u64::MAX));
        let limits = FileLimits {
            soft: Some(Limit::At(1_000_000)),
            hard: Some(Limit::Unlimited),
            per_process_cap: Some(184_320),
            ephemeral_ports: Some(16_384),
        };
        assert_eq!(limits.usable(), Some(184_320));
        let no_cap = FileLimits { per_process_cap: None, ..limits };
        assert_eq!(no_cap.usable(), Some(1_000_000));
        assert_eq!(FileLimits::default().usable(), None);
    }

    #[test]
    fn available_memory_is_read_from_vm_stat() {
        let text = "Mach Virtual Memory Statistics: (page size of 16384 bytes)\n\
                    Pages free:                                   100.\n\
                    Pages active:                                 999.\n\
                    Pages inactive:                               200.\n\
                    Pages speculative:                             30.\n\
                    Pages wired down:                             777.\n\
                    Pages purgeable:                                4.\n";
        assert_eq!(vm_stat_available(text), Some(334 * 16_384));
        assert_eq!(vm_stat_available("no header\nPages free: 5.\n"), None);
        // This machine tells its numbers.
        let available = available_memory_bytes().expect("the system tells the free memory");
        assert!(available > 1_000_000, "{available} bytes are available");
        let load = load_average().expect("the system tells the load");
        assert!((0.0..10_000.0).contains(&load), "load average {load}");
    }

    #[test]
    fn kibibytes_become_bytes() {
        assert_eq!(kib("2048 kB"), Some(2_097_152));
        assert_eq!(kib(" 12\n"), Some(12_288));
        assert_eq!(kib("none"), None);
    }
}
