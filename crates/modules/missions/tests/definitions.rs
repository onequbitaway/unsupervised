//! The definition of a mission is data. Every field is checked, and a bad
//! definition is refused before any cash moves.

mod common;

use common::{bread_mission, escrow, ok, post, rich, world};
use serde_json::{Value, json};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ErrorCode, Money, ResidentId};
use unsup_missions::Missions;

/// Returns the refusal of a definition. The zone checks it, so the answer
/// comes at once and the poster keeps its cash.
fn refusal(w: &mut TestWorld<'_>, poster: ResidentId, definition: Value) -> String {
    let before = w.cash(poster);
    let error = w.act_err(poster, "mission_post", definition);
    assert_eq!(error.code, ErrorCode::InvalidArgs, "{}", error.message);
    assert_eq!(w.cash(poster), before);
    error.message
}

/// Returns the bread mission with one field changed.
fn with(field: &str, value: Value) -> Value {
    let mut definition = bread_mission(500, 2);
    definition[field] = value;
    definition
}

/// Returns the bread mission with one field of its objective changed.
fn with_objective(field: &str, value: Value) -> Value {
    let mut definition = bread_mission(500, 2);
    definition["objectives"][0][field] = value;
    definition
}

#[test]
fn each_field_of_a_definition_is_checked() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 1_000_000);
    let watched = "Use a kind that starts with one of: life., economy., finance., crime., \
                   politics., missions., society.org_, society.contract_, society.married, \
                   society.divorced, society.household_, society.opinion_.";
    let cases: Vec<(Value, String)> = vec![
        // The title.
        (with("title", json!("")), "The title is empty.".into()),
        (with("title", json!("  ")), "The title is empty.".into()),
        (with("title", json!("x".repeat(81))), "The title has more than 80 bytes.".into()),
        (with("title", json!("a\u{7}b")), "The title has a control character.".into()),
        (with("title", json!("two\nlines")), "The title has a line break.".into()),
        // The text.
        (with("text", json!("x".repeat(1_001))), "The text has more than 1000 bytes.".into()),
        (with("text", json!("a\u{0}b")), "The text has a control character.".into()),
        // The objectives.
        (with("objectives", json!([])), "A mission has from 1 to 4 objectives.".into()),
        (
            with("objectives", json!(vec![json!({ "event": "life.slept" }); 5])),
            "A mission has from 1 to 4 objectives.".into(),
        ),
        (
            with_objective("event", json!("kernel.spawned")),
            format!("`kernel.spawned` is not an event that a mission can count. {watched}"),
        ),
        (
            with_objective("event", json!("society.said")),
            format!("`society.said` is not an event that a mission can count. {watched}"),
        ),
        (
            with_objective("event", json!("economy.Goods Produced")),
            format!("`economy.Goods Produced` is not an event that a mission can count. {watched}"),
        ),
        (
            with_objective("event", json!(format!("economy.{}", "x".repeat(60)))),
            format!(
                "`economy.{}` is not an event that a mission can count. {watched}",
                "x".repeat(60)
            ),
        ),
        (with_objective("count", json!(0)), "`count` must be from 1 to 1000.".into()),
        (with_objective("count", json!(1_001)), "`count` must be from 1 to 1000.".into()),
        (
            with_objective("actor", json!("team")),
            "Only a team mission can count the events of team members.".into(),
        ),
        // The data fields of an objective.
        (
            with_objective("fields", json!({ "a": 1, "b": 2, "c": 3, "d": 4, "e": 5 })),
            "An objective compares at most 4 data fields.".into(),
        ),
        (
            with_objective("fields", json!({ "Item Name": "bread" })),
            "`Item Name` is not the name of a data field.".into(),
        ),
        (
            with_objective("fields", json!({ "qty": 1.5 })),
            "The value of `qty` must be a whole number.".into(),
        ),
        (
            with_objective("fields", json!({ "item": "fresh bread" })),
            "The value of `item` must be a whole number, a boolean or a short name without spaces."
                .into(),
        ),
        (
            with_objective("fields", json!({ "item": ["bread"] })),
            "The value of `item` must be a whole number, a boolean or a short name without spaces."
                .into(),
        ),
        (
            with_objective("fields", json!({ "item": "" })),
            "The value of `item` must be a whole number, a boolean or a short name without spaces."
                .into(),
        ),
        (
            with_objective("fields", json!({ "item": "x".repeat(65) })),
            "The value of `item` must be a whole number, a boolean or a short name without spaces."
                .into(),
        ),
        // Solo or team.
        (with("org", json!(77)), "Only a team mission can name an organization.".into()),
        // The reward, the takers and the deadline.
        (with("reward", json!(99)), "`reward` must be from 100 to 10000000 cents.".into()),
        (with("reward", json!(-5)), "`reward` must be from 100 to 10000000 cents.".into()),
        (with("reward", json!(10_000_001)), "`reward` must be from 100 to 10000000 cents.".into()),
        (with("takers", json!(0)), "`takers` must be from 1 to 50.".into()),
        (with("takers", json!(51)), "`takers` must be from 1 to 50.".into()),
        (
            with("deadline_in", json!(9)),
            "`deadline_in` must be from 10 to 40320 game minutes.".into(),
        ),
        (
            with("deadline_in", json!(40_321)),
            "`deadline_in` must be from 10 to 40320 game minutes.".into(),
        ),
    ];
    assert_eq!(cases.len(), 31);
    for (definition, message) in cases {
        assert_eq!(refusal(&mut w, ann, definition), message);
    }
    assert_eq!(w.global().res::<Missions>().open_count(), 0);
    assert_eq!(w.cash(ann), Money(1_000_000));
    w.assert_conserved();
}

#[test]
fn a_definition_that_does_not_parse_is_refused() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    let cases: Vec<(Value, &str)> = vec![
        // A field that the definition does not have. A mission carries no
        // code and no other data.
        (with("script", json!("pay(me)")), "unknown field `script`"),
        (with_objective("when", json!("always")), "unknown field `when`"),
        (with("mode", json!("duo")), "unknown variant `duo`"),
        (with_objective("actor", json!("anybody")), "unknown variant `anybody`"),
        (with("reward", json!("much")), "invalid type: string \"much\""),
        (with("takers", json!(-1)), "invalid value: integer `-1`"),
        (
            json!({ "title": "No objectives", "reward": 500, "deadline_in": 60 }),
            "missing field `objectives`",
        ),
        (
            json!({ "objectives": [{ "event": "life.slept" }], "reward": 500, "deadline_in": 60 }),
            "missing field `title`",
        ),
        (
            json!({ "title": "Free", "objectives": [{ "event": "life.slept" }], "deadline_in": 60 }),
            "missing field `reward`",
        ),
        (
            json!({ "title": "Forever", "objectives": [{ "event": "life.slept" }], "reward": 500 }),
            "missing field `deadline_in`",
        ),
        (with("objectives", json!([{ "count": 2 }])), "missing field `event`"),
    ];
    for (definition, part) in cases {
        let message = refusal(&mut w, ann, definition);
        assert!(message.contains(part), "`{message}` does not name `{part}`");
    }
    w.assert_conserved();
}

#[test]
fn the_bounds_come_from_parameters() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 10_000);
    w.set_param("missions.title_max", 4);
    w.set_param("missions.takers_max", 1);
    w.set_param("missions.reward_min", 600);
    w.set_param("missions.objectives_max", 1);
    // The zones see a new value in the next frame.
    w.step(1);
    assert_eq!(refusal(&mut w, ann, bread_mission(700, 1)), "The title has more than 4 bytes.");
    let mut definition = bread_mission(500, 1);
    definition["title"] = json!("Bake");
    assert_eq!(
        refusal(&mut w, ann, definition.clone()),
        "`reward` must be from 600 to 10000000 cents."
    );
    definition["reward"] = json!(600);
    definition["takers"] = json!(2);
    assert_eq!(refusal(&mut w, ann, definition.clone()), "`takers` must be from 1 to 1.");
    definition["takers"] = json!(1);
    definition["objectives"] = json!([{ "event": "life.slept" }, { "event": "life.meal_eaten" }]);
    assert_eq!(refusal(&mut w, ann, definition), "A mission has from 1 to 1 objectives.");
}

#[test]
fn the_defaults_and_the_widest_definition_are_accepted() {
    let mut w = world();
    let ann = rich(&mut w, "Ann", 1, 100_000);
    // Only the required fields: one event, one taker, solo.
    let small = post(
        &mut w,
        ann,
        json!({ "title": "Sleep", "objectives": [{ "event": "life.slept" }], "reward": 100, "deadline_in": 10 }),
    );
    assert_eq!(escrow(&w, small), 100);
    let read = ok(&mut w, ann, "mission_list", json!({ "mission": small }));
    assert_eq!(
        read["mission"],
        json!({
            "mission": small,
            "reward": 100,
            "mode": "solo",
            "deadline": 11,
            "taken": 0,
            "takers": 1,
            "title": { "untrusted": true, "from": ann, "text": "Sleep" },
            "poster": ann,
            "posted": 1,
            "completed": 0,
            "objectives": [
                { "event": { "untrusted": true, "from": ann, "text": "life.slept" }, "count": 1, "actor": "assignee" }
            ],
            "you": { "status": "posted" },
        })
    );

    // Every field at its limit.
    let wide = post(
        &mut w,
        ann,
        json!({
            "title": "x".repeat(80),
            "text": format!("line one\n{}", "y".repeat(991)),
            "objectives": [
                { "event": "economy.goods_produced", "fields": { "item": "bread", "qty": 2, "fresh": true, "site": -4 }, "count": 1000, "actor": "team" },
                { "event": "crime.arrest_made", "actor": "assignee" },
                { "event": "society.org_created" },
                { "event": "missions.mission_completed" },
            ],
            "mode": "team",
            "org": 4_242,
            "reward": 100,
            "deadline_in": 40_320,
            "takers": 50,
        }),
    );
    assert_eq!(escrow(&w, wide), 5_000);
    let read = ok(&mut w, ann, "mission_list", json!({ "mission": wide }));
    assert_eq!(read["mission"]["org"], json!(4_242));
    assert_eq!(read["mission"]["mode"], json!("team"));
    // The event kind and the data fields are text of the poster. They go
    // out as untrusted values; the fields are the text of a JSON object.
    assert_eq!(
        read["mission"]["objectives"][0],
        json!({
            "event": { "untrusted": true, "from": ann, "text": "economy.goods_produced" },
            "fields": { "untrusted": true, "from": ann, "text": "{\"fresh\":true,\"item\":\"bread\",\"qty\":2,\"site\":-4}" },
            "count": 1000,
            "actor": "team",
        })
    );
    assert_eq!(w.cash(ann), Money(100_000 - 100 - 5_000));
    w.assert_conserved();
}
