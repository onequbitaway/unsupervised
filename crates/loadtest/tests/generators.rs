//! The argument generators of behaviour profiles.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use unsup_kernel::Rng;
use unsup_loadtest::local::{LocalScene, Placement, Shape, WorldSpec, populate};
use unsup_loadtest::map::WorldMap;
use unsup_loadtest::profile::{ArgSpec, Memory, Profile};
use unsup_loadtest::scene::{Carried, ListScene, NearSite, Neighbour, Scene};
use unsup_protocol::{
    ActionRequest, ActionStatus, CityId, CityView, Clock, ItemId, ItemInfo, ObserveOpts, Pos, Rect,
    ResidentId, SiteId, SiteView, WorldView, ZoneId, ZoneView,
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

    let mut rng = Rng::seeded(1);
    let none = spec(r#"{ gen = "nearby_site", radius = 5, kind = "farm" }"#);
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
    // A generator that makes the whole table has the names of its table.
    assert_eq!(spec(r#"{ gen = "near_tile", radius = 1 }"#).keys(), Some(vec!["x", "y"]));
    assert_eq!(spec(r#"{ gen = "carried" }"#).keys(), Some(vec!["item", "qty"]));
    // One value is not a table of arguments.
    assert_eq!(spec(r#"{ gen = "self" }"#).keys(), None);
    assert_eq!(spec("3").keys(), None);
}

/// A city of one zone of 8 by 8 tiles. `rows` gives the tile kinds, one
/// digit for each tile. The city has a depot in the north-west and two
/// farms.
fn small_map(rows: [&str; 8]) -> WorldMap {
    let bounds = Rect::new(100, 200, 108, 208);
    let tiles: Vec<u8> =
        rows.iter().flat_map(|row| row.bytes().map(|digit| digit - b'0')).collect();
    let site = |id: u64, kind: &str, rect: Rect| SiteView {
        id: SiteId(id),
        kind: kind.into(),
        name: format!("{kind} {id}"),
        rect,
        tags: Vec::new(),
    };
    let zone = ZoneView { id: ZoneId(4), city: CityId(2), name: "Z".into(), bounds, residents: 0 };
    let layout = WorldView::Layout {
        frame: 0,
        clock: Clock::default(),
        cities: vec![CityView { id: CityId(2), name: "C".into(), bounds, zones: vec![ZoneId(4)] }],
        zones: vec![zone.clone()],
    };
    let view = WorldView::Zone {
        zone,
        width: 8,
        height: 8,
        tiles,
        sites: vec![
            site(1, "depot", Rect::new(100, 200, 103, 203)),
            site(2, "farm", Rect::new(104, 204, 106, 206)),
            site(3, "farm", Rect::new(106, 200, 108, 202)),
        ],
        residents: Vec::new(),
    };
    WorldMap::from_views(&layout, vec![view]).expect("the views make a map")
}

/// Grass with a wall (4) around a floor (3), a pond (5) and a rock (8).
const TILES: [&str; 8] = [
    "44400000", "43400000", "44400000", "00000550", "00000000", "01111110", "00080000", "00000000",
];

fn scene_on(map: WorldMap, pos: Pos) -> ListScene {
    let mut scene = ListScene::new(ResidentId(1), pos, None);
    scene.map = Some(Arc::new(map));
    scene.city = Some(CityId(2));
    scene
}

#[test]
fn a_near_tile_is_a_tile_on_which_a_resident_can_stand() {
    let map = small_map(TILES);
    let city = map.city(CityId(2)).expect("the city is on the map");
    // The map knows the kinds: wall, water and rock are not walkable.
    assert_eq!(city.tile(Pos::new(100, 200)), Some(4));
    assert!(!city.walkable(Pos::new(100, 200)), "a wall");
    assert!(city.walkable(Pos::new(101, 201)), "the floor inside the wall");
    assert!(!city.walkable(Pos::new(105, 203)), "water");
    assert!(!city.walkable(Pos::new(103, 206)), "rock");
    assert!(city.walkable(Pos::new(101, 205)), "a road");
    assert!(!city.walkable(Pos::new(99, 200)), "outside the city");
    assert_eq!(city.tile(Pos::new(108, 208)), None);

    let mut scene = scene_on(map.clone(), Pos::new(104, 204));
    let near = spec(r#"{ gen = "near_tile", radius = 6 }"#);
    let values = draw(&near, &mut scene, &[], 4_000);
    let chosen: BTreeSet<(i64, i64)> = values.iter().map(xy).collect();
    let walkable: BTreeSet<(i64, i64)> = (200..208)
        .flat_map(|y| (100..108).map(move |x| (x, y)))
        .filter(|&(x, y)| city.walkable(Pos::new(x as i32, y as i32)))
        .collect();
    // 64 tiles: 8 of wall, 2 of water and 1 of rock are left out.
    assert_eq!(walkable.len(), 53);
    assert_eq!(chosen, walkable, "every walkable tile of the city, and no other tile");

    // A resident that only walls and water surround has no target: the
    // action is not possible, and the tool does not send a bad walk.
    let closed = small_map([
        "44444444", "44444444", "44444444", "44455444", "44455444", "44444444", "44444444",
        "44444444",
    ]);
    let mut scene = scene_on(closed, Pos::new(103, 203));
    let mut rng = Rng::seeded(2);
    assert_eq!(near.generate(&mut rng, &mut scene, &[]), None);
}

#[test]
fn the_nearest_sites_of_a_kind_come_first() {
    let map = small_map(TILES);
    let city = map.city(CityId(2)).expect("the city is on the map");
    let kinds = ["farm".to_string()];
    // From the south-west corner, farm 2 is 3 tiles away and farm 3 is 6.
    let near: Vec<u64> =
        city.nearest(&kinds, Pos::new(100, 207), 3).iter().map(|site| site.id.0).collect();
    assert_eq!(near, [2, 3]);
    let one: Vec<u64> =
        city.nearest(&kinds, Pos::new(107, 200), 1).iter().map(|site| site.id.0).collect();
    assert_eq!(one, [3], "only the nearest one is asked for");
    let both = ["farm".to_string(), "depot".to_string()];
    let all: Vec<u64> =
        city.nearest(&both, Pos::new(100, 200), 9).iter().map(|site| site.id.0).collect();
    assert_eq!(all, [1, 2, 3], "the depot covers the tile; the farms follow by distance");
    assert!(city.nearest(&["bank".to_string()], Pos::new(100, 200), 3).is_empty());
    assert_eq!(city.site_at(Pos::new(105, 205)).map(|site| site.id), Some(SiteId(2)));
    assert_eq!(city.site_at(Pos::new(103, 203)), None);
    assert!(map.has_kind("depot") && !map.has_kind("bank"));
    assert!(map.city(CityId(0)).is_none());
}

#[test]
fn a_map_needs_the_view_of_each_zone_with_the_right_number_of_tiles() {
    let bounds = Rect::new(0, 0, 4, 4);
    let zone = ZoneView { id: ZoneId(0), city: CityId(0), name: "Z".into(), bounds, residents: 0 };
    let layout = WorldView::Layout {
        frame: 0,
        clock: Clock::default(),
        cities: vec![CityView { id: CityId(0), name: "C".into(), bounds, zones: vec![ZoneId(0)] }],
        zones: vec![zone.clone()],
    };
    let missing = WorldMap::from_views(&layout, Vec::new()).expect_err("the zone has no view");
    assert!(missing.message.contains("zone 0 of city 0 has no view"), "{missing}");

    let short = WorldView::Zone {
        zone,
        width: 4,
        height: 4,
        tiles: vec![0; 15],
        sites: Vec::new(),
        residents: Vec::new(),
    };
    let wrong = WorldMap::from_views(&layout, vec![short]).expect_err("15 tiles for 4 by 4");
    assert!(wrong.message.contains("zone 0 has 15 tiles for 4 columns and 4 rows"), "{wrong}");

    let not_layout = WorldMap::from_views(&layout.clone(), vec![layout.clone()])
        .expect_err("a layout is not the view of a zone");
    assert!(not_layout.message.contains("a view of a zone is the layout"), "{not_layout}");
}

#[test]
fn carried_goods_and_cash_come_from_what_the_resident_holds() {
    let items = [
        item(1, "vegetables", &["raw", "food"]),
        item(2, "grain", &["raw"]),
        item(6, "bread", &["food"]),
    ];
    let mut scene = ListScene::new(ResidentId(1), Pos::new(0, 0), None);
    scene.carried = vec![Carried { item: ItemId(2), qty: 7 }, Carried { item: ItemId(6), qty: 2 }];

    // Only grain is raw: the whole table names it with all its units.
    let raw = draw(&spec(r#"{ gen = "carried", tag = "raw" }"#), &mut scene, &items, 50);
    assert!(raw.iter().all(|v| *v == json!({ "item": "grain", "qty": 7 })), "{raw:?}");
    let some = draw(&spec(r#"{ gen = "carried", max_qty = 3 }"#), &mut scene, &items, 300);
    let seen: BTreeSet<String> = some.iter().map(Value::to_string).collect();
    assert_eq!(
        seen,
        BTreeSet::from([
            json!({ "item": "bread", "qty": 2 }).to_string(),
            json!({ "item": "grain", "qty": 3 }).to_string(),
        ])
    );
    let mut rng = Rng::seeded(1);
    let none = spec(r#"{ gen = "carried", tag = "medicine" }"#);
    assert_eq!(none.generate(&mut rng, &mut scene, &items), None, "nothing with the tag");

    // Half of 1,801 cents, rounded down.
    scene.cash = Some(1_801);
    let half = spec(r#"{ amount = { gen = "cash", percent = 50 } }"#);
    assert_eq!(half.generate(&mut rng, &mut scene, &items), Some(json!({ "amount": 900 })));
    // A share of less than one cent, and cash that is not known, give no
    // amount: the world would refuse an amount of zero.
    scene.cash = Some(1);
    assert_eq!(half.generate(&mut rng, &mut scene, &items), None);
    scene.cash = None;
    assert_eq!(half.generate(&mut rng, &mut scene, &items), None);
}

#[test]
fn the_scene_of_a_world_in_this_process_matches_the_observation() {
    // Residents that stay where the game put them: in the spawn sites of
    // the city.
    let spec = WorldSpec {
        seed: 3,
        residents: 12,
        shape: Shape { cities: 1, districts: 1 },
        placement: Placement::Spawn,
        data: Vec::new(),
    };
    let population = populate(&spec, &[]).expect("the world is built");
    let (world, map) = (population.world, population.map);
    let me = population.ids[1];
    let mut scratch = Default::default();
    let mut scene =
        LocalScene::of(&world, &map, me, &mut scratch).expect("the resident is in the world");

    let observation = world.observe(me, &ObserveOpts::default()).expect("an observation");
    let own = &observation.sections["self"];
    assert_eq!(scene.me(), me);
    assert_eq!((i64::from(scene.pos().x), i64::from(scene.pos().y)), xy(&own["pos"]));
    assert_eq!(scene.bounds(), Some(world.layout().cities()[0].bounds));
    // A new resident stands in a shelter or a station, with no cash and no
    // goods, and with full needs.
    let kind = own["site"]["kind"].as_str().expect("a new resident is in a site");
    assert!(["shelter", "station"].contains(&kind), "{kind}");
    assert_eq!(scene.site_kind(), Some(kind));
    assert_eq!(scene.cash(), Some(0));
    assert_eq!(own["cash"], json!(0));
    assert!(scene.carried().is_empty());
    assert_eq!(scene.attr("need.food"), Some(1_000));
    assert_eq!(scene.attr("need.food"), own["attrs"]["need.food"].as_i64());
    assert_eq!(scene.attr("no.such.attribute"), None);
    // The map of the scene is the map of the city: the resident stands on
    // a walkable tile.
    let city = scene.map().expect("the scene has the map of the city");
    assert!(city.walkable(scene.pos()));

    // The others in sight, in ascending id order, never the resident
    // itself. The observation lists the same residents, nearest first.
    let others: Vec<u64> = scene.residents(12).iter().map(|n| n.id.0).collect();
    assert!(!others.is_empty(), "the spawn sites hold several residents");
    assert!(!others.contains(&me.0));
    assert!(others.is_sorted());
    let mut seen: Vec<u64> = observation.sections["nearby"]["residents"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|r| r["id"].as_u64().expect("an id"))
        .collect();
    seen.sort_unstable();
    assert_eq!(others, seen);

    let mut none = Default::default();
    assert!(LocalScene::of(&world, &map, ResidentId(999_999), &mut none).is_none());
}

/// Sends a walk of each resident to a generated tile and returns the
/// longest walk in ticks. Panics when the world does not accept a walk.
fn walk_everybody(shape: Shape, residents: u32, radius: u32) -> (u32, usize) {
    let spec =
        WorldSpec { seed: 5, residents, shape, placement: Placement::Scatter, data: Vec::new() };
    let population = populate(&spec, &[]).expect("the world is built");
    let (mut world, map) = (population.world, population.map);
    let text = format!(
        "name = \"far\"\n[think]\nkind = \"constant\"\nticks = 0\n[[action]]\nverb = \"move_to\"\nweight = 1\nargs = {{ gen = \"near_tile\", radius = {radius} }}\n"
    );
    let profile = Profile::parse(&text).expect("the profile parses");
    let resolved = profile.resolve(&world.catalogue()).expect("move_to exists");
    let mut rng = Rng::seeded(9);
    let mut scratch = Default::default();
    let (mut longest, mut across) = (0, 0);
    // Every city of these worlds is city 0.
    let city = map.city(CityId(0)).expect("the map of the city");
    for &id in &population.ids {
        let (choice, target) = {
            let mut scene =
                LocalScene::of(&world, &map, id, &mut scratch).expect("the resident exists");
            // The scatter placement puts each resident on a walkable tile.
            let start = scene.pos();
            assert!(city.walkable(start), "resident {id} starts on {start}: not walkable");
            let choice = resolved
                .pick(&mut rng, &mut scene, Memory::default())
                .expect("a walk is always possible");
            let (x, y) = xy(&choice.args);
            let target = Pos::new(x as i32, y as i32);
            assert!(city.walkable(target), "the target {target} is not walkable");
            assert!(city.bounds().contains(target), "the target {target} is outside the city");
            (choice, target)
        };
        let ack = world.act(id, ActionRequest::new(choice.verb, choice.args)).expect("an answer");
        match ack.status {
            ActionStatus::Started { ticks, .. } => longest = longest.max(ticks),
            // The target is in another district: the walk waits for the
            // arrival.
            ActionStatus::Pending => across += 1,
            // The target is the tile of the resident.
            ActionStatus::Done { .. } => {}
            other => {
                panic!("the world did not accept the walk of resident {id} to {target}: {other:?}")
            }
        }
    }
    (longest, across)
}

#[test]
fn the_world_accepts_every_generated_walk_also_at_the_edge_of_a_city() {
    // One zone of 64 tiles and a radius of 40: without the cut at the city
    // bounds, most targets would be outside the world. Without the map,
    // one target in six would be a wall, water or rock.
    let (longest, across) = walk_everybody(Shape { cities: 1, districts: 1 }, 300, 40);
    assert_eq!(across, 0, "a city of one zone has no other district");
    // A walk goes around walls and costs more on grass and in trees.
    assert!(longest > 40, "some walks are long, the longest is {longest}");
}

#[test]
fn a_generated_walk_into_another_district_is_accepted_too() {
    // Four districts and a radius of 30: many targets are across a border.
    let (_, across) = walk_everybody(Shape { cities: 1, districts: 2 }, 300, 30);
    assert!(across > 20, "{across} of 300 walks cross a district border");
}

#[test]
fn every_resident_reaches_its_generated_target() {
    // The life module says that each walkable tile of a zone can be
    // reached. This test walks: every walk ends with a report that it
    // succeeded, at the target.
    let spec = WorldSpec {
        seed: 8,
        residents: 150,
        shape: Shape { cities: 1, districts: 2 },
        placement: Placement::Scatter,
        data: Vec::new(),
    };
    let population = populate(&spec, &[]).expect("the world is built");
    let (mut world, map) = (population.world, population.map);
    let near = spec_of_walk(20);
    let mut rng = Rng::seeded(4);
    let mut scratch = Default::default();
    let mut targets = Vec::new();
    for &id in &population.ids {
        let args = {
            let mut scene =
                LocalScene::of(&world, &map, id, &mut scratch).expect("the resident exists");
            near.generate(&mut rng, &mut scene, &[]).expect("a walkable tile is near")
        };
        let ack = world.act(id, ActionRequest::new("move_to", args.clone())).expect("an answer");
        assert!(ack.status.is_accepted(), "resident {id}: {:?}", ack.status);
        targets.push((id, xy(&args)));
    }
    // The longest way of 20 tiles through trees and around walls is far
    // below 400 ticks.
    for _ in 0..400 {
        world.step();
    }
    for (id, target) in targets {
        let observation = world.observe(id, &ObserveOpts::default()).expect("an observation");
        assert_eq!(xy(&observation.sections["self"]["pos"]), target, "resident {id}");
        let reports = observation.sections["reports"].as_array().expect("a list of reports");
        assert!(
            reports.iter().all(|report| report["ok"] == json!(true)),
            "resident {id}: {reports:?}"
        );
    }
}

fn spec_of_walk(radius: u32) -> ArgSpec {
    spec(&format!(r#"{{ gen = "near_tile", radius = {radius} }}"#))
}
