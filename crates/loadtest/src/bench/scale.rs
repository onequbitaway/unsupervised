//! Several world sizes in a row, each in a process of its own.
//!
//! A process of its own gives each size a clean memory measurement, and a
//! size that does not fit in memory or in time does not lose the results of
//! the sizes before it.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::time::{Duration, Instant};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::BenchReport;
use crate::report::{Report, thousands};

/// Time between two looks at a child process.
const POLL: Duration = Duration::from_millis(100);

/// A size that did not finish.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScaleFailure {
    /// Number of residents of the size.
    pub residents: u64,
    /// Why there is no result.
    pub reason: String,
}

/// How to run the sizes.
#[derive(Clone, Debug)]
pub struct ScaleOptions {
    /// The arguments that every size shares, for example `--frames 300`.
    /// `bench --residents N --out FILE --child` is put in front of them.
    pub shared_args: Vec<String>,
    /// Greatest time for one size. A size that takes longer is stopped.
    pub time_limit: Option<Duration>,
    /// Directory for the report files of the children.
    pub work_dir: PathBuf,
}

/// Reads a size such as `10000`, `10_000`, `10k` or `1M`.
pub fn parse_size(text: &str) -> Result<u32, String> {
    let clean = text.trim().replace('_', "");
    let (digits, factor) = match clean.char_indices().last() {
        Some((at, 'k' | 'K')) => (&clean[..at], 1_000u64),
        Some((at, 'm' | 'M')) => (&clean[..at], 1_000_000u64),
        _ => (clean.as_str(), 1),
    };
    let number: u64 = digits
        .parse()
        .map_err(|_| format!("`{text}` is not a number of residents; write 10000, 10k or 1M"))?;
    number
        .checked_mul(factor)
        .and_then(|n| u32::try_from(n).ok())
        .filter(|&n| n > 0)
        .ok_or_else(|| format!("`{text}` is not a usable number of residents"))
}

/// Says why a child ended without a result.
fn reason_of(status: ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!(
                "The process was stopped by signal {signal}. The system does this when the memory runs out."
            );
        }
    }
    match status.code() {
        Some(code) => format!("The process ended with exit code {code}. Its messages are above."),
        None => "The process ended without an exit code.".to_string(),
    }
}

/// Waits for a child. Stops it when it takes longer than the limit.
fn wait(child: &mut Child, limit: Option<Duration>) -> anyhow::Result<Result<ExitStatus, String>> {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().context("cannot wait for the bench process")? {
            return Ok(Ok(status));
        }
        if let Some(limit) = limit.filter(|limit| start.elapsed() >= *limit) {
            child.kill().context("cannot stop the bench process")?;
            child.wait().context("cannot wait for the bench process")?;
            return Ok(Err(format!(
                "The run did not finish in {:.0} seconds and was stopped.",
                limit.as_secs_f64()
            )));
        }
        std::thread::sleep(POLL);
    }
}

fn run_child(
    program: &Path,
    residents: u32,
    options: &ScaleOptions,
) -> anyhow::Result<Result<BenchReport, String>> {
    let file = options.work_dir.join(format!("bench-{residents}-{}.part.json", std::process::id()));
    let mut child = Command::new(program)
        .arg("bench")
        .arg("--residents")
        .arg(residents.to_string())
        .arg("--out")
        .arg(&file)
        .arg("--child")
        .args(&options.shared_args)
        .spawn()
        .with_context(|| format!("cannot start {}", program.display()))?;
    let outcome = match wait(&mut child, options.time_limit)? {
        // A child that stops for a reason of its own (a world that is not
        // valid, the memory limit) still writes a report, with the reason.
        Ok(status) => match Report::<BenchReport>::read(&file) {
            Ok(report) => Ok(report.result),
            Err(_) if !status.success() => Err(reason_of(status)),
            Err(error) => Err(format!("The run ended but its report is not usable: {error:#}")),
        },
        Err(reason) => Err(reason),
    };
    // The file is only a hand-over between the two processes.
    let _ = std::fs::remove_file(&file);
    Ok(outcome)
}

/// Runs `bench` once for each size, each in a new process of this program,
/// and joins the results. A size that fails is listed in
/// [`BenchReport::failures`]; the other sizes still run.
///
/// # Errors
///
/// Fails when this program cannot start itself.
pub fn run_scale(sizes: &[u32], options: &ScaleOptions) -> anyhow::Result<BenchReport> {
    let program = std::env::current_exe().context("cannot find the path of this program")?;
    std::fs::create_dir_all(&options.work_dir)
        .with_context(|| format!("cannot make the directory {}", options.work_dir.display()))?;
    let mut report = BenchReport::default();
    for &residents in sizes {
        eprintln!("bench: size {} of the scale list", thousands(u64::from(residents)));
        match run_child(&program, residents, options)? {
            Ok(child) => {
                report.runs.extend(child.runs);
                report.failures.extend(child.failures);
            }
            Err(reason) => {
                eprintln!("bench: {} residents: {reason}", thousands(u64::from(residents)));
                report.failures.push(ScaleFailure { residents: u64::from(residents), reason });
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_can_have_a_unit() {
        assert_eq!(parse_size("10000"), Ok(10_000));
        assert_eq!(parse_size("10_000"), Ok(10_000));
        assert_eq!(parse_size("10k"), Ok(10_000));
        assert_eq!(parse_size(" 100K "), Ok(100_000));
        assert_eq!(parse_size("1M"), Ok(1_000_000));
        assert_eq!(parse_size("2m"), Ok(2_000_000));
        assert!(parse_size("0").is_err());
        assert!(parse_size("ten").is_err());
        assert!(parse_size("k").is_err());
        assert!(parse_size("5000M").is_err(), "more residents than a u32 holds");
        assert!(parse_size("-5").is_err());
    }
}
