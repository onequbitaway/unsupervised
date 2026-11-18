//! The profile parser and the match with the catalogue of a world.

use std::collections::BTreeMap;

use serde_json::json;
use unsup_kernel::Rng;
use unsup_loadtest::local::{Shape, build_world};
use unsup_loadtest::profile::Profile;
use unsup_loadtest::scene::ListScene;
use unsup_loadtest::think::TickThink;
use unsup_protocol::{Catalogue, Pos, ResidentId, VerbInfo, VerbScope};

/// A catalogue with the given verbs. Each takes one argument `n`.
fn catalogue(verbs: &[&str]) -> Catalogue {
    Catalogue {
        verbs: verbs
            .iter()
            .map(|verb| VerbInfo {
                verb: (*verb).to_string(),
                module: "test".into(),
                summary: String::new(),
                free: false,
                scope: VerbScope::Zone,
                args_schema: json!({
                    "type": "object",
                    "properties": { "n": { "type": "integer" } },
                    "required": ["n"],
                }),
            })
            .collect(),
        ..Catalogue::default()
    }
}

const WEIGHTED: &str = r#"
name = "weighted"
observe_share = 0.5

[think]
kind = "constant"
ticks = 3

[[action]]
verb = "often"
weight = 3
args = { n = 1 }

[[action]]
verb = "seldom"
weight = 1
args = { n = 2 }

[[action]]
verb = "never"
weight = 0
args = { n = 3 }

[[action]]
verb = "from_the_future"
weight = 100
args = { n = 4 }
"#;

fn scene() -> ListScene {
    ListScene::new(ResidentId(1), Pos::new(0, 0), None)
}

#[test]
fn a_profile_file_is_read() {
    let profile = Profile::parse(WEIGHTED).expect("the profile parses");
    assert_eq!(profile.name, "weighted");
    assert_eq!(profile.think, TickThink::Constant { ticks: 3 });
    assert!((profile.observe_share - 0.5).abs() < f64::EPSILON);
    assert!(profile.wait_share.abs() < f64::EPSILON, "a share that is not written is 0");
    let verbs: Vec<(&str, u32)> =
        profile.actions.iter().map(|action| (action.verb.as_str(), action.weight)).collect();
    assert_eq!(verbs, [("often", 3), ("seldom", 1), ("never", 0), ("from_the_future", 100)]);
}

#[test]
fn a_verb_that_the_world_does_not_have_is_skipped_with_a_warning() {
    let profile = Profile::parse(WEIGHTED).expect("the profile parses");
    let resolved =
        profile.resolve(&catalogue(&["often", "seldom", "never"])).expect("some verbs exist");
    assert_eq!(resolved.skipped, ["from_the_future"]);
    assert_eq!(resolved.verbs(), [("often", 3), ("seldom", 1)]);
    assert_eq!(resolved.warnings.len(), 1, "warnings: {:?}", resolved.warnings);
    assert!(resolved.warnings[0].contains("no verb `from_the_future`"));
    assert!(resolved.warnings[0].contains("skipped"));
}

#[test]
fn weights_set_how_often_each_action_is_chosen() {
    let profile = Profile::parse(WEIGHTED).expect("the profile parses");
    let resolved =
        profile.resolve(&catalogue(&["often", "seldom", "never"])).expect("some verbs exist");
    let mut rng = Rng::seeded(7);
    let mut scene = scene();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    let draws = 40_000;
    for _ in 0..draws {
        let choice = resolved.pick(&mut rng, &mut scene).expect("an action is always possible");
        // Each action carries its own arguments.
        let n = match choice.verb.as_str() {
            "often" => 1,
            "seldom" => 2,
            other => panic!("`{other}` must not be chosen"),
        };
        assert_eq!(choice.args, json!({ "n": n }));
        *counts.entry(choice.verb).or_default() += 1;
    }
    // Weights 3 and 1: three quarters and one quarter. The standard
    // deviation of the count is 87, so 600 is far outside chance.
    let often = counts["often"];
    let seldom = counts["seldom"];
    assert_eq!(often + seldom, draws);
    assert!(often.abs_diff(30_000) < 600, "`often` was chosen {often} times of {draws}");
    assert!(seldom.abs_diff(10_000) < 600, "`seldom` was chosen {seldom} times of {draws}");
    assert_eq!(counts.len(), 2, "weight 0 and unknown verbs are never chosen: {counts:?}");
}

#[test]
fn the_weights_of_the_other_actions_do_not_change_when_one_is_skipped() {
    let profile = Profile::parse(WEIGHTED).expect("the profile parses");
    // Without `often`, only `seldom` is left: it is chosen every time.
    let resolved = profile.resolve(&catalogue(&["seldom"])).expect("one verb exists");
    assert_eq!(resolved.skipped, ["often", "never", "from_the_future"]);
    let mut rng = Rng::seeded(1);
    let mut scene = scene();
    for _ in 0..100 {
        assert_eq!(resolved.pick(&mut rng, &mut scene).expect("an action").verb, "seldom");
    }
}

#[test]
fn a_world_with_none_of_the_verbs_is_an_error() {
    let profile = Profile::parse(WEIGHTED).expect("the profile parses");
    let error = profile.resolve(&catalogue(&["other"])).expect_err("no verb matches");
    assert!(error.message.contains("none of the verbs of profile `weighted`"), "{error}");
    assert!(error.message.contains("often, seldom, never, from_the_future"), "{error}");
    // A verb with weight 0 alone is not enough.
    let error = profile.resolve(&catalogue(&["never"])).expect_err("only weight 0 matches");
    assert!(error.message.contains("none of the verbs"), "{error}");
}

#[test]
fn arguments_that_do_not_match_the_catalogue_give_a_warning() {
    let text = r#"
name = "mismatch"
[think]
kind = "constant"
ticks = 1
[[action]]
verb = "often"
weight = 1
args = { count = 1 }
"#;
    let profile = Profile::parse(text).expect("the profile parses");
    let resolved = profile.resolve(&catalogue(&["often"])).expect("the verb exists");
    assert!(resolved.skipped.is_empty());
    assert_eq!(
        resolved.warnings,
        [
            "profile `mismatch`: the verb `often` needs the argument `n`, which the profile does not give",
            "profile `mismatch`: the verb `often` has no argument `count`",
        ]
    );
}

#[test]
fn invalid_profiles_are_refused_with_the_reason() {
    let base = "name = \"p\"\n[think]\nkind = \"constant\"\nticks = 1\n";
    let message = |text: &str| Profile::parse(text).expect_err("the profile is invalid").message;

    assert!(message(base).contains("action"), "a profile without actions");
    let zero = format!("{base}[[action]]\nverb = \"wait\"\nweight = 0\n");
    assert!(message(&zero).contains("every action has weight 0"));
    let share = format!("observe_share = 1.5\n{base}[[action]]\nverb = \"wait\"\nweight = 1\n");
    assert!(message(&share).contains("observe_share is 1.5"));
    let think = "name = \"p\"\n[think]\nkind = \"uniform\"\nmin = 5\nmax = 2\n[[action]]\nverb = \"wait\"\nweight = 1\n";
    assert!(message(think).contains("min (5) is greater than max (2)"));
    let unknown_gen = format!(
        "{base}[[action]]\nverb = \"wait\"\nweight = 1\nargs = {{ ticks = {{ gen = \"dice\" }} }}\n"
    );
    assert!(message(&unknown_gen).contains("dice"), "{}", message(&unknown_gen));
    let typo = format!("{base}[[action]]\nverb = \"wait\"\nwieght = 1\n");
    assert!(message(&typo).contains("wieght"), "{}", message(&typo));
    let nameless = "name = \" \"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"wait\"\nweight = 1\n";
    assert!(message(nameless).contains("no name"));
}

#[test]
fn the_built_in_profiles_work_with_the_world_of_the_server() {
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let catalogue = world.catalogue();
    let known: Vec<&str> = catalogue.verbs.iter().map(|verb| verb.verb.as_str()).collect();
    assert_eq!(Profile::builtin_names(), ["walkers", "rush", "daily_life"]);
    for name in Profile::builtin_names() {
        let profile = Profile::load(name).unwrap_or_else(|e| panic!("profile `{name}`: {e}"));
        assert_eq!(profile.name, name);
        let resolved =
            profile.resolve(&catalogue).unwrap_or_else(|e| panic!("profile `{name}`: {e}"));
        // Every verb that is left exists in the world, and every skipped
        // verb does not.
        for (verb, _) in resolved.verbs() {
            assert!(known.contains(&verb), "profile `{name}` uses the unknown verb `{verb}`");
        }
        for verb in &resolved.skipped {
            assert!(!known.contains(&verb.as_str()), "profile `{name}` skipped `{verb}`");
        }
        // The arguments of the verbs that exist match the catalogue: every
        // warning is about a skipped verb.
        for warning in &resolved.warnings {
            assert!(warning.contains("is skipped"), "profile `{name}`: {warning}");
        }
    }
    // The profiles for the world of today use no verb that is missing.
    for name in ["walkers", "rush"] {
        let resolved = Profile::load(name).expect("built in").resolve(&catalogue).expect("verbs");
        assert!(resolved.skipped.is_empty(), "`{name}` skipped {:?}", resolved.skipped);
    }
}

#[test]
fn a_profile_is_loaded_from_a_file_and_a_missing_file_names_the_built_in_ones() {
    let dir = std::env::temp_dir().join(format!("unsup-profile-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the directory is made");
    let path = dir.join("mine.toml");
    std::fs::write(&path, WEIGHTED).expect("the file is written");
    let profile = Profile::load(path.to_str().expect("a UTF-8 path")).expect("the file loads");
    assert_eq!(profile.name, "weighted");
    std::fs::remove_dir_all(&dir).expect("the directory is removed");

    let error = Profile::load("no/such/profile.toml").expect_err("the file is missing");
    assert!(error.message.contains("no/such/profile.toml"), "{error}");
    assert!(error.message.contains("walkers, rush, daily_life"), "{error}");
}
