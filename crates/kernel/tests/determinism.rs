//! Same snapshot + same inputs => same state hash.

mod common;

use common::Demo;
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionRequest, Condition, Routine, SpawnSpec};
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{
    CityConfig, CityId, Input, OwnerId, Pos, ResidentId, Rng, World, WorldConfig, ZoneId,
};

fn config(seed: u64) -> WorldConfig {
    WorldConfig {
        seed,
        cities: vec![
            CityConfig::new("Ashford", Pos::new(0, 0), 2),
            CityConfig::new("Brine", Pos::new(500, 500), 2),
        ],
        ..WorldConfig::default()
    }
}

/// Spawns residents in both cities and gives some of them routines.
fn populate(w: &mut TestWorld<'_>) -> Vec<ResidentId> {
    let mut ids = Vec::new();
    for i in 0..40u64 {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i % 5),
            name: format!("resident {i}"),
            model: (i % 3 == 0).then(|| "test-model".to_string()),
            city: Some(CityId((i % 2) as u16)),
        };
        let id = w.world.spawn(spec).expect("spawn");
        w.give_item(id, "bread", 2);
        if i % 4 == 0 {
            let routine = Routine {
                name: "keep busy".into(),
                when: Condition::Always,
                then: ActionRequest::new("work", json!({ "ticks": 3 + i % 5 })),
            };
            w.act_ok(id, "set_routines", json!({ "routines": [routine] }));
        }
        ids.push(id);
    }
    ids
}

/// Drives the world for `frames` frames with inputs from a script
/// generator. With `restore_at`, the world is replaced by one restored from
/// its own snapshot before that frame.
fn drive(
    w: &mut TestWorld<'_>,
    ids: &[ResidentId],
    script_seed: u64,
    frames: u32,
    restore_at: Option<u32>,
) {
    let mut rng = Rng::seeded(script_seed);
    for frame in 0..frames {
        if restore_at == Some(frame) {
            let hash = w.hash();
            let bytes = w.world.snapshot();
            w.world = World::restore(&[&Demo], &bytes).expect("the snapshot restores");
            assert_eq!(w.hash(), hash, "a restored world has the hash of the saved one");
        }
        step_script(w, ids, &mut rng, frame);
    }
}

fn run(seed: u64, script_seed: u64, frames: u32) -> TestWorld<'static> {
    let mut w = TestWorld::with_config(&[&Demo], config(seed));
    let ids = populate(&mut w);
    drive(&mut w, &ids, script_seed, frames, None);
    w
}

#[test]
fn two_worlds_with_the_same_seed_and_inputs_have_the_same_hash() {
    let a = run(7, 99, 400);
    let b = run(7, 99, 400);
    assert_eq!(a.hash(), b.hash());
    assert_eq!(a.world.frame(), 400);
    // The scenario did what it says: residents moved between zones, money
    // moved, the global realm worked.
    let moved: u32 = (0..8).map(|z| a.zone(ZoneId(z)).len() as u32).sum();
    assert_eq!(moved, 39, "one resident left the world");
    assert!(
        a.zone(ZoneId(1)).len() + a.zone(ZoneId(2)).len() + a.zone(ZoneId(3)).len() > 0,
        "nobody migrated"
    );
    assert!(a.global().res::<common::GlobalLog>().pings > 5);
    assert!(a.assert_conserved().money.minted > 40_000);

    // The hash is sensitive: another world seed, other inputs, or one more
    // frame give another hash.
    assert_ne!(run(8, 99, 400).hash(), a.hash(), "the world seed is part of the state");
    assert_ne!(run(7, 100, 400).hash(), a.hash());
    assert_ne!(run(7, 99, 401).hash(), a.hash());
}

#[test]
fn a_world_restored_from_a_snapshot_reaches_the_same_hash() {
    // The original runs 400 frames in one go.
    let original = run(7, 99, 400);

    // The copy runs 180 frames, is saved and restored, and is fed the same
    // later inputs.
    for at in [1, 180, 399] {
        let mut copy = TestWorld::with_config(&[&Demo], config(7));
        let ids = populate(&mut copy);
        drive(&mut copy, &ids, 99, 400, Some(at));
        assert_eq!(copy.hash(), original.hash(), "restored before frame {at}");
        copy.assert_conserved();
    }
}

/// One frame of the script: residents act, walk across zone borders, pay
/// each other across zones, use the global realm, shout and whisper.
fn step_script(w: &mut TestWorld<'_>, ids: &[ResidentId], rng: &mut Rng, frame: u32) {
    for _ in 0..8 {
        let who = ids[rng.below(ids.len() as u64) as usize];
        let other = ids[rng.below(ids.len() as u64) as usize];
        if !w.is_alive(who) {
            continue;
        }
        let (verb, args) = match rng.below(12) {
            0 | 1 => ("step", json!({ "dx": rng.range(-1, 2), "dy": rng.range(-1, 2) })),
            2 => ("work", json!({ "ticks": rng.range(1, 8) })),
            3 => ("pay", json!({ "to": other, "amount": rng.range(1, 400) })),
            4 => ("shout", json!({ "text": format!("frame {frame}") })),
            5 => ("whisper", json!({ "to": [other], "text": "psst" })),
            6 => ("ping", json!({ "n": rng.below(20) })),
            7 => ("deposit", json!({ "amount": rng.range(1, 100) })),
            8 => {
                let zone = w.zone_of(who);
                let base = zone.0 / 4 * 4;
                let target = ZoneId(base + rng.below(4) as u32);
                let bounds = w.world.layout().zone(target).expect("zone").bounds;
                ("jump", json!({ "zone": target, "x": bounds.x0 + 3, "y": bounds.y0 + 3 }))
            }
            9 => ("eat", Value::Null),
            10 => ("offer", json!({ "ticks": 4 })),
            _ => ("cancel", Value::Null),
        };
        let _ = w.act(who, verb, args);
    }
    if frame.is_multiple_of(7) {
        let who = ids[rng.below(ids.len() as u64) as usize];
        if w.is_alive(who) {
            let zone = w.zone_of(who);
            let bounds = w.world.layout().zone(zone).expect("zone").bounds;
            let col = zone.0 % 2;
            let edge = if col == 0 {
                Pos::new(bounds.x1 - 1, bounds.y0 + 5)
            } else {
                Pos::new(bounds.x0, bounds.y0 + 5)
            };
            w.set_pos(who, edge);
            let dx = if col == 0 { 1 } else { -1 };
            let _ = w.act(who, "step", json!({ "dx": dx, "dy": 0 }));
        }
    }
    if frame == 150 {
        let _ = w.act(ids[3], "leave", json!({ "heir": ids[4] }));
        w.set_param("demo.work_pay", 620);
    }
    if frame % 31 == 30 {
        w.world.apply(Input::Step { skip: vec![ZoneId(rng.below(8) as u32)] });
    } else {
        w.step(1);
    }
}

#[test]
fn actions_that_end_in_the_same_tick_end_in_resident_order() {
    // Bob's action is scheduled first and Ada's second, for the same tick.
    // Both have a routine, so each takes a new action id when its action
    // ends: the order in which the two actions end is visible in the state.
    // A restored world schedules them again in table order (Ada, Bob). The
    // kernel must end them in the same order in both worlds.
    let scenario = |restore: bool| {
        let mut w = TestWorld::with_config(&[&Demo], config(5));
        let ada = w.spawn("Ada");
        let bob = w.spawn("Bob");
        let routine = |ticks: u32| {
            json!({ "routines": [Routine {
                name: "work".into(),
                when: Condition::Always,
                then: ActionRequest::new("work", json!({ "ticks": ticks })),
            }] })
        };
        w.act_ok(bob, "set_routines", routine(5));
        w.step(2);
        w.act_ok(ada, "set_routines", routine(3));
        w.step(2);
        // Bob works since tick 1, Ada since tick 3. Both end at tick 6.
        let ends = |w: &TestWorld<'_>, r| {
            w.with_resident(r, |realm, idx| realm.activity(idx).map(|a| a.ends.0))
        };
        assert_eq!((ends(&w, ada), ends(&w, bob)), (Some(6), Some(6)));
        if restore {
            w.snapshot_restore();
        }
        w.step(3);
        let ids = |w: &TestWorld<'_>, r| {
            w.with_resident(r, |realm, idx| realm.activity(idx).map(|a| a.action_id))
        };
        (w.hash(), ids(&w, ada), ids(&w, bob))
    };
    let (hash, ada_action, bob_action) = scenario(false);
    assert!(
        ada_action < bob_action,
        "Ada has the lower id, so her action ended first and got the next id first"
    );
    assert_eq!(scenario(true), (hash, ada_action, bob_action));
}

#[test]
fn the_result_does_not_depend_on_the_number_of_threads() {
    let hash_with = |threads: usize| {
        let pool =
            rayon::ThreadPoolBuilder::new().num_threads(threads).build().expect("a thread pool");
        pool.install(|| run(3, 5, 120).hash())
    };
    let one = hash_with(1);
    assert_eq!(hash_with(2), one);
    assert_eq!(hash_with(8), one);
}

#[test]
fn the_determinism_helper_checks_both_properties() {
    let hash = assert_deterministic(
        &[&Demo],
        &config(21),
        |w| {
            let ada = w.spawn("Ada");
            let bob = w.spawn_at("Bob", Pos::new(100, 100));
            w.act_ok(ada, "work", json!({ "ticks": 30 }));
            w.act_ok(bob, "ping", json!({ "n": 1 }));
            w.step(10);
        },
        |w| {
            let ada = ResidentId(1);
            let bob = ResidentId(2);
            w.act_ok(bob, "pay", json!({ "to": ada, "amount": 5 }));
            w.act_ok(ada, "shout", json!({ "text": "hi" }));
            w.step(40);
        },
    );
    assert_eq!(hash.len(), 64);
    assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
}

#[test]
fn random_numbers_come_from_the_seed() {
    let draw = |seed: u64, zone: u32| {
        let w = TestWorld::with_config(&[&Demo], config(seed));
        w.zone(ZoneId(zone)).rng().next_u64()
    };
    assert_eq!(draw(1, 0), draw(1, 0));
    assert_ne!(draw(1, 0), draw(2, 0), "another seed, another stream");
    assert_ne!(draw(1, 0), draw(1, 1), "every realm has its own stream");
    // Drawing a number is a change of state.
    let w = TestWorld::with_config(&[&Demo], config(1));
    let before = w.hash();
    let _ = w.global().rng().next_u64();
    assert_ne!(w.hash(), before);
}
