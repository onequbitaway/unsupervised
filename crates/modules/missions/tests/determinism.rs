//! Determinism and conservation under a long random script: the same seed
//! and the same inputs give the same state hash, also across a snapshot,
//! and no cent is ever created or lost outside the faucets and the sinks.

mod common;

use std::collections::BTreeMap;

use common::{burned, minted, plugins, spawn_as, sponsor};
use serde_json::{Value, json};
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Holder, Money, Pos, ResidentId, Rng, WorldConfig};
use unsup_missions::{Missions, SEASON_SINK, SPONSOR_FAUCET, SPONSOR_SINK, Seasons};

/// Cash of each resident at the start.
const START_CASH: i64 = 150_000;
/// Residents in the script.
const RESIDENTS: u64 = 10;
/// The organizations that residents join and leave.
const ORGS: [u64; 3] = [5_000, 6_000, 7_000];

/// Makes the residents of the script. They have four owners and three
/// models, they stand in all four zones, and each has cash. A season lasts
/// one game day, and the zones report and the boards publish often.
fn populate(w: &mut TestWorld<'_>) -> Vec<ResidentId> {
    w.set_param("kernel.free_actions_per_tick", 50);
    w.set_param("missions.season_days", 1);
    w.set_param("missions.report_period", 20);
    w.set_param("missions.board_period", 40);
    w.set_param("missions.sponsor_daily_limit", 20_000);
    w.set_param("missions.posted_max", 3);
    let models = [Some("alpha"), Some("beta"), None];
    let ids: Vec<ResidentId> = (0..RESIDENTS)
        .map(|i| {
            let id = spawn_as(w, &format!("R{i}"), i % 4 + 1, models[(i % 3) as usize]);
            w.give_money(id, Money(START_CASH));
            let (x, y) = (20 + 30 * (i % 4) as i32, 20 + 40 * (i / 4) as i32);
            w.set_pos(id, Pos::new(x, y));
            id
        })
        .collect();
    w.step(1);
    ids
}

/// Makes a random definition: sometimes a bad one.
fn definition(rng: &mut Rng) -> Value {
    let events = [
        ("economy.goods_produced", json!({ "item": "bread" })),
        ("economy.goods_produced", json!({})),
        ("economy.shift_completed", json!({})),
        ("crime.arrest_made", json!({ "watch": false })),
        ("missions.mission_completed", json!({})),
        ("society.said", json!({})),
    ];
    let team = rng.chance(1, 3);
    let objectives: Vec<Value> = (0..rng.range(1, 3))
        .map(|_| {
            let (event, fields) = &events[rng.below(events.len() as u64) as usize];
            let actor = if team && rng.chance(1, 2) { "team" } else { "assignee" };
            json!({ "event": event, "fields": fields, "count": rng.range(1, 3), "actor": actor })
        })
        .collect();
    let mut out = json!({
        "title": format!("Mission {}", rng.below(1_000)),
        "objectives": objectives,
        "mode": if team { "team" } else { "solo" },
        "reward": rng.range(50, 900),
        "deadline_in": rng.range(5, 90),
        "takers": rng.range(1, 3),
    });
    if team && rng.chance(1, 3) {
        out["org"] = json!(ORGS[rng.below(3) as usize]);
    }
    out
}

/// Returns the id of an open mission, or a number that is no mission.
fn some_mission(w: &TestWorld<'_>, rng: &mut Rng) -> u64 {
    let ids: Vec<u64> = w.global().res::<Missions>().ids().collect();
    ids.get(rng.below(ids.len() as u64 + 1) as usize).copied().unwrap_or(1)
}

/// One frame of the script: a few residents act at random, then the world
/// steps. Returns the kinds of the public events of the frame.
fn script_frame(
    w: &mut TestWorld<'_>,
    ids: &[ResidentId],
    rng: &mut Rng,
    frame: u32,
) -> Vec<String> {
    for _ in 0..4 {
        let who = ids[rng.below(ids.len() as u64) as usize];
        let other = ids[rng.below(ids.len() as u64) as usize];
        if !w.is_alive(who) {
            continue;
        }
        let org = ORGS[rng.below(3) as usize];
        let mission = some_mission(w, rng);
        let (verb, args) = match rng.below(26) {
            0..=2 => ("mission_post", definition(rng)),
            3..=5 => ("mission_accept", json!({ "mission": mission })),
            6..=8 => ("mission_accept", json!({ "mission": mission, "org": org })),
            9 => ("mission_abandon", json!({ "mission": mission })),
            10 => ("mission_claim", json!({ "mission": mission })),
            11 => ("mission_list", json!({ "min_reward": rng.range(0, 500) })),
            12 => ("leaderboard", json!({ "board": "wealth", "by": "model" })),
            13..=16 => (
                "produce_test",
                json!({ "item": if rng.chance(3, 4) { "bread" } else { "cake" }, "qty": 1 }),
            ),
            17..=18 => ("shift_test", json!({})),
            19 => ("arrest_test", json!({ "target": other })),
            20..=21 => ("join_test", json!({ "org": org })),
            22 => ("leave_test", json!({ "org": org })),
            23 => {
                sponsor(w, u64::from(frame), "Acme", definition(rng));
                ("leaderboard", json!({ "board": "missions" }))
            }
            _ => {
                w.set_pos(who, Pos::new(rng.range(0, 128) as i32, rng.range(0, 128) as i32));
                ("say_test", json!({}))
            }
        };
        // Many requests are refused: that is part of the script.
        w.act(who, verb, args);
    }
    match frame {
        // Two residents leave the world in the middle of everything: one
        // dies with an heir, one is removed.
        300 => {
            w.act(ids[3], "die_test", json!({ "heir": ids[4] }));
        }
        700 => w.despawn(ids[7]),
        _ => {}
    }
    w.step(1).into_iter().flat_map(|out| out.events).map(|event| event.kind).collect()
}

/// Runs the script for `frames` frames. With `restore_at`, the world is
/// replaced by its own snapshot before that frame. Returns how often each
/// kind of public event happened.
fn drive(
    w: &mut TestWorld<'_>,
    ids: &[ResidentId],
    seed: u64,
    frames: u32,
    restore_at: Option<u32>,
) -> BTreeMap<String, u32> {
    let mut rng = Rng::seeded(seed);
    let mut seen = BTreeMap::new();
    for frame in 0..frames {
        if restore_at == Some(frame) {
            w.snapshot_restore();
        }
        for kind in script_frame(w, ids, &mut rng, frame) {
            *seen.entry(kind).or_insert(0) += 1;
        }
    }
    seen
}

/// Returns the money in all escrows of the global realm.
fn escrowed(w: &TestWorld<'_>) -> i64 {
    w.global()
        .ledger()
        .balances()
        .filter(|(holder, _)| matches!(holder, Holder::Escrow(_)))
        .map(|(_, money)| money.cents())
        .sum()
}

#[test]
fn money_is_conserved_after_every_frame_of_a_long_script() {
    let mut w = TestWorld::new(&plugins());
    let ids = populate(&mut w);
    let mut rng = Rng::seeded(1);
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    // 1,500 frames: one season ends at frame 1,440.
    for frame in 0..1_500 {
        for kind in script_frame(&mut w, &ids, &mut rng, frame) {
            *seen.entry(kind).or_insert(0) += 1;
        }
        let totals = w.assert_conserved();
        // The only faucet is the sponsor faucet; the only sinks are the two
        // sinks of this module.
        let created = minted(&w, SPONSOR_FAUCET);
        let destroyed = burned(&w, SPONSOR_SINK) + burned(&w, SEASON_SINK);
        assert_eq!(totals.money.minted, START_CASH * RESIDENTS as i64 + created, "frame {frame}");
        assert_eq!(totals.money.burned, destroyed, "frame {frame}");
        assert_eq!(
            totals.money.held + totals.money.in_flight,
            totals.money.minted - totals.money.burned,
            "frame {frame}"
        );
    }
    // The script really used the module: every kind of thing happened.
    for kind in [
        "missions.mission_posted",
        "missions.mission_accepted",
        "missions.mission_abandoned",
        "missions.mission_completed",
        "missions.mission_closed",
        "missions.sponsor_refused",
        "missions.leaderboard_published",
        "missions.season_started",
        "missions.season_ended",
        "missions.hall_of_fame_published",
        "missions.season_reset_applied",
        "kernel.despawned",
    ] {
        assert!(
            seen.get(kind).copied().unwrap_or(0) > 0,
            "the script never caused `{kind}`: {seen:?}"
        );
    }
    assert!(minted(&w, SPONSOR_FAUCET) > 0 && burned(&w, SPONSOR_SINK) > 0);
    assert!(burned(&w, SEASON_SINK) > 0);
    assert_eq!(w.global().res::<Seasons>().number(), 2);

    // Let every open mission run out. Then no escrow holds money, and the
    // engine has no mission and no taker left.
    w.step(120);
    let state = w.global().res::<Missions>();
    assert_eq!(state.open_count(), 0);
    for &id in &ids {
        assert_eq!(state.accepted_by(id), [] as [u64; 0]);
    }
    drop(state);
    assert_eq!(escrowed(&w), 0);
    w.assert_conserved();
}

#[test]
fn the_same_script_gives_the_same_hash_and_another_script_another_one() {
    let run = |seed: u64| {
        let mut w = TestWorld::new(&plugins());
        let ids = populate(&mut w);
        drive(&mut w, &ids, seed, 400, None);
        w.hash()
    };
    assert_eq!(run(5), run(5));
    assert_ne!(run(5), run(6));
}

#[test]
fn a_world_restored_from_a_snapshot_reaches_the_same_hash() {
    let original = {
        let mut w = TestWorld::new(&plugins());
        let ids = populate(&mut w);
        let seen = drive(&mut w, &ids, 9, 1_500, None);
        (w.hash(), seen)
    };
    // The snapshot is taken with open missions, takers with progress, money
    // in escrows and wires in flight; and once more after the season ended.
    for restore_at in [130, 777, 1_441] {
        let mut w = TestWorld::new(&plugins());
        let ids = populate(&mut w);
        let seen = drive(&mut w, &ids, 9, 1_500, Some(restore_at));
        assert_eq!((w.hash(), seen), original, "restored before frame {restore_at}");
    }
}

#[test]
fn a_scripted_scenario_is_deterministic() {
    let setup = |w: &mut TestWorld<'static>| {
        let ids = populate(w);
        w.act(ids[0], "join_test", json!({ "org": 5_000 }));
        w.act(ids[1], "join_test", json!({ "org": 5_000 }));
        let team = json!({
            "title": "Team bread",
            "objectives": [{ "event": "economy.goods_produced", "count": 3, "actor": "team" }],
            "mode": "team",
            "reward": 901,
            "deadline_in": 60,
            "takers": 2,
        });
        w.act(ids[2], "mission_post", team);
        sponsor(w, 1, "Acme", common::bread_mission(700, 2));
        w.step(2);
    };
    let run = |w: &mut TestWorld<'static>| {
        let missions: Vec<u64> = w.global().res::<Missions>().ids().collect();
        assert_eq!(missions.len(), 2);
        let ids: Vec<ResidentId> = (1..=RESIDENTS).map(ResidentId).collect();
        w.act(ids[0], "mission_accept", json!({ "mission": missions[0], "org": 5_000 }));
        w.act(ids[1], "mission_accept", json!({ "mission": missions[0], "org": 5_000 }));
        w.act(ids[5], "mission_accept", json!({ "mission": missions[1] }));
        w.step(2);
        for baker in [ids[0], ids[1], ids[1], ids[5], ids[5]] {
            w.act(baker, "produce_test", json!({ "item": "bread", "qty": 1 }));
        }
        // Past both deadlines and past the end of the season.
        w.step(1_500);
        assert_eq!(w.global().res::<Missions>().open_count(), 0);
        w.assert_conserved();
    };
    assert_deterministic(&plugins(), &WorldConfig::default(), setup, run);
}
