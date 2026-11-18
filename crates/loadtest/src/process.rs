//! Facts about this process: the memory that it holds and the files that
//! it has open.
//!
//! The workspace forbids `unsafe` code, so this module asks the operating
//! system through files (`/proc`, `/dev/fd`) and small standard commands
//! (`ps`, `footprint`). Every fact is optional: a fact that cannot be read
//! is `None` and the report says so.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::machine::{kib, output_of, proc_field};

/// Returns the resident set size of a process in bytes: the memory that it
/// really holds. `None` when the system gives no answer.
#[must_use]
pub fn rss_bytes(pid: u32) -> Option<u64> {
    if let Some(text) = proc_field(&format!("/proc/{pid}/status"), "VmRSS") {
        return kib(&text);
    }
    kib(&output_of("ps", &["-o", "rss=", "-p", &pid.to_string()])?)
}

/// Returns the resident set size of this process in bytes.
#[must_use]
pub fn own_rss_bytes() -> Option<u64> {
    rss_bytes(std::process::id())
}

/// The memory that a process owns, with the part that the system moved out
/// of the resident set.
///
/// A system that is short of memory compresses pages or writes them to
/// swap. The resident set size then goes down, but the process still owns
/// the memory, and each use of such a page costs time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Footprint {
    /// Memory that the process owns now, in bytes: resident, compressed and
    /// swapped.
    pub bytes: u64,
    /// Largest value since the process started, when the system keeps it.
    pub peak_bytes: Option<u64>,
}

/// Reads the output of `footprint -f bytes --noCategories` (macOS): the
/// lines `phys_footprint: 123 B` and `phys_footprint_peak: 456 B`.
fn parse_footprint(text: &str) -> Option<Footprint> {
    let field = |key: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| value.split_whitespace().next()?.parse().ok())?
        })
    };
    Some(Footprint { bytes: field("phys_footprint")?, peak_bytes: field("phys_footprint_peak") })
}

/// Reads `VmRSS`, `VmSwap` and `VmHWM` of a `/proc/<pid>/status` file
/// (Linux). The peak is the largest resident set size; the system does not
/// keep the largest sum.
fn parse_proc_status(text: &str) -> Option<Footprint> {
    let field = |key: &str| -> Option<u64> {
        text.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim() == key).then(|| kib(value))?
        })
    };
    let bytes = field("VmRSS")?.checked_add(field("VmSwap").unwrap_or(0))?;
    Some(Footprint { bytes, peak_bytes: field("VmHWM").map(|peak| peak.max(bytes)) })
}

/// Returns the memory that a process owns, with compressed and swapped
/// pages. `None` when the system gives no answer.
///
/// On macOS this runs the `footprint` tool, which takes longer than a read
/// of the resident set size. Call it at the end of a run, not in a loop.
#[must_use]
pub fn footprint(pid: u32) -> Option<Footprint> {
    if let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
        return parse_proc_status(&text);
    }
    let pid = pid.to_string();
    parse_footprint(&output_of("footprint", &["-p", &pid, "-f", "bytes", "--noCategories"])?)
}

/// Returns the memory that this process owns, with compressed and swapped
/// pages.
#[must_use]
pub fn own_footprint() -> Option<Footprint> {
    footprint(std::process::id())
}

/// Returns the number of files that this process has open now, or `None`
/// when the system does not list them.
#[must_use]
pub fn open_files() -> Option<u64> {
    // The listing itself holds one descriptor open while it runs.
    let entries = std::fs::read_dir("/dev/fd").ok()?;
    Some((entries.count() as u64).saturating_sub(1))
}

/// Returns the memory that this process holds, in bytes: the larger of
/// its resident set size and its footprint. `None` when the system gives
/// neither.
#[must_use]
pub fn own_held_bytes() -> Option<u64> {
    match (own_rss_bytes(), own_footprint().map(|footprint| footprint.bytes)) {
        (Some(rss), Some(footprint)) => Some(rss.max(footprint)),
        (rss, footprint) => rss.or(footprint),
    }
}

/// The largest values that a [`MemorySampler`] saw, in bytes. `None` when
/// no sample could be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Peaks {
    /// Largest resident set size.
    pub rss: Option<u64>,
    /// Largest held memory: the larger of the resident set size and the
    /// footprint.
    pub held: Option<u64>,
}

/// Samples the memory of this process on a thread of its own and keeps the
/// largest values. It reads the resident set size at every sample and the
/// footprint, which costs more, at every n-th sample.
#[derive(Debug)]
pub struct MemorySampler {
    stop: Arc<AtomicBool>,
    rss_peak: Arc<AtomicU64>,
    held_peak: Arc<AtomicU64>,
    limit: Option<u64>,
    thread: Option<JoinHandle<()>>,
}

impl MemorySampler {
    /// Takes one full sample now and then one sample every `interval`. The
    /// footprint is read at every `footprint_every`-th sample. With a
    /// `limit` in bytes, [`MemorySampler::over_limit`] tells when the
    /// process holds more.
    #[must_use]
    pub fn start(interval: Duration, footprint_every: u32, limit: Option<u64>) -> MemorySampler {
        let stop = Arc::new(AtomicBool::new(false));
        // The first sample is taken here, so that a limit holds from the
        // first check on.
        let rss = own_rss_bytes().unwrap_or(0);
        let rss_peak = Arc::new(AtomicU64::new(rss));
        let held_peak = Arc::new(AtomicU64::new(own_held_bytes().unwrap_or(rss)));
        let (stop_flag, rss_cell, held_cell) =
            (Arc::clone(&stop), Arc::clone(&rss_peak), Arc::clone(&held_peak));
        let footprint_every = u64::from(footprint_every.max(1));
        let thread = std::thread::Builder::new()
            .name("memory-sampler".into())
            .spawn(move || {
                let mut samples = 0u64;
                while !stop_flag.load(Ordering::Relaxed) {
                    std::thread::park_timeout(interval);
                    samples += 1;
                    if let Some(bytes) = own_rss_bytes() {
                        rss_cell.fetch_max(bytes, Ordering::Relaxed);
                        held_cell.fetch_max(bytes, Ordering::Relaxed);
                    }
                    if samples.is_multiple_of(footprint_every)
                        && let Some(footprint) = own_footprint()
                    {
                        held_cell.fetch_max(footprint.bytes, Ordering::Relaxed);
                    }
                }
            })
            .ok();
        MemorySampler { stop, rss_peak, held_peak, limit, thread }
    }

    /// Returns the largest held memory so far when it is above the limit.
    /// The check is one atomic read, so a loop can call it often.
    #[must_use]
    pub fn over_limit(&self) -> Option<u64> {
        let peak = self.held_peak.load(Ordering::Relaxed);
        self.limit.is_some_and(|limit| peak > limit).then_some(peak)
    }

    /// Takes one more full sample, stops the thread and returns the
    /// largest values.
    #[must_use]
    pub fn finish(mut self) -> Peaks {
        self.halt();
        if let Some(bytes) = own_rss_bytes() {
            self.rss_peak.fetch_max(bytes, Ordering::Relaxed);
        }
        if let Some(bytes) = own_held_bytes() {
            self.held_peak.fetch_max(bytes, Ordering::Relaxed);
        }
        let known = |cell: &AtomicU64| match cell.load(Ordering::Relaxed) {
            0 => None,
            bytes => Some(bytes),
        };
        Peaks { rss: known(&self.rss_peak), held: known(&self.held_peak) }
    }

    fn halt(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            // The thread only reads numbers; it cannot panic in a way that
            // matters here.
            let _ = thread.join();
        }
    }
}

impl Drop for MemorySampler {
    fn drop(&mut self) {
        self.halt();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sampler_sees_the_memory_of_the_process_and_a_limit() {
        // An hour between two samples: only the sample of `start` counts.
        let hour = Duration::from_secs(3_600);
        let unlimited = MemorySampler::start(hour, 1, None);
        let tiny = MemorySampler::start(hour, 1, Some(1));
        let huge = MemorySampler::start(hour, 1, Some(u64::MAX));
        assert_eq!(unlimited.over_limit(), None, "no limit, never over it");
        assert_eq!(huge.over_limit(), None);
        // The limit holds at once: `start` took the first sample itself.
        let over = tiny.over_limit().expect("the process holds more than one byte");
        assert!(over > 100_000, "{over} bytes");
        let peaks = unlimited.finish();
        let rss = peaks.rss.expect("a sample of the resident set size");
        let held = peaks.held.expect("a sample of the held memory");
        assert!(rss > 100_000, "{rss} bytes");
        assert!(held >= rss, "held {held} is the larger of the two, the resident set is {rss}");
        assert!(tiny.finish().held.is_some());
    }

    #[test]
    fn the_sampler_thread_sees_the_process_grow_and_keeps_the_peak() {
        let before = own_held_bytes().expect("the system tells the memory");
        let limit = before + (32 << 20);
        let sampler = MemorySampler::start(Duration::from_millis(2), 1, Some(limit));
        assert_eq!(sampler.over_limit(), None, "the process did not grow yet");
        // 64 MiB that the process really touches.
        let block = vec![1u8; 64 << 20];
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while sampler.over_limit().is_none() {
            assert!(std::time::Instant::now() < deadline, "no sample saw the 64 MiB");
            std::thread::yield_now();
        }
        std::hint::black_box(&block);
        drop(block);
        // The memory is gone, the peak stays.
        let peaks = sampler.finish();
        assert!(peaks.held.expect("a sample") > limit, "{peaks:?}, limit {limit}");
    }

    #[test]
    fn the_footprint_counts_compressed_and_swapped_memory() {
        // macOS: the answer of `footprint -f bytes --noCategories`.
        let mac = "======\nbench [31315]: 64-bit    Footprint: 1901024 B (16384 bytes per page)\n\
                   ======\n\nAuxiliary data:\n    phys_footprint: 1901024 B\n    \
                   phys_footprint_peak: 2000000 B\n";
        assert_eq!(
            parse_footprint(mac),
            Some(Footprint { bytes: 1_901_024, peak_bytes: Some(2_000_000) })
        );
        assert_eq!(parse_footprint("no such process\n"), None);
        // Linux: resident plus swapped. The peak of the system is the
        // largest resident set size.
        let linux =
            "Name:\tbench\nVmHWM:\t    5000 kB\nVmRSS:\t    3000 kB\nVmSwap:\t    1000 kB\n";
        assert_eq!(
            parse_proc_status(linux),
            Some(Footprint { bytes: 4_000 * 1_024, peak_bytes: Some(5_000 * 1_024) })
        );
        let no_swap = "VmRSS:\t 3000 kB\n";
        assert_eq!(
            parse_proc_status(no_swap),
            Some(Footprint { bytes: 3_000 * 1_024, peak_bytes: None })
        );
        assert_eq!(parse_proc_status("Name:\tkthreadd\n"), None);
        // This process: nothing is swapped in a test, so the footprint is
        // near the resident set size, and the peak is not below it.
        let own = own_footprint().expect("the system tells the footprint");
        assert!(own.bytes > 100_000, "{} bytes", own.bytes);
        let peak = own.peak_bytes.expect("the system keeps the peak");
        assert!(peak >= own.bytes, "peak {peak} below {}", own.bytes);
    }

    #[test]
    fn this_process_has_memory_and_open_files() {
        // Both facts come from the system; the test only needs them to be
        // there and plausible on the systems that the project supports.
        let rss = own_rss_bytes().expect("the system reports the resident set size");
        assert!(rss > 100_000, "a test process holds more than 100 kB, got {rss}");
        // Other tests of this process open and close files at the same time,
        // so the count is compared with a margin.
        let before = open_files().expect("the system lists open files");
        let files: Vec<std::fs::File> = (0..50)
            .map(|_| std::fs::File::open("Cargo.toml").expect("the manifest opens"))
            .collect();
        let after = open_files().expect("the system lists open files");
        drop(files);
        assert!(after >= before + 40, "50 more files are open: {before} before, {after} after");
    }
}
