//! A tiny `bench` run through the library.

use unsup_loadtest::bench::{self, BenchConfig, BenchReport};
use unsup_loadtest::local::{Placement, Shape, WorldSpec};
use unsup_loadtest::profile::Profile;

fn config(profile: &str, residents: u32, frames: u32, warmup: u32) -> BenchConfig {
    BenchConfig {
        world: WorldSpec {
            seed: 4,
            residents,
            shape: Shape { cities: 1, districts: 2 },
            placement: Placement::Scatter,
            data: Vec::new(),
        },
        frames,
        warmup,
        profile: Profile::load(profile).expect("a built-in profile"),
        request_ids: false,
        max_memory_bytes: None,
    }
}

#[test]
fn a_tiny_bench_measures_every_frame_and_counts_every_action() {
    let run = bench::run(&config("walkers", 400, 40, 10), &[]).expect("the bench runs");

    assert_eq!(run.residents, 400);
    assert_eq!(run.residents_end, 400, "walking removes nobody");
    assert_eq!((run.cities, run.districts, run.zones), (1, 2, 4));
    assert_eq!(run.placement, Placement::Scatter);
    assert_eq!((run.warmup_frames, run.frames), (10, 40));
    // One sample for each measured frame, in every series.
    for (name, series) in [
        ("frame", &run.frame_ms),
        ("inputs", &run.inputs_ms),
        ("observe", &run.observe_ms),
        ("server", &run.server_ms),
        ("generator", &run.generator_ms),
    ] {
        assert_eq!(series.count, 40, "series `{name}`");
        assert!(series.min <= series.p50 && series.p50 <= series.p99 && series.p99 <= series.max);
    }
    assert!(run.frame_ms.max > 0.0, "a frame takes time");
    // The server work of a frame holds its three parts.
    assert!(run.server_ms.mean >= run.frame_ms.mean);

    // Walkers: one action about every 13 ticks for each resident. 400
    // residents in 40 frames send about 1,200 actions; all are valid.
    let actions = &run.actions;
    assert!((800..1_800).contains(&actions.sent), "{} actions were sent", actions.sent);
    assert_eq!(actions.rejected, 0, "rejections: {:?}", actions.rejected_by_code);
    assert_eq!(actions.accepted(), actions.sent);
    assert_eq!(actions.started, actions.sent, "`move_to` and `wait` both take time");
    let verbs: Vec<&str> = actions.by_verb.keys().map(String::as_str).collect();
    assert_eq!(verbs, ["move_to", "wait"]);
    let moves = actions.by_verb["move_to"].sent;
    let waits = actions.by_verb["wait"].sent;
    assert_eq!(moves + waits, actions.sent);
    assert!(moves > 5 * waits, "weights 9 and 1: {moves} walks and {waits} waits");
    assert!((run.actions_per_frame - actions.sent as f64 / 40.0).abs() < 1e-9);
    assert!(run.actions_per_wall_second > 0.0);
    assert!(run.actions_per_server_second >= run.actions_per_wall_second);
    // A quarter of the actions is followed by an observation.
    assert!(actions.observes > actions.sent / 8 && actions.observes < actions.sent / 2);
    // Residents moved, and some crossed a district border.
    assert!(run.moves_per_frame > 5.0, "moves for each frame: {}", run.moves_per_frame);
    assert!(run.migrations_per_frame > 0.0);
    assert!(run.fullest_zone < 400, "the scatter placement uses every zone");

    assert_eq!(run.profile.name, "walkers");
    assert_eq!(run.profile.verbs, [("move_to".to_string(), 9), ("wait".to_string(), 1)]);
    assert!(run.profile.skipped.is_empty());
    let rss = run.memory.rss_end_bytes.expect("the system reports memory");
    assert!(rss > run.memory.rss_baseline_bytes.expect("a baseline"), "a world needs memory");
    assert!(run.memory.kernel_estimate_bytes_per_resident > 0);
}

#[test]
fn the_rush_profile_sends_more_actions_than_the_walkers_profile() {
    let walkers = bench::run(&config("walkers", 300, 30, 10), &[]).expect("the bench runs");
    let rush = bench::run(&config("rush", 300, 30, 10), &[]).expect("the bench runs");
    // Rush: one action about every 3.3 ticks, walkers: about every 13.
    assert!(
        rush.actions_per_frame > 2.5 * walkers.actions_per_frame,
        "rush {} and walkers {} actions for each frame",
        rush.actions_per_frame,
        walkers.actions_per_frame
    );
    assert_eq!(rush.actions.rejected, 0);
}

#[test]
fn residents_that_stay_at_the_spawn_point_are_all_in_one_zone() {
    let mut config = config("walkers", 200, 5, 0);
    config.world.placement = Placement::Spawn;
    let run = bench::run(&config, &[]).expect("the bench runs");
    assert_eq!(run.placement, Placement::Spawn);
    // The world of today has no spawn sites: everybody starts at the
    // centre of the first zone. When the game gets spawn sites, the
    // residents are spread over them instead.
    if run.spawn_sites == 0 {
        assert!(run.fullest_zone > 150, "fullest zone: {}", run.fullest_zone);
    } else {
        assert!(run.fullest_zone <= 200);
    }
}

#[test]
fn request_ids_are_sent_when_asked_and_the_run_still_balances() {
    let mut config = config("walkers", 100, 20, 0);
    config.request_ids = true;
    let run = bench::run(&config, &[]).expect("the bench runs");
    assert!(run.request_ids);
    assert_eq!(run.actions.rejected, 0);
    assert!(run.actions.sent > 100);
}

#[test]
fn a_run_above_the_memory_limit_stops_before_the_first_frame_with_a_clear_message() {
    let mut config = config("walkers", 100, 20, 5);
    // No process fits in one byte.
    config.max_memory_bytes = Some(1);
    let error = bench::run(&config, &[]).expect_err("the process holds more than one byte");
    let message = format!("{error:#}");
    assert!(message.starts_with("The run stopped after 0 of 25 frames"), "{message}");
    assert!(message.contains("more than the limit of 0.0 GiB"), "{message}");
    assert!(message.contains("--max-memory-gib"), "{message}");

    // The same run with a limit that it does not reach finishes, and the
    // report holds the limit.
    config.max_memory_bytes = Some(1 << 40);
    let run = bench::run(&config, &[]).expect("the bench runs");
    assert_eq!(run.memory.limit_bytes, Some(1 << 40));
    assert_eq!(run.frame_ms.count, 20);
}

#[test]
fn the_memory_of_a_run_is_measured_with_and_without_squeezed_pages() {
    let run = bench::run(&config("walkers", 400, 40, 10), &[]).expect("the bench runs");
    let m = &run.memory;
    let rss = m.rss_end_bytes.expect("the system reports the resident set size");
    let baseline = m.rss_baseline_bytes.expect("a baseline");
    assert_eq!(m.bytes_per_resident, Some(rss.saturating_sub(baseline) / 400));
    assert!(m.rss_peak_bytes.expect("the sampler ran") >= m.rss_populated_bytes.expect("noted"));
    let footprint = m.footprint_end_bytes.expect("the system reports the footprint");
    assert!(footprint > 1_000_000, "a test process owns more than a megabyte: {footprint}");
    assert!(m.footprint_peak_bytes.expect("the system keeps the peak") >= footprint);
    assert_eq!(m.footprint_bytes_per_resident, Some(footprint.saturating_sub(baseline) / 400));
    assert_eq!(m.squeezed_bytes, Some(footprint.saturating_sub(rss)));
    assert!(run.summary().contains("footprint (RSS with compressed and swapped pages)"));
}

#[test]
fn a_data_file_replaces_a_number_of_the_game_and_the_inboxes_tell_how_full_they_are() {
    // The game as it ships keeps 32 notices and reports for each resident.
    // After 60 frames a walker has ended about 4 actions: the inboxes are
    // far from full.
    let shipped = bench::run(&config("walkers", 200, 50, 10), &[]).expect("the bench runs");
    let fill = shipped.memory.inbox_fill.expect("the world has `kernel.inbox_max`");
    assert!(fill > 0.02 && fill < 0.5, "the inboxes are {fill} full");
    assert!(shipped.summary().contains("the memory was still growing"));

    // With room for one entry, every inbox is full after the first action
    // that ended: one entry of one for every resident.
    let mut small = config("walkers", 200, 50, 10);
    small.world.data = vec!["[params.\"kernel.inbox_max\"]\nvalue = 1\n".to_string()];
    let run = bench::run(&small, &[]).expect("the bench runs");
    assert_eq!(run.memory.inbox_fill, Some(1.0));
    assert!(run.summary().contains("inboxes: 100 % full\n"), "{}", run.summary());

    // A data file that is not valid stops the run and names the reason.
    let mut broken = config("walkers", 10, 1, 0);
    broken.world.data = vec!["[params.\"kernel.inbox_max\"]\nvalue = 0\n".to_string()];
    let error = bench::run(&broken, &[]).expect_err("0 is below the least value");
    assert!(format!("{error:#}").contains("kernel.inbox_max"), "{error:#}");
}

#[test]
fn a_profile_without_a_known_verb_stops_the_bench_with_a_clear_message() {
    let text = "name = \"future\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"teleport_home\"\nweight = 1\n";
    let mut config = config("walkers", 10, 1, 0);
    config.profile = Profile::parse(text).expect("the profile parses");
    let error = bench::run(&config, &[]).expect_err("no verb of the profile exists");
    let message = format!("{error:#}");
    assert!(message.contains("none of the verbs of profile `future`"), "{message}");
    assert!(message.contains("teleport_home"), "{message}");
}

#[test]
fn a_world_that_the_kernel_cannot_hold_is_refused() {
    let mut config = config("walkers", 10, 1, 0);
    // 100 by 100 districts are 10,000 zones; the kernel allows 8,190.
    config.world.shape = Shape { cities: 1, districts: 100 };
    let error = bench::run(&config, &[]).expect_err("too many zones");
    let message = format!("{error:#}");
    assert!(message.contains("10000 zones"), "{message}");
    assert!(message.contains("The world would have 1 city of 100 x 100 districts"), "{message}");
}

#[test]
fn the_summary_names_every_size_and_every_failure() {
    let run = bench::run(&config("walkers", 50, 3, 0), &[]).expect("the bench runs");
    let report = BenchReport {
        runs: vec![run.clone(), run],
        failures: vec![bench::ScaleFailure {
            residents: 5_000_000,
            reason: "The run did not finish in 10 seconds and was stopped.".into(),
        }],
    };
    let summary = report.summary();
    assert_eq!(summary.matches("50 residents in 4 zones").count(), 2, "{summary}");
    assert!(summary.contains("residents | zones | frame ms"), "{summary}");
    assert!(
        summary.contains("5,000,000 residents: NOT MEASURED. The run did not finish"),
        "{summary}"
    );
}
