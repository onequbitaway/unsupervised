//! `Realm::set_routines` for module code: default routines for a new
//! resident (architecture.md, section 5.3).

mod common;

use common::Demo;
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionRequest, Condition, Detail, ErrorCode, ObserveOpts, Routine};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{ActionError, Builder, Plugin, ResidentId};

fn routine(name: &str, verb: &str, args: Value) -> Routine {
    Routine { name: name.into(), when: Condition::Always, then: ActionRequest::new(verb, args) }
}

/// A module that gives every new resident a routine, so that it does
/// something when its owner is away.
struct Defaults;

impl Plugin for Defaults {
    fn name(&self) -> &'static str {
        "defaults"
    }

    fn build(&self, b: &mut Builder) {
        b.on_spawn(10, |realm, idx| {
            let rest = routine("rest", "nap", json!({ "ticks": 3 }));
            realm.set_routines(idx, &[rest]).expect("the default routine is valid");
        });
    }
}

/// The routines of a resident as its owner sees them: `(name, runs)`.
fn routines(w: &TestWorld<'_>, r: ResidentId) -> Vec<(String, u64)> {
    let full = w.observe_with(r, &ObserveOpts { detail: Detail::Full, ..Default::default() });
    full.sections["self"]["routines"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            (
                entry["name"].as_str().expect("a name").to_string(),
                entry["runs"].as_u64().expect("a count"),
            )
        })
        .collect()
}

fn set(w: &TestWorld<'_>, r: ResidentId, list: &[Routine]) -> Result<(), ActionError> {
    w.with_resident(r, |realm, idx| realm.set_routines(idx, list))
}

#[test]
fn a_spawn_hook_gives_a_new_resident_its_default_routines() {
    let mut w = TestWorld::new(&[&Demo, &Defaults]);
    let ada = w.spawn("Ada");
    assert_eq!(routines(&w, ada), [("rest".to_string(), 0)]);
    // Nobody sent a request for Ada. The routine starts her first action
    // in the first tick, and the next one when that ends.
    assert_eq!(w.activity(ada), None);
    w.step(1);
    assert_eq!(w.activity(ada), Some("nap"));
    w.step(3);
    assert_eq!(routines(&w, ada), [("rest".to_string(), 1)]);
    assert_eq!(w.activity(ada), Some("nap"), "the routine fired again");
    // The owner can still replace the default with the verb.
    w.act_ok(ada, "set_routines", json!({ "routines": [] }));
    assert_eq!(routines(&w, ada), []);
    // The world with the hook is deterministic and restores.
    w.snapshot_restore();
    w.step(5);
    assert_eq!(w.activity(ada), None);
}

#[test]
fn the_call_replaces_the_routines_and_refuses_a_bad_list_as_a_whole() {
    let mut w = TestWorld::new(&[&Demo, &Defaults]);
    let ada = w.spawn("Ada");

    let two =
        [routine("a", "wait", json!({ "ticks": 2 })), routine("b", "work", json!({ "ticks": 1 }))];
    assert_eq!(set(&w, ada, &two), Ok(()));
    assert_eq!(routines(&w, ada), [("a".to_string(), 0), ("b".to_string(), 0)]);

    // More than `kernel.routines_max`.
    assert_eq!(w.param("kernel.routines_max"), 8);
    let nine: Vec<Routine> =
        (0..9).map(|n| routine(&format!("r{n}"), "wait", json!({ "ticks": 1 }))).collect();
    let error = set(&w, ada, &nine).expect_err("nine routines are one too many");
    assert_eq!(
        (error.code, error.message.as_str()),
        (ErrorCode::InvalidArgs, "A resident has at most 8 routines.")
    );
    assert_eq!(set(&w, ada, &nine[..8]), Ok(()));
    assert_eq!(routines(&w, ada).len(), 8);
    assert_eq!(set(&w, ada, &two), Ok(()));
    // The limit is the parameter, also when it changes.
    w.set_param("kernel.routines_max", 1);
    w.step(1);
    assert_eq!(set(&w, ada, &two).map_err(|e| e.code), Err(ErrorCode::InvalidArgs));
    assert_eq!(set(&w, ada, &two[..1]), Ok(()));
    assert_eq!(routines(&w, ada), [("a".to_string(), 0)]);

    // One bad routine refuses the list, and the old list stays.
    w.set_param("kernel.routines_max", 8);
    w.step(1);
    let bad = [
        (routine("x", "fly", Value::Null), ErrorCode::UnknownVerb),
        (routine("x", "wait", json!({ "ticks": "long" })), ErrorCode::InvalidArgs),
        (routine("x", "cancel", Value::Null), ErrorCode::InvalidArgs),
        (routine(&"n".repeat(65), "wait", json!({ "ticks": 1 })), ErrorCode::InvalidArgs),
        (
            Routine {
                when: Condition::AttrBelow { attr: "mana".into(), value: 1 },
                ..routine("x", "wait", json!({ "ticks": 1 }))
            },
            ErrorCode::InvalidArgs,
        ),
    ];
    for (wrong, code) in bad {
        let list = [routine("ok", "wait", json!({ "ticks": 1 })), wrong];
        assert_eq!(set(&w, ada, &list).map_err(|e| e.code), Err(code));
        assert_eq!(routines(&w, ada), [("a".to_string(), 0)]);
    }
    // An empty list removes the routines: Ada stops when her action ends.
    assert_eq!(set(&w, ada, &[]), Ok(()));
    w.step(6);
    assert_eq!(w.activity(ada), None);
}
