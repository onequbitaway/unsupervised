//! The profile parser and the match with the catalogue of a world.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use unsup_kernel::Rng;
use unsup_loadtest::local::{Shape, build_world};
use unsup_loadtest::map::WorldMap;
use unsup_loadtest::profile::{Memory, Profile};
use unsup_loadtest::scene::{Carried, ListScene};
use unsup_loadtest::think::TickThink;
use unsup_protocol::{
    AttrInfo, Catalogue, CityId, ItemId, ItemInfo, Pos, ResidentId, SiteId, VerbInfo, VerbScope,
};

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
        let choice = resolved
            .pick(&mut rng, &mut scene, Memory::default())
            .expect("an action is always possible");
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
        let choice = resolved.pick(&mut rng, &mut scene, Memory::default()).expect("an action");
        assert_eq!(choice.verb, "seldom");
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

/// A profile with one action. `rest` is the TOML of the action after its
/// verb and its weight.
fn one_action(verb: &str, rest: &str) -> Profile {
    let text = format!(
        "name = \"one\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"{verb}\"\nweight = 1\n{rest}\n"
    );
    Profile::parse(&text).unwrap_or_else(|error| panic!("the profile parses: {error}\n{text}"))
}

#[test]
fn an_argument_that_the_verb_does_not_have_stops_the_run() {
    // The verb `often` of the test catalogue takes `n` and needs it.
    let wrong = one_action("often", "args = { count = 1 }");
    let error = wrong.resolve(&catalogue(&["often"])).expect_err("the names do not fit");
    assert_eq!(
        error.message,
        "profile `one` does not fit the verbs of the world: the verb `often` has no argument \
         `count`; the verb `often` needs the argument `n`, which the profile does not give"
    );
    // The same action with the right name is accepted.
    let right = one_action("often", "args = { n = 1 }");
    assert!(right.resolve(&catalogue(&["often"])).is_ok());

    // An extra argument beside the right one is refused too: the world
    // would ignore it, and the profile would not do what it says.
    let extra = one_action("often", "args = { n = 1, speed = 3 }");
    let error = extra.resolve(&catalogue(&["often"])).expect_err("`speed` is unknown");
    assert!(error.message.contains("the verb `often` has no argument `speed`"), "{error}");
    assert!(!error.message.contains("needs the argument"), "{error}");

    // A generator inside the table does not hide the name.
    let generated = one_action("often", "args = { m = { gen = \"int\", min = 1, max = 2 } }");
    let error = generated.resolve(&catalogue(&["often"])).expect_err("`m` is unknown");
    assert!(error.message.contains("the verb `often` has no argument `m`"), "{error}");

    // An action with weight 0 is checked too: a later edit can give it a
    // weight.
    let text = "name = \"p\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"often\"\nweight = 1\nargs = { n = 1 }\n[[action]]\nverb = \"seldom\"\nweight = 0\nargs = { typo = 1 }\n";
    let asleep = Profile::parse(text).expect("the profile parses");
    let error = asleep.resolve(&catalogue(&["often", "seldom"])).expect_err("`typo` is unknown");
    assert!(error.message.contains("the verb `seldom` has no argument `typo`"), "{error}");
}

#[test]
fn the_arguments_of_a_generator_that_makes_the_whole_table_are_checked_too() {
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let catalogue = world.catalogue();
    // `near_tile` makes `{ x, y }`: right for `move_to`, wrong for `wait`.
    let walk = one_action("move_to", "args = { gen = \"near_tile\", radius = 3 }");
    assert!(walk.resolve(&catalogue).is_ok());
    let wait = one_action("wait", "args = { gen = \"near_tile\", radius = 3 }");
    let error = wait.resolve(&catalogue).expect_err("`wait` takes `ticks`");
    assert!(error.message.contains("the verb `wait` has no argument `x`"), "{error}");
    assert!(error.message.contains("the verb `wait` has no argument `y`"), "{error}");
    assert!(error.message.contains("the verb `wait` needs the argument `ticks`"), "{error}");
    // `carried` makes `{ item, qty }`: right for `sell`.
    let sell = one_action("sell", "args = { gen = \"carried\", tag = \"raw\" }");
    assert!(sell.resolve(&catalogue).is_ok());
    // A verb that takes no argument refuses every name.
    let work = one_action("work", "args = { hours = 4 }");
    let error = work.resolve(&catalogue).expect_err("`work` takes no argument");
    assert!(error.message.contains("the verb `work` has no argument `hours`"), "{error}");
    // A free verb runs at once, also during a walk: a plan of a walk and
    // a free verb would run the verb before the resident arrives.
    let free = one_action("market_book", "at = [\"exchange\"]\nargs = { item = \"grain\" }");
    let error = free.resolve(&catalogue).expect_err("a free verb cannot wait behind a walk");
    assert!(
        error.message.contains("the verb `market_book` is free and does not wait behind a walk"),
        "{error}"
    );
    assert!(one_action("market_book", "args = { item = \"grain\" }").resolve(&catalogue).is_ok());
    // One value is not a table of arguments.
    let scalar = one_action("wait", "args = { gen = \"int\", min = 1, max = 2 }");
    let error = scalar.resolve(&catalogue).expect_err("a number is not a table");
    assert!(error.message.contains("the arguments of the verb `wait` are not a table"), "{error}");
}

#[test]
fn a_condition_that_names_something_the_world_does_not_have_stops_the_run() {
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let catalogue = world.catalogue();
    let message = |rest: &str| {
        one_action("rest", &format!("args = {{ ticks = 5 }}\n{rest}"))
            .resolve(&catalogue)
            .expect_err("the condition does not fit")
            .message
    };
    let attr = message("when = { attr_below = { \"need.fod\" = 500 } }");
    assert!(attr.contains("the verb `rest`: the world has no attribute `need.fod`"), "{attr}");
    let tag = message("when = { carries_tag = \"fod\" }");
    assert!(tag.contains("the verb `rest`: no item of the world has the tag `fod`"), "{tag}");
    let did = message("when = { did = \"open_account\" }");
    assert!(did.contains("the profile has no action `open_account`"), "{did}");
    // The same conditions with names of the game are accepted.
    let fine = one_action(
        "rest",
        "args = { ticks = 5 }\nwhen = { attr_below = { \"need.food\" = 500 }, carries_tag = \"food\" }",
    );
    assert!(fine.resolve(&catalogue).is_ok());
    // A key of `when` that the tool does not know is refused when the
    // file is read.
    let text = "name = \"p\"\n[think]\nkind = \"constant\"\nticks = 1\n[[action]]\nverb = \"rest\"\nweight = 1\nwhen = { cash_minimum = 5 }\n";
    let error = Profile::parse(text).expect_err("`cash_minimum` is not a condition");
    assert!(error.message.contains("cash_minimum"), "{error}");
}

/// A catalogue for the tests of places and conditions: `move_to` with
/// `site`, a few verbs without arguments, the food `bread` and the
/// attribute `need.food`.
fn game_catalogue() -> Catalogue {
    let verb = |name: &str, schema: serde_json::Value| VerbInfo {
        verb: name.to_string(),
        module: "test".into(),
        summary: String::new(),
        free: false,
        scope: VerbScope::Zone,
        args_schema: schema,
    };
    let none = json!({ "type": "object" });
    Catalogue {
        verbs: vec![
            verb(
                "move_to",
                json!({ "type": "object", "properties": { "site": {}, "x": {}, "y": {} } }),
            ),
            verb("work", none.clone()),
            verb("eat", none.clone()),
            verb("open_account", none.clone()),
            verb("deposit", none.clone()),
            verb("idle", none),
        ],
        items: vec![ItemInfo {
            id: ItemId(6),
            name: "bread".into(),
            weight: 1,
            tags: vec!["food".into()],
            attrs: BTreeMap::new(),
            summary: String::new(),
        }],
        attrs: vec![AttrInfo {
            name: "need.food".into(),
            min: Some(0),
            max: Some(1_000),
            default: 1_000,
            summary: String::new(),
        }],
        ..Catalogue::default()
    }
}

const PLACES: &str = r#"
name = "places"
[think]
kind = "constant"
ticks = 1

[[action]]
verb = "work"
weight = 1
at = ["depot", "city_hall"]
"#;

#[test]
fn an_action_at_a_site_makes_the_resident_walk_there_first() {
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let map = Arc::new(WorldMap::of_world(&world).expect("the map of the world"));
    let city = map.city(CityId(0)).expect("the city");
    let site = |kind: &str| {
        city.sites().iter().find(|site| site.kind == kind).expect("each city has the kind").clone()
    };
    let (depot, hall, farm) = (site("depot"), site("city_hall"), site("farm"));
    let resolved = Profile::parse(PLACES)
        .expect("the profile parses")
        .resolve(&game_catalogue())
        .expect("the verbs exist");
    let mut rng = Rng::seeded(3);

    // On a farm: the action is a plan of the walk to the depot or to the
    // city hall, and the work.
    let mut scene = ListScene::new(ResidentId(1), farm.rect.centre(), None);
    scene.map = Some(Arc::clone(&map));
    scene.city = Some(CityId(0));
    scene.site_kind = Some("farm".into());
    let mut walked_to: BTreeMap<SiteId, u32> = BTreeMap::new();
    for _ in 0..200 {
        let choice = resolved.pick(&mut rng, &mut scene, Memory::default()).expect("a choice");
        assert_eq!(choice.verb, "work");
        let target = choice.walk_to.expect("the resident is not at a site of the kinds");
        let walk = choice.walk().expect("a plan has the walk");
        assert_eq!((walk.verb.as_str(), &walk.args), ("move_to", &json!({ "site": target })));
        *walked_to.entry(target).or_default() += 1;
    }
    // One of the nearest sites of the kinds, not always the same one.
    assert_eq!(walked_to.keys().copied().collect::<Vec<_>>(), {
        let mut both = vec![depot.id, hall.id];
        both.sort();
        both
    });
    assert!(walked_to.values().all(|&count| count > 50), "{walked_to:?}");

    // In the depot: no walk.
    scene.pos = depot.rect.centre();
    scene.site_kind = Some("depot".into());
    let choice = resolved.pick(&mut rng, &mut scene, Memory::default()).expect("a choice");
    assert_eq!(choice.walk_to, None);
    assert_eq!(choice.walk(), None);

    // A scene without a map cannot find the site: the action is not
    // possible, and nothing else is in the profile.
    let mut blind = ListScene::new(ResidentId(1), Pos::new(0, 0), None);
    assert_eq!(resolved.pick(&mut rng, &mut blind, Memory::default()), None);

    // A kind that no city has gives a warning when the run starts.
    let mut odd = Profile::parse(&PLACES.replace("\"depot\", \"city_hall\"", "\"lighthouse\""))
        .expect("the profile parses")
        .resolve(&game_catalogue())
        .expect("the verbs exist");
    assert!(odd.warnings.is_empty());
    odd.check_map(&map);
    assert_eq!(
        odd.warnings,
        ["profile `places`: no city of the world has a site of the kind `lighthouse`"]
    );
    assert_eq!(odd.pick(&mut rng, &mut scene, Memory::default()), None);
}

#[test]
fn an_action_with_a_place_needs_the_walk_verb_of_the_world() {
    // The world of this catalogue has no `move_to`.
    let mut catalogue = game_catalogue();
    catalogue.verbs.retain(|verb| verb.verb != "move_to");
    let error = Profile::parse(PLACES)
        .expect("the profile parses")
        .resolve(&catalogue)
        .expect_err("nothing walks to a site");
    assert!(
        error.message.contains("the world has no verb `move_to` with the argument `site`"),
        "{error}"
    );
}

const CONDITIONS: &str = r#"
name = "conditions"
[think]
kind = "constant"
ticks = 1

[[action]]
verb = "eat"
weight = 1
when = { carries_tag = "food", attr_below = { "need.food" = 900 } }

[[action]]
verb = "open_account"
weight = 1
once = true
when = { cash_min = 100 }

[[action]]
verb = "deposit"
weight = 1
when = { did = "open_account", cash_min = 50, cash_max = 5000 }
"#;

/// Returns the verbs that a resident can choose in a scene, from many
/// tries.
fn possible(
    resolved: &unsup_loadtest::profile::Resolved,
    scene: &mut ListScene,
    memory: Memory,
) -> Vec<String> {
    let mut rng = Rng::seeded(5);
    let mut verbs = std::collections::BTreeSet::new();
    for _ in 0..200 {
        if let Some(choice) = resolved.pick(&mut rng, scene, memory) {
            verbs.insert(choice.verb);
        }
    }
    verbs.into_iter().collect()
}

#[test]
fn conditions_decide_which_actions_a_resident_can_choose() {
    let resolved = Profile::parse(CONDITIONS)
        .expect("the profile parses")
        .resolve(&game_catalogue())
        .expect("the verbs exist");
    let none = Memory::default();
    let mut scene = scene();

    // A newcomer: no cash, no goods. Nothing is possible, and a scene that
    // does not know the cash or the attribute gives nothing either.
    assert!(possible(&resolved, &mut scene, none).is_empty());
    scene.cash = Some(0);
    scene.attrs.insert("need.food".into(), 1_000);
    assert!(possible(&resolved, &mut scene, none).is_empty());

    // Bread and hunger: the resident can eat. Exactly at the limit it
    // cannot: `attr_below` is strict.
    scene.carried = vec![Carried { item: ItemId(6), qty: 1 }];
    assert!(possible(&resolved, &mut scene, none).is_empty(), "the resident is full");
    scene.attrs.insert("need.food".into(), 900);
    assert!(possible(&resolved, &mut scene, none).is_empty(), "900 is not below 900");
    scene.attrs.insert("need.food".into(), 899);
    assert_eq!(possible(&resolved, &mut scene, none), ["eat"]);

    // With cash the resident can open an account, and cannot deposit yet.
    scene.carried.clear();
    scene.cash = Some(99);
    assert!(possible(&resolved, &mut scene, none).is_empty());
    scene.cash = Some(100);
    assert_eq!(possible(&resolved, &mut scene, none), ["open_account"]);

    // After the world accepted `open_account`, the resident remembers it:
    // it does not open an account again, and it can deposit.
    let mut rng = Rng::seeded(6);
    let choice = resolved.pick(&mut rng, &mut scene, none).expect("a choice");
    assert_eq!(choice.verb, "open_account");
    let mut memory = none;
    choice.remember(&mut memory);
    assert_ne!(memory, none);
    assert_eq!(possible(&resolved, &mut scene, memory), ["deposit"]);
    // A choice that leaves no mark changes nothing.
    let deposit = resolved.pick(&mut rng, &mut scene, memory).expect("a choice");
    let before = memory;
    deposit.remember(&mut memory);
    assert_eq!(memory, before);

    // `cash_max` is the upper limit, both ends included.
    scene.cash = Some(5_000);
    assert_eq!(possible(&resolved, &mut scene, memory), ["deposit"]);
    scene.cash = Some(5_001);
    assert!(possible(&resolved, &mut scene, memory).is_empty());
}

#[test]
fn the_weights_hold_among_the_actions_that_are_possible() {
    // `idle` has weight 3 and `eat` weight 1, and `eat` needs food.
    let text = r#"
name = "mix"
[think]
kind = "constant"
ticks = 1
[[action]]
verb = "idle"
weight = 3
[[action]]
verb = "eat"
weight = 1
when = { carries_tag = "food" }
"#;
    let resolved = Profile::parse(text)
        .expect("the profile parses")
        .resolve(&game_catalogue())
        .expect("the verbs exist");
    let mut rng = Rng::seeded(8);
    let mut scene = scene();
    let count = |scene: &mut ListScene, rng: &mut Rng| {
        let mut eats = 0u32;
        for _ in 0..20_000 {
            let choice = resolved.pick(rng, scene, Memory::default()).expect("`idle` is possible");
            eats += u32::from(choice.verb == "eat");
        }
        eats
    };
    // Without food the resident never eats, and never has an empty turn.
    assert_eq!(count(&mut scene, &mut rng), 0);
    // With food it eats one time in four: 5,000 of 20,000, give or take.
    scene.carried = vec![Carried { item: ItemId(6), qty: 1 }];
    let eats = count(&mut scene, &mut rng);
    assert!(eats.abs_diff(5_000) < 400, "`eat` was chosen {eats} times of 20000");
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
fn the_built_in_profiles_fit_the_world_of_the_server() {
    // Every verb of every built-in profile exists in the game, and every
    // argument name and every condition fits. `resolve` fails for a name
    // that does not fit, so a wrong profile fails here.
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let catalogue = world.catalogue();
    let map = WorldMap::of_world(&world).expect("the map of the world");
    assert_eq!(Profile::builtin_names(), ["walkers", "rush", "daily_life", "still"]);
    for name in Profile::builtin_names() {
        let profile = Profile::load(name).unwrap_or_else(|e| panic!("profile `{name}`: {e}"));
        assert_eq!(profile.name, name);
        let mut resolved =
            profile.resolve(&catalogue).unwrap_or_else(|e| panic!("profile `{name}`: {e}"));
        assert!(
            resolved.skipped.is_empty(),
            "`{name}` names unknown verbs: {:?}",
            resolved.skipped
        );
        // Every site kind of an `at` exists in a city.
        resolved.check_map(&map);
        assert!(resolved.warnings.is_empty(), "profile `{name}`: {:?}", resolved.warnings);
        // No action was dropped: the verbs with a weight are those of the
        // file.
        let in_file: Vec<(&str, u32)> = profile
            .actions
            .iter()
            .filter(|action| action.weight > 0)
            .map(|action| (action.verb.as_str(), action.weight))
            .collect();
        assert_eq!(resolved.verbs(), in_file, "profile `{name}`");
    }
}

#[test]
fn a_built_in_profile_with_a_wrong_argument_name_fails_against_the_game() {
    // The check above must be able to fail. Take the real `daily_life`
    // file and change one argument name: `amount` of `deposit`.
    let world =
        build_world(Shape { cities: 1, districts: 1 }.to_config(1), &[]).expect("a valid world");
    let catalogue = world.catalogue();
    let text = include_str!("../profiles/daily_life.toml");
    let needle = "args = { amount = { gen = \"cash\", percent = 50 } }";
    assert_eq!(text.matches(needle).count(), 1, "the profile has the `deposit` line");
    let broken = text.replace(needle, "args = { cents = { gen = \"cash\", percent = 50 } }");
    let error = Profile::parse(&broken)
        .expect("the file still parses")
        .resolve(&catalogue)
        .expect_err("`deposit` has no argument `cents`");
    assert_eq!(
        error.message,
        "profile `daily_life` does not fit the verbs of the world: the verb `deposit` has no \
         argument `cents`; the verb `deposit` needs the argument `amount`, which the profile does \
         not give"
    );
    // A verb that the game does not have is named too.
    let renamed = text.replace("verb = \"work\"", "verb = \"labour\"");
    let resolved = Profile::parse(&renamed)
        .expect("the file still parses")
        .resolve(&catalogue)
        .expect("the other verbs exist");
    assert_eq!(resolved.skipped, ["labour"]);
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
    assert!(error.message.contains("walkers, rush, daily_life, still"), "{error}");
}
