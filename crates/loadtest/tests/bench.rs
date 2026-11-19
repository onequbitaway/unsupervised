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
        observe_sections: Vec::new(),
        observe_batch: 0,
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

    // Walkers: a walk of up to 12 tiles on each axis, around walls and over
    // grass, then 0 to 10 ticks of thinking. 400 residents in 40 frames
    // send several hundred actions.
    let actions = &run.actions;
    assert!((300..1_800).contains(&actions.sent), "{} actions were sent", actions.sent);
    // Every target is a walkable tile of the city, and every resident was
    // idle at its turn: the world has no reason to reject a request.
    assert_eq!(actions.rejected, 0, "rejections: {:?}", actions.rejected_by_code);
    assert!(actions.rejection_rate().abs() < f64::EPSILON);
    assert_eq!(actions.accepted(), actions.sent);
    // A walk in one district is `started`, a walk into another district is
    // `pending`, a walk to the own tile is `done`. Nothing is queued: a
    // resident acts only when it is idle.
    assert_eq!(actions.started + actions.pending + actions.done, actions.sent);
    assert_eq!(actions.queued, 0);
    assert!(actions.started > actions.pending, "most walks stay in one district");
    assert!(actions.pending > 0, "some walks cross a district border");
    let verbs: Vec<&str> = actions.by_verb.keys().map(String::as_str).collect();
    assert_eq!(verbs, ["move_to", "wait"]);
    let moves = actions.by_verb["move_to"].sent;
    let waits = actions.by_verb["wait"].sent;
    assert_eq!(moves + waits, actions.sent);
    assert!(moves > 5 * waits, "weights 9 and 1: {moves} walks and {waits} waits");
    assert!((run.actions_per_frame - actions.sent as f64 / 40.0).abs() < 1e-9);
    assert!(run.actions_per_wall_second > 0.0);
    assert!(run.actions_per_server_second >= run.actions_per_wall_second);
    // The cost of one call is the sum of its series over the calls.
    let total_micros = |series: &unsup_loadtest::stats::Millis| series.mean * 40.0 * 1e3;
    let per_request = total_micros(&run.inputs_ms) / actions.sent as f64;
    assert!(run.micros_per_request > 0.0);
    assert!((run.micros_per_request - per_request).abs() < per_request * 1e-6);
    let per_observation = total_micros(&run.observe_ms) / actions.observes as f64;
    assert!((run.micros_per_observation - per_observation).abs() < per_observation * 1e-6);
    // A quarter of the actions is followed by an observation.
    assert!(actions.observes > actions.sent / 8 && actions.observes < actions.sent / 2);
    // The observations hold the reports of the walks that ended, and every
    // walk reached its target.
    assert!(actions.reports_finished > 0, "no report was read");
    assert_eq!(actions.reports_failed, 0, "failed walks: {:?}", actions.failed_by_code);
    assert_eq!(
        actions.by_verb["move_to"].finished + actions.by_verb["wait"].finished,
        actions.reports_finished
    );
    // Residents moved, and some crossed a district border.
    assert!(run.moves_per_frame > 5.0, "moves for each frame: {}", run.moves_per_frame);
    assert!(run.migrations_per_frame > 0.0);
    assert!(run.fullest_zone < 400, "the scatter placement uses every zone");
    let summary = run.summary();
    assert!(summary.contains("rejected at once: 0 of"), "{summary}");
    assert!(summary.contains("reports read: "), "{summary}");

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
    // Rush: walks of at most 3 tiles and 0 to 2 ticks of thinking. Walkers:
    // walks of at most 12 tiles and 0 to 10 ticks.
    assert!(
        rush.actions_per_frame > 2.5 * walkers.actions_per_frame,
        "rush {} and walkers {} actions for each frame",
        rush.actions_per_frame,
        walkers.actions_per_frame
    );
    assert_eq!(rush.actions.rejected, 0, "rejections: {:?}", rush.actions.rejected_by_code);
}

#[test]
fn residents_that_stay_at_the_spawn_point_are_in_the_spawn_sites_of_the_city() {
    let mut config = config("walkers", 200, 5, 0);
    config.world.placement = Placement::Spawn;
    let run = bench::run(&config, &[]).expect("the bench runs");
    assert_eq!(run.placement, Placement::Spawn);
    // A city of 2 by 2 districts has two shelters and one station. The
    // game spreads new residents over them, so no zone has all of them.
    assert_eq!(run.spawn_sites, 3);
    assert!(run.fullest_zone < 200, "fullest zone: {}", run.fullest_zone);
    assert!(run.fullest_zone >= 200 / 3, "fullest zone: {}", run.fullest_zone);
}

#[test]
fn the_daily_life_profile_runs_the_verbs_of_the_full_game_and_reports_each_refusal() {
    // Residents that start in the spawn sites, as newcomers do. 300 frames
    // are longer than one shift of public works (240 ticks).
    let mut config = config("daily_life", 300, 300, 0);
    config.world.shape = Shape { cities: 1, districts: 1 };
    config.world.placement = Placement::Spawn;
    let run = bench::run(&config, &[]).expect("the bench runs and the world balances");
    let actions = &run.actions;
    assert!(run.profile.skipped.is_empty(), "skipped: {:?}", run.profile.skipped);
    assert!(run.profile.warnings.is_empty(), "warnings: {:?}", run.profile.warnings);
    assert_eq!(run.residents_end, 300, "nobody dies in five game hours");

    // The world accepted each of these verbs at least one time: the verb
    // and its arguments are those of the game.
    for verb in [
        "move_to",
        "wait",
        "work",
        "gather",
        "sell",
        "say",
        "message",
        "inspect",
        "list_jobs",
        "post",
        "relate",
        "write_will",
        "list_property",
    ] {
        let counts = actions.by_verb.get(verb).unwrap_or_else(|| panic!("`{verb}` was not sent"));
        assert!(counts.sent > counts.rejected, "`{verb}`: {counts:?}");
    }
    // No request had arguments that the verb does not take.
    assert_eq!(actions.rejected_by_code.get("invalid_args"), None, "{:?}", actions.by_verb);
    assert_eq!(actions.rejected_by_code.get("unknown_verb"), None);
    assert!(actions.refused_by_sim.is_empty(), "{:?}", actions.refused_by_sim);

    // A refusal is data. The counts by code add up, for the whole run and
    // for each verb.
    assert_eq!(actions.rejected_by_code.values().sum::<u64>(), actions.rejected);
    assert_eq!(actions.failed_by_code.values().sum::<u64>(), actions.reports_failed);
    assert_eq!(actions.accepted() + actions.rejected, actions.sent);
    let (mut sent, mut rejected) = (0, 0);
    for (verb, counts) in &actions.by_verb {
        assert_eq!(counts.rejected_by_code.values().sum::<u64>(), counts.rejected, "`{verb}`");
        assert_eq!(counts.failed_by_code.values().sum::<u64>(), counts.failed, "`{verb}`");
        sent += counts.sent;
        rejected += counts.rejected;
    }
    assert_eq!((sent, rejected), (actions.sent, actions.rejected));
    // An action at a site is a plan: the action waits in the queue behind
    // the walk.
    assert!(actions.queued > 0, "no action was queued behind a walk");
    // Residents read the reports of ended actions, and walks to sites end
    // well: the site ids come from the map.
    assert!(actions.reports_finished > 0);
    assert_eq!(actions.by_verb["move_to"].failed, 0, "{:?}", actions.by_verb["move_to"]);
    let summary = run.summary();
    assert!(summary.contains("    work "), "{summary}");
    assert!(summary.contains(" % rejected"), "{summary}");
}

#[test]
fn an_observation_can_ask_for_one_section_only() {
    // The `still` profile observes after each action. With only the
    // section `self`, the residents read no report; with all sections they
    // read the report of each `wait` that ended.
    let all = bench::run(&config("still", 200, 30, 5), &[]).expect("the bench runs");
    assert!(all.observe_sections.is_empty());
    assert_eq!(all.actions.observes, all.actions.sent, "one observation for each action");
    assert!(all.actions.reports_finished > 0, "all sections have the reports");

    let mut only_self = config("still", 200, 30, 5);
    only_self.observe_sections = vec!["self".to_string()];
    let run = bench::run(&only_self, &[]).expect("the bench runs");
    assert_eq!(run.observe_sections, ["self"]);
    assert_eq!(run.actions.observes, run.actions.sent);
    assert_eq!(run.actions.reports_finished, 0, "the section `reports` was not asked for");
    assert!(run.summary().contains("(only the sections self)"), "{}", run.summary());
}

#[test]
fn observations_in_batches_are_the_observations_one_by_one() {
    // The residents that observe, their order and the answers are the same
    // in both ways. So the two runs count the same actions, observations
    // and reports. A batch of 7 leaves a last batch that is not full.
    let one_by_one = bench::run(&config("daily_life", 300, 40, 10), &[]).expect("the bench runs");
    assert_eq!(one_by_one.observe_batch, 0);
    assert!(
        one_by_one.actions.observes > 100,
        "{} observations were made",
        one_by_one.actions.observes
    );
    assert!(one_by_one.actions.reports_finished > 0, "the residents read reports");
    assert!(
        one_by_one.summary().contains("microseconds in World::observe on one thread"),
        "{}",
        one_by_one.summary()
    );

    let mut batched = config("daily_life", 300, 40, 10);
    batched.observe_batch = 7;
    let run = bench::run(&batched, &[]).expect("the bench runs");
    assert_eq!(run.observe_batch, 7);
    assert_eq!(run.actions, one_by_one.actions);
    assert!(
        run.summary().contains("World::observe_batch on all cores, at most 7 requests in one call"),
        "{}",
        run.summary()
    );
}

#[test]
fn request_ids_are_sent_when_asked_and_the_run_still_balances() {
    let mut config = config("walkers", 100, 20, 0);
    config.request_ids = true;
    let run = bench::run(&config, &[]).expect("the bench runs");
    assert!(run.request_ids);
    assert_eq!(run.actions.rejected, 0, "rejections: {:?}", run.actions.rejected_by_code);
    // 100 residents send about one action in 20 ticks each.
    assert!(run.actions.sent > 50, "{} actions were sent", run.actions.sent);
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
