//! The `unsup-loadtest` binary: commands, report files and exit codes.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

/// A directory for the files of one test. It is removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!("unsup-cli-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("the directory is made");
        Scratch(dir)
    }

    fn path(&self, file: &str) -> PathBuf {
        self.0.join(file)
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.0)
            .expect("the directory is read")
            .map(|entry| entry.expect("an entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_unsup-loadtest"))
        .args(args)
        .output()
        .expect("the binary starts")
}

/// Runs the binary in a shell that allows at most `open_files` open files
/// for it.
fn run_with_open_files(open_files: u32, args: &[&str]) -> Output {
    Command::new("sh")
        .arg("-c")
        .arg(format!("ulimit -n {open_files} && exec \"$0\" \"$@\""))
        .arg(env!("CARGO_BIN_EXE_unsup-loadtest"))
        .args(args)
        .output()
        .expect("the shell starts")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn report(path: &PathBuf) -> Value {
    let text = std::fs::read_to_string(path).expect("the report file exists");
    serde_json::from_str(&text).expect("the report is JSON")
}

#[test]
fn help_lists_the_three_commands() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let help = text(&output.stdout);
    for command in ["bench", "soak", "swarm"] {
        assert!(help.contains(command), "the help names `{command}`:\n{help}");
    }
}

#[test]
fn bench_scale_runs_each_size_and_writes_one_report() {
    let scratch = Scratch::new("scale");
    let out = scratch.path("bench.json");
    let output = run(&[
        "bench",
        "--scale",
        "300,1k",
        "--frames",
        "5",
        "--warmup",
        "2",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert!(output.status.success(), "stderr:\n{}", text(&output.stderr));
    let summary = text(&output.stdout);
    assert!(summary.contains("300 residents in"), "{summary}");
    assert!(summary.contains("1,000 residents in"), "{summary}");
    assert!(summary.contains("residents | zones | frame ms"), "{summary}");
    // One head and one report line: the child processes print neither.
    assert_eq!(summary.matches("\nmachine: ").count(), 1, "{summary}");
    assert_eq!(summary.matches("\nload average: ").count(), 1, "{summary}");
    assert_eq!(summary.matches("report: ").count(), 1, "{summary}");
    assert!(summary.trim_end().ends_with(&format!("report: {}", out.display())), "{summary}");

    let report = report(&out);
    assert_eq!(report["tool"], "unsup-loadtest");
    assert_eq!(report["command"], "bench");
    assert_eq!(report["argv"][0], "bench");
    assert_eq!(report["argv"][2], "300,1k");
    assert!(report["started_at"].as_str().expect("a time").ends_with('Z'));
    assert!(report["machine"]["sim_threads"].as_u64().expect("a number") > 0);
    let runs = report["result"]["runs"].as_array().expect("a list of runs");
    let sizes: Vec<u64> =
        runs.iter().map(|run| run["residents"].as_u64().expect("a number")).collect();
    assert_eq!(sizes, [300, 1_000]);
    for run in runs {
        assert_eq!(run["frames"], 5);
        assert_eq!(run["warmup_frames"], 2);
        assert_eq!(run["frame_ms"]["count"], 5);
        assert_eq!(run["profile"]["name"], "walkers");
    }
    assert_eq!(report["result"]["failures"], serde_json::json!([]));
    // The hand-over files of the two child processes are gone.
    assert_eq!(scratch.files(), ["bench.json"]);
}

#[test]
fn a_size_that_does_not_finish_is_named_and_the_exit_code_is_not_zero() {
    let scratch = Scratch::new("limit");
    let out = scratch.path("bench.json");
    // No run finishes in zero seconds.
    let output = run(&[
        "bench",
        "--scale",
        "300",
        "--frames",
        "5",
        "--warmup",
        "0",
        "--time-limit-secs",
        "0",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "stderr:\n{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("1 of the sizes did not finish"));
    assert!(text(&output.stdout).contains("300 residents: NOT MEASURED."));
    let report = report(&out);
    assert_eq!(report["result"]["runs"], serde_json::json!([]));
    let failure = &report["result"]["failures"][0];
    assert_eq!(failure["residents"], 300);
    assert_eq!(failure["reason"], "The run did not finish in 0 seconds and was stopped.");
}

#[test]
fn a_run_above_the_memory_limit_is_reported_as_not_measured() {
    let scratch = Scratch::new("memory");
    let out = scratch.path("bench.json");
    // One millionth of a gibibyte is about 1,000 bytes.
    let output = run(&[
        "bench",
        "--residents",
        "300",
        "--frames",
        "5",
        "--warmup",
        "0",
        "--max-memory-gib",
        "0.000001",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1), "stderr:\n{}", text(&output.stderr));
    assert!(
        text(&output.stdout)
            .contains("300 residents: NOT MEASURED. The run stopped after 0 of 5 frames")
    );
    let report = report(&out);
    assert_eq!(report["result"]["runs"], serde_json::json!([]));
    let failure = &report["result"]["failures"][0];
    assert_eq!(failure["residents"], 300);
    let reason = failure["reason"].as_str().expect("text");
    assert!(reason.contains("more than the limit of 0.0 GiB"), "{reason}");
}

#[test]
fn a_data_file_on_the_command_line_changes_the_game() {
    let scratch = Scratch::new("data");
    let data = scratch.path("override.toml");
    std::fs::write(&data, "[params.\"kernel.inbox_max\"]\nvalue = 1\n")
        .expect("the file is written");
    let out = scratch.path("bench.json");
    let output = run(&[
        "bench",
        "--scale",
        "200",
        "--frames",
        "50",
        "--warmup",
        "10",
        "--data",
        data.to_str().expect("a UTF-8 path"),
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert!(output.status.success(), "stderr:\n{}", text(&output.stderr));
    // The option reaches the child process of the size: one entry fills
    // an inbox.
    let report = report(&out);
    assert_eq!(report["result"]["runs"][0]["memory"]["inbox_fill"], 1.0);

    let missing = run(&["soak", "--ticks", "1", "--residents", "5", "--data", "no/such/file.toml"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        text(&missing.stderr).contains("cannot read the data file no/such/file.toml"),
        "{}",
        text(&missing.stderr)
    );
}

#[test]
fn a_size_that_fails_does_not_stop_the_other_sizes() {
    let scratch = Scratch::new("fail");
    let out = scratch.path("bench.json");
    // 100 by 100 districts are more zones than the kernel allows: every
    // size fails, and each one is reported.
    let output = run(&[
        "bench",
        "--scale",
        "100,200",
        "--frames",
        "2",
        "--districts",
        "100",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains("10000 zones"), "stderr:\n{}", text(&output.stderr));
    let report = report(&out);
    let failures = report["result"]["failures"].as_array().expect("a list");
    let sizes: Vec<u64> =
        failures.iter().map(|f| f["residents"].as_u64().expect("a number")).collect();
    assert_eq!(sizes, [100, 200]);
    // The reason of the child process is in the report of the parent.
    let reason = failures[0]["reason"].as_str().expect("text");
    assert!(reason.contains("the world has 10000 zones, the limit is 8190"), "{reason}");
}

#[test]
fn soak_passes_with_exit_code_zero_and_writes_a_report() {
    let scratch = Scratch::new("soak");
    let out = scratch.path("soak.json");
    let output = run(&[
        "soak",
        "--ticks",
        "30",
        "--residents",
        "150",
        "--verify-determinism",
        "--hash-every",
        "10",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert!(output.status.success(), "stderr:\n{}", text(&output.stderr));
    assert!(text(&output.stdout).contains("result: PASSED"));
    let report = report(&out);
    assert_eq!(report["command"], "soak");
    let result = &report["result"];
    assert_eq!(result["ticks_run"], 30);
    assert_eq!(result["audits_passed"], 31);
    assert_eq!(result["hash_checks_passed"], 4);
    assert_eq!(result["violation"], Value::Null);
}

#[test]
fn a_profile_that_the_world_cannot_run_stops_the_command() {
    let scratch = Scratch::new("profile");
    let profile = scratch.path("future.toml");
    std::fs::write(
        &profile,
        "name = \"future\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"fly\"\nweight = 1\n",
    )
    .expect("the profile is written");
    let output = run(&[
        "soak",
        "--ticks",
        "5",
        "--residents",
        "10",
        "--profile",
        profile.to_str().expect("a UTF-8 path"),
    ]);
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(stderr.contains("the world has none of the verbs of profile `future`"), "{stderr}");
    assert!(stderr.contains("fly"), "{stderr}");
}

#[test]
fn a_profile_with_an_argument_that_the_verb_does_not_have_stops_the_command() {
    let scratch = Scratch::new("argument");
    let profile = scratch.path("typo.toml");
    // `wait` takes `ticks`, not `minutes`.
    std::fs::write(
        &profile,
        "name = \"typo\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"wait\"\nweight = 1\nargs = { minutes = 3 }\n",
    )
    .expect("the profile is written");
    for command in ["soak", "bench"] {
        let output = run(&[
            command,
            "--residents",
            "10",
            "--profile",
            profile.to_str().expect("a UTF-8 path"),
        ]);
        assert_eq!(output.status.code(), Some(1), "`{command}`");
        let all = format!("{}{}", text(&output.stdout), text(&output.stderr));
        assert!(all.contains("the verb `wait` has no argument `minutes`"), "`{command}`: {all}");
        assert!(all.contains("the verb `wait` needs the argument `ticks`"), "`{command}`: {all}");
    }
}

#[test]
fn soak_with_the_daily_life_profile_reports_the_money_and_each_refusal() {
    let scratch = Scratch::new("daily");
    let out = scratch.path("soak.json");
    let output = run(&[
        "soak",
        "--ticks",
        "400",
        "--residents",
        "120",
        "--profile",
        "daily_life",
        "--placement",
        "spawn",
        "--verify-determinism",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert!(output.status.success(), "stderr:\n{}", text(&output.stderr));
    let summary = text(&output.stdout);
    assert!(summary.contains("result: PASSED"), "{summary}");
    assert!(summary.contains("rejected at once: "), "{summary}");
    assert!(summary.contains("money faucets: {"), "{summary}");
    // No warning: every verb and every site kind of the profile exists.
    assert!(!summary.contains("warning:"), "{summary}");
    assert!(!text(&output.stderr).contains("warning:"), "{}", text(&output.stderr));

    let report = report(&out);
    let result = &report["result"];
    assert_eq!(result["profile"]["name"], "daily_life");
    assert_eq!(result["profile"]["skipped"], serde_json::json!([]));
    assert_eq!(result["audits_passed"], 401);
    // Hashes at ticks 0, 50, ..., 400.
    assert_eq!(result["hash_checks_passed"], 9);
    assert_eq!(result["violation"], Value::Null);
    let money = &result["money"];
    let minted = money["minted"].as_i64().expect("a number");
    let burned = money["burned"].as_i64().expect("a number");
    assert!(minted > 0, "{money}");
    assert_eq!(
        money["held"].as_i64().expect("a number") + money["in_flight"].as_i64().expect("a number"),
        minted - burned,
        "{money}"
    );
    // The rejections by code of each verb are in the report file.
    let by_verb = result["actions"]["by_verb"].as_object().expect("a table");
    assert!(by_verb.contains_key("work") && by_verb.contains_key("say"), "{by_verb:?}");
    assert!(by_verb["work"]["rejected_by_code"].is_object());
}

#[test]
fn swarm_needs_a_server() {
    let output = run(&["swarm", "--agents", "2"]);
    assert_eq!(output.status.code(), Some(2), "a usage error");
    assert!(text(&output.stderr).contains("--url"), "{}", text(&output.stderr));

    let both = run(&["swarm", "--url", "http://127.0.0.1:1", "--spawn-server"]);
    assert_eq!(both.status.code(), Some(2));
}

#[test]
fn swarm_stops_before_it_starts_when_the_limit_of_open_files_is_too_low() {
    // 200 agents with a connection each, and a server in the same process
    // that holds the other end: 2 * 200 files, and 256 for the process.
    let output = run_with_open_files(
        300,
        &["swarm", "--spawn-server", "--agents", "200", "--ws-share", "0", "--duration", "1"],
    );
    assert_eq!(output.status.code(), Some(1));
    let stderr = text(&output.stderr);
    assert!(
        stderr.contains("This run needs about 656 open files for 200 connections."),
        "{stderr}"
    );
    assert!(stderr.contains("This process can open 300 (ulimit -n is 300"), "{stderr}");
    assert!(stderr.contains("`ulimit -n 656`"), "{stderr}");
    // No agent started.
    assert!(!stderr.contains("agents against"), "{stderr}");
}

#[test]
fn swarm_stops_with_a_clear_message_when_it_reaches_the_limit_of_open_files() {
    let scratch = Scratch::new("files");
    let out = scratch.path("swarm.json");
    // A shared pool holds no connection for the whole run, so the check
    // before the start passes with 256 files. Then 2,000 agents sign up in
    // the same moment, and each sign-up opens a connection.
    let output = run_with_open_files(
        256,
        &[
            "swarm",
            "--spawn-server",
            "--agents",
            "2000",
            "--ws-share",
            "0",
            "--http-connections",
            "shared",
            "--ramp-up",
            "0",
            "--duration",
            "30",
            "--tick-ms",
            "50",
            "--out",
            out.to_str().expect("a UTF-8 path"),
        ],
    );
    let stderr = text(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.contains(
            "The run stopped at a limit of this machine: the process cannot open one more file"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("The limit is 256 (ulimit -n)"), "{stderr}");
    let report = report(&out);
    let result = &report["result"];
    assert_eq!(result["sustained"], false);
    let limit = result["machine_limit"].as_str().expect("the report names the limit");
    assert!(limit.starts_with("The run stopped at a limit of this machine"), "{limit}");
    assert_eq!(result["not_sustained_because"][0], limit);
    assert!(result["error_examples"]["too_many_open_files"].is_string(), "{result}");
    // The run did not go on for its 30 seconds.
    assert!(result["duration_secs"].as_f64().expect("a number") < 25.0, "{result}");
}

#[test]
fn swarm_runs_against_a_server_that_it_starts() {
    let scratch = Scratch::new("swarm");
    let out = scratch.path("swarm.json");
    let output = run(&[
        "swarm",
        "--spawn-server",
        "--agents",
        "4",
        "--ramp-up",
        "0.2",
        "--duration",
        "1.5",
        "--think",
        "constant",
        "--think-mean",
        "0.05",
        "--tick-ms",
        "20",
        "--p99-target-ms",
        "8000",
        "--require-sustained",
        "--out",
        out.to_str().expect("a UTF-8 path"),
    ]);
    assert!(output.status.success(), "stderr:\n{}", text(&output.stderr));
    let summary = text(&output.stdout);
    assert!(summary.contains("result: SUSTAINED"), "{summary}");
    let report = report(&out);
    assert_eq!(report["command"], "swarm");
    let result = &report["result"];
    assert_eq!(result["agents"], 4);
    assert_eq!(result["agents_ready"], 4);
    assert_eq!(result["sustained"], true);
    assert_eq!(result["failed_requests"], 0);
    assert_eq!(result["throttled_requests"], 0);
    assert!(result["acks"].as_u64().expect("a number") > 4);
    assert_eq!(result["server_stats"]["sim"]["residents"], 4);
    // The work of the one simulation thread in the window.
    assert!(result["server_stats"]["sim"]["apply_ms"].as_f64().expect("a number") > 0.0);
    assert!(summary.contains("simulation thread: "), "{summary}");
    assert!(
        summary.contains("sign-up: 4 with POST /v1/owners and POST /v1/residents"),
        "{summary}"
    );
    assert_eq!(result["signup"]["count"], 4);
    assert!(
        result["connections"]["limits"]["soft"].is_object()
            || result["connections"]["limits"]["soft"].is_string()
    );
}
