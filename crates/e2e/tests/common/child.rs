//! Child processes: the SDKs and the agents in `node` and `python3`.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

/// What a child process left behind.
#[derive(Debug)]
pub struct Finished {
    /// True when the process ended with the status 0.
    pub success: bool,
    /// Everything that the process wrote to its output.
    pub stdout: String,
    /// Everything that the process wrote to its error stream.
    pub stderr: String,
}

impl Finished {
    /// Returns the lines of the output that are JSON objects, in order.
    /// The scripts of the tests write one object for each fact.
    pub fn facts(&self) -> Vec<serde_json::Value> {
        self.stdout
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(serde_json::Value::is_object)
            .collect()
    }

    /// Returns the fact with a `name`.
    ///
    /// # Panics
    ///
    /// Panics when the process did not write it.
    pub fn fact(&self, name: &str) -> serde_json::Value {
        self.facts()
            .into_iter()
            .find(|fact| fact["fact"] == name)
            .unwrap_or_else(|| panic!("the process wrote the fact `{name}`:\n{self}"))
    }
}

impl std::fmt::Display for Finished {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "success: {}\n--- output\n{}\n--- errors\n{}",
            self.success, self.stdout, self.stderr
        )
    }
}

/// Runs a program to its end and returns what it wrote. The two streams
/// are read at the same time, so a full pipe cannot stop the process.
/// The process is killed when it runs longer than `limit`.
///
/// # Panics
///
/// Panics when the program cannot start, or when the time is over.
pub async fn run<I, S>(
    program: &str,
    args: I,
    dir: &Path,
    env: &[(&str, String)],
    limit: Duration,
) -> Finished
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // A test that fails leaves no process behind.
        .kill_on_drop(true);
    for (name, value) in env {
        command.env(name, value);
    }
    // A proxy of the machine must not take the requests to this machine.
    command.env("NO_PROXY", "127.0.0.1,localhost").env("no_proxy", "127.0.0.1,localhost");
    let child = command.spawn().unwrap_or_else(|error| panic!("`{program}` starts: {error}"));
    let output = tokio::time::timeout(limit, child.wait_with_output())
        .await
        .unwrap_or_else(|_| panic!("`{program}` ends in {limit:?}"))
        .expect("the process ends");
    Finished {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A child process that runs until the test stops it.
pub struct Running {
    child: tokio::process::Child,
    stdout: tokio::task::JoinHandle<String>,
    stderr: tokio::task::JoinHandle<String>,
}

/// Reads a stream of a child process to its end.
fn drain<R>(stream: Option<R>) -> tokio::task::JoinHandle<String>
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut text = Vec::new();
        if let Some(mut stream) = stream {
            // A stream that breaks ends here. What came so far stays.
            let _ = stream.read_to_end(&mut text).await;
        }
        String::from_utf8_lossy(&text).into_owned()
    })
}

/// Starts a program that runs until [`Running::stop`]. The two streams
/// are read at the same time, so a full pipe cannot stop the process.
///
/// # Panics
///
/// Panics when the program cannot start.
pub fn start<I, S>(program: &str, args: I, dir: &Path, env: &[(&str, String)]) -> Running
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (name, value) in env {
        command.env(name, value);
    }
    command.env("NO_PROXY", "127.0.0.1,localhost").env("no_proxy", "127.0.0.1,localhost");
    let mut child = command.spawn().unwrap_or_else(|error| panic!("`{program}` starts: {error}"));
    let (stdout, stderr) = (drain(child.stdout.take()), drain(child.stderr.take()));
    Running { child, stdout, stderr }
}

impl Running {
    /// Returns true while the process runs.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Stops the process and returns what it wrote.
    pub async fn stop(mut self) -> Finished {
        // The process can be gone already.
        let _ = self.child.kill().await;
        let status = self.child.wait().await.expect("the process ends");
        Finished {
            success: status.success(),
            stdout: self.stdout.await.expect("the output is read"),
            stderr: self.stderr.await.expect("the errors are read"),
        }
    }
}
