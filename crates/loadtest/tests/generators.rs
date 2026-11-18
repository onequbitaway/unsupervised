//! The argument generators of behaviour profiles.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use unsup_kernel::Rng;
use unsup_loadtest::local::{LocalScene, Placement, Shape, WorldSpec, populate};
use unsup_loadtest::profile::{ArgSpec, Profile};
use unsup_loadtest::scene::{ListScene, NearSite, Neighbour, Scene};
use unsup_protocol::{
    ActionRequest, ActionStatus, ItemId, ItemInfo, ObserveOpts, Pos, Rect, ResidentId, SiteId,
};

fn spec(toml_args: &str) -> ArgSpec {
    #[derive(serde::Deserialize)]
    struct Holder {
        args: ArgSpec,
    }
    toml::from_str::<Holder>(&format!("args = {toml_args}")).expect("the arguments parse").args
}

fn item(id: u16, name: &str, tags: &[&str]) -> ItemInfo {
    ItemInfo {
        id: ItemId(id),
        name: name.into(),
        weight: 1,
        tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
        attrs: Default::default(),
        summary: String::new(),
    }
}

fn xy(value: &Value) -> (i64, i64) {
    (value["x"].as_i64().expect("x is a number"), value["y"].as_i64().expect("y is a number"))
}

/// Makes `count` values and returns them.
fn draw(spec: &ArgSpec, scene: &mut ListScene, items: &[ItemInfo], count: usize) -> Vec<Value> {
    let mut rng = Rng::seeded(11);
    (0..count).map(|_| spec.generate(&mut rng, scene, items).expect("a value")).collect()
}

#[test]
fn a_near_tile_is_within_the_radius_on_both_axes() {
    let mut scene = ListScene::new(ResidentId(1), Pos::new(100, -40), None);
    let values = draw(&spec(r#"{ gen = "near_tile", radius = 5 }"#), &mut scene, &[], 4_000);
    let xs: BTreeSet<i64> = values.iter().map(|v| xy(v).0).collect();
    let ys: BTreeSet<i64> = values.iter().map(|v| xy(v).1).collect();
    // Every tile of the square is possible, and nothing outside it.
    assert_eq!(xs, (95..=105).collect::<BTreeSet<i64>>());
    assert_eq!(ys, (-45..=-35).collect::<BTreeSet<i64>>());
    // The object has exactly the two coordinates.
    assert_eq!(values[0].as_object().expect("an object").len(), 2);
}

#[test]
fn a_near_tile_stays_inside_the_city() {
    // The resident stands in the north-west corner of a city of 64 tiles.
    let city = Rect::new(0, 0, 64, 64);
    let mut scene = ListScene::new(ResidentId(1), Pos::new(0, 0), Some(city));
    let values = draw(&spec(r#"{ gen = "near_tile", radius = 12 }"#), &mut scene, &[], 4_000);
    let xs: BTreeSet<i64> = values.iter().map(|v| xy(v).0).collect();
    let ys: BTreeSet<i64> = values.iter().map(|v| xy(v).1).collect();
    assert_eq!(xs, (0..=12).collect::<BTreeSet<i64>>());
    assert_eq!(ys, (0..=12).collect::<BTreeSet<i64>>());
    // The edge is not chosen more often than its share: the range is cut
    // before the draw, not after. 1 of 13 is 308 of 4,000.
    let on_edge = values.iter().filter(|v| xy(v).0 == 0).count();
    assert!((200..420).contains(&on_edge), "{on_edge} of 4000 draws are on the west edge");

    // The south-east corner: the last tile of the city is 63.
    scene.pos = Pos::new(63, 63);
    let values = draw(&spec(r#"{ gen = "near_tile", radius = 3 }"#), &mut scene, &[], 500);
    assert!(values.iter().all(|v| (60..=63).contains(&xy(v).0) && (60..=63).contains(&xy(v).1)));
    assert!(values.iter().any(|v| xy(v).0 == 63), "the last column is reachable");

    // A city that is smaller than the radius.
    scene.bounds = Some(Rect::new(10, 10, 13, 13));
    scene.pos = Pos::new(11, 11);
    let values = draw(&spec(r#"{ gen = "near_tile", radius = 50 }"#), &mut scene, &[], 500);
    let xs: BTreeSet<i64> = values.iter().map(|v| xy(v).0).collect();
    assert_eq!(xs, BTreeSet::from([10, 11, 12]));
}

#[test]
fn near_x_and_near_y_give_one_coordinate_each() {
    let city = Rect::new(-8, 0, 8, 100);
    let mut scene = ListScene::new(ResidentId(1), Pos::new(6, 50), Some(city));
    let both = spec(
        r#"{ x = { gen = "near_x", radius = 4 }, y = { gen = "near_y", radius = 2 }, speed = 3 }"#,
    );
    let values = draw(&both, &mut scene, &[], 2_000);
    let xs: BTreeSet<i64> = values.iter().map(|v| xy(v).0).collect();
    let ys: BTreeSet<i64> = values.iter().map(|v| xy(v).1).collect();
    // x: 2..=10 cut at the last column of the city, 7.
    assert_eq!(xs, (2..=7).collect::<BTreeSet<i64>>());
    assert_eq!(ys, (48..=52).collect::<BTreeSet<i64>>());
    assert!(values.iter().all(|v| v["speed"] == json!(3)), "fixed values stay as written");
}

#[test]
fn a_nearby_resident_is_another_resident_within_the_radius() {
    let mut scene = ListScene::new(ResidentId(5), Pos::new(10, 10), None);
    scene.residents = vec![
        Neighbour { id: ResidentId(5), pos: Pos::new(10, 10) }, // the resident itself
        Neighbour { id: ResidentId(6), pos: Pos::new(12, 11) }, // distance 2
        Neighbour { id: ResidentId(7), pos: Pos::new(13, 13) }, // distance 3: a diagonal step counts as one
        Neighbour { id: ResidentId(8), pos: Pos::new(14, 10) }, // distance 4
    ];
    let generator = spec(r#"{ to = { gen = "nearby_resident", radius = 3 } }"#);
    let values = draw(&generator, &mut scene, &[], 500);
    let chosen: BTreeSet<u64> = values.iter().map(|v| v["to"].as_u64().expect("an id")).collect();
    assert_eq!(chosen, BTreeSet::from([6, 7]), "only residents 6 and 7 are within 3 tiles");

    // With nobody near, the action is not possible.
    let mut rng = Rng::seeded(1);
    let alone = spec(r#"{ to = { gen = "nearby_resident", radius = 1 } }"#);
    assert_eq!(alone.generate(&mut rng, &mut scene, &[]), None);
}

#[test]
fn simple_generators_give_numbers_choices_text_and_the_own_id() {
    let mut scene = ListScene::new(ResidentId(77), Pos::new(0, 0), None);

    let ints = draw(&spec(r#"{ gen = "int", min = -2, max = 3 }"#), &mut scene, &[], 2_000);
    let seen: BTreeSet<i64> = ints.iter().map(|v| v.as_i64().expect("a number")).collect();
    assert_eq!(seen, (-2..=3).collect::<BTreeSet<i64>>(), "both ends are included");

    let choices = draw(&spec(r#"{ gen = "choice", of = ["a", 2, true] }"#), &mut scene, &[], 500);
    let seen: BTreeSet<String> = choices.iter().map(Value::to_string).collect();
    assert_eq!(seen, BTreeSet::from(["\"a\"".to_string(), "2".to_string(), "true".to_string()]));

    let texts = draw(&spec(r#"{ gen = "text", words = 4 }"#), &mut scene, &[], 50);
    for text in &texts {
        let text = text.as_str().expect("text");
        assert_eq!(text.split(' ').count(), 4, "`{text}` has four words");
        assert!(text.split(' ').all(|word| !word.is_empty() && word.is_ascii()));
    }
    let different: BTreeSet<&str> = texts.iter().filter_map(Value::as_str).collect();
    assert!(different.len() > 10, "texts differ");

    let me = draw(&spec(r#"{ gen = "self" }"#), &mut scene, &[], 1);
    assert_eq!(me, [json!(77)]);
}

#[test]
fn an_item_comes_from_the_catalogue_by_tag() {
    let items =
        [item(1, "bread", &["food"]), item(2, "coal", &["fuel"]), item(3, "apple", &["food"])];
    let mut scene = ListScene::new(ResidentId(1), Pos::new(0, 0), None);

    let food = draw(&spec(r#"{ gen = "item", tag = "food" }"#), &mut scene, &items, 300);
    let names: BTreeSet<&str> = food.iter().map(|v| v.as_str().expect("a name")).collect();
    assert_eq!(names, BTreeSet::from(["apple", "bread"]));

    let any = draw(&spec(r#"{ gen = "item", as = "id" }"#), &mut scene, &items, 300);
    let ids: BTreeSet<u64> = any.iter().map(|v| v.as_u64().expect("an id")).collect();
    assert_eq!(ids, BTreeSet::from([1, 2, 3]));

    let mut rng = Rng::seeded(1);
    let none = spec(r#"{ gen = "item", tag = "medicine" }"#);
    assert_eq!(none.generate(&mut rng, &mut scene, &items), None, "no item has the tag");
    assert_eq!(spec(r#"{ gen = "item" }"#).generate(&mut rng, &mut scene, &[]), None);
}

#[test]
fn a_site_is_chosen_by_distance_and_kind() {
    let mut scene = ListScene::new(ResidentId(1), Pos::new(0, 0), None);
    scene.sites = vec![
        NearSite { id: SiteId(10), kind: "shop".into(), pos: Pos::new(3, 0) },
        NearSite { id: SiteId(11), kind: "bank".into(), pos: Pos::new(0, 4) },
        NearSite { id: SiteId(12), kind: "shop".into(), pos: Pos::new(9, 9) },
    ];
    let near = draw(&spec(r#"{ gen = "nearby_site", radius = 5 }"#), &mut scene, &[], 300);
    let ids: BTreeSet<u64> = near.iter().map(|v| v.as_u64().expect("an id")).collect();
    assert_eq!(ids, BTreeSet::from([10, 11]), "site 12 is 9 tiles away");

    let shops =
        draw(&spec(r#"{ gen = "nearby_site", radius = 20, kind = "shop" }"#), &mut scene, &[], 300);
    let ids: BTreeSet<u64> = shops.iter().map(|v| v.as_u64().expect("an id")).collect();
    assert_eq!(ids, BTreeSet::from([10, 12]));

    let tiles =
        draw(&spec(r#"{ gen = "site_tile", radius = 5, kind = "bank" }"#), &mut scene, &[], 20);
    assert!(tiles.iter().all(|v| xy(v) == (0, 4)), "the tile of the bank");

    let mut rng = Rng::seeded(1);
    let none = spec(r#"{ gen = "site_tile", radius = 5, kind = "farm" }"#);
    assert_eq!(none.generate(&mut rng, &mut scene, &[]), None);
}

#[test]
fn generators_work_inside_tables_and_lists_and_fixed_values_stay() {
    let mut scene = ListScene::new(ResidentId(9), Pos::new(0, 0), None);
    let nested = spec(
        r#"{ routines = [ { name = "idle", when = { type = "always" }, then = { verb = "wait", args = { ticks = { gen = "int", min = 4, max = 4 } } } } ], who = [ { gen = "self" }, 1 ] }"#,
    );
    let values = draw(&nested, &mut scene, &[], 1);
    assert_eq!(
        values[0],
        json!({
            "routines": [{
                "name": "idle",
                "when": { "type": "always" },
                "then": { "verb": "wait", "args": { "ticks": 4 } },
            }],
            "who": [9, 1],
        })
    );
    // An action without arguments carries none.
    let mut rng = Rng::seeded(1);
    assert_eq!(ArgSpec::Absent.generate(&mut rng, &mut scene, &[]), Some(Value::Null));
    assert_eq!(ArgSpec::Absent.keys(), Some(vec![]));
    assert_eq!(spec(r#"{ a = 1, b = { gen = "self" } }"#).keys(), Some(vec!["a", "b"]));
    assert_eq!(spec(r#"{ gen = "near_tile", radius = 1 }"#).keys(), None);
}

#[test]
fn the_scene_of_a_world_in_this_process_matches_the_observation() {
    // Four residents that stay where the game put them: all on one tile.
    let spec = WorldSpec {
        seed: 3,
        residents: 4,
        shape: Shape { cities: 1, districts: 2 },
        placement: Placement::Spawn,
        data: Vec::new(),
    };
    let population = populate(&spec, &[]).expect("the world is built");
    let world = population.world;
    let me = population.ids[1];
    let mut scratch = Default::default();
    let mut scene = LocalScene::of(&world, me, &mut scratch).expect("the resident is in the world");

    let observation = world.observe(me, &ObserveOpts::default()).expect("an observation");
    let pos = &observation.sections["self"]["pos"];
    assert_eq!(scene.me(), me);
    assert_eq!((i64::from(scene.pos().x), i64::from(scene.pos().y)), xy(pos));
    assert_eq!(scene.bounds(), Some(world.layout().cities()[0].bounds));

    // The three others, in ascending id order, never the resident itself.
    let others: Vec<ResidentId> = scene.residents(0).iter().map(|n| n.id).collect();
    let expected: Vec<ResidentId> = population.ids.iter().copied().filter(|&id| id != me).collect();
    assert_eq!(others, expected);
    let seen: Vec<u64> = observation.sections["nearby"]["residents"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|r| r["id"].as_u64().expect("an id"))
        .collect();
    assert_eq!(others.iter().map(|id| id.0).collect::<Vec<u64>>(), seen);

    let mut none = Default::default();
    assert!(LocalScene::of(&world, ResidentId(999_999), &mut none).is_none());
}

#[test]
fn the_world_accepts_every_generated_walk_also_at_the_edge_of_a_city() {
    // One zone of 64 tiles and a radius of 40: without the cut at the city
    // bounds, most targets would be outside the world.
    let spec = WorldSpec {
        seed: 5,
        residents: 200,
        shape: Shape { cities: 1, districts: 1 },
        placement: Placement::Scatter,
        data: Vec::new(),
    };
    let population = populate(&spec, &[]).expect("the world is built");
    let mut world = population.world;
    let text = r#"
name = "far"
[think]
kind = "constant"
ticks = 0
[[action]]
verb = "move_to"
weight = 1
args = { gen = "near_tile", radius = 40 }
"#;
    let profile = Profile::parse(text).expect("the profile parses");
    let resolved = profile.resolve(&world.catalogue()).expect("move_to exists");
    let mut rng = Rng::seeded(9);
    let mut scratch = Default::default();
    let mut longest = 0;
    for &id in &population.ids {
        let choice = {
            let mut scene = LocalScene::of(&world, id, &mut scratch).expect("the resident exists");
            resolved.pick(&mut rng, &mut scene).expect("a walk is always possible")
        };
        let ack = world.act(id, ActionRequest::new(choice.verb, choice.args)).expect("an answer");
        match ack.status {
            ActionStatus::Started { ticks, .. } => longest = longest.max(ticks),
            other => panic!("the world did not accept the walk of resident {id}: {other:?}"),
        }
    }
    assert!(longest <= 40, "no walk is longer than the radius, the longest is {longest}");
    assert!(longest > 20, "some walks are long, the longest is {longest}");
}
