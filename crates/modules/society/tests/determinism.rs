//! Determinism and conservation under a long random script: the same seed
//! and the same inputs give the same state hash, also across a snapshot,
//! and no cent or loaf is ever created or lost.

mod common;

use std::collections::BTreeMap;

use common::{DEPOT, HOME, give_site, plugins, site, spawn_owned};
use serde_json::{Value, json};
use unsup_kernel::testing::{TestWorld, assert_deterministic};
use unsup_kernel::{Money, Pos, ResidentId, Rng, WorldConfig};
use unsup_society::{Contracts, Orgs};

/// Cash of each resident at the start.
const START_CASH: i64 = 20_000;
/// Loaves of each resident at the start.
const START_BREAD: i64 = 10;
/// Residents in the script.
const RESIDENTS: u64 = 12;

/// Makes the residents of the script: each has its own owner, cash and
/// bread. They stand in all four zones. The first one owns the home.
fn populate(w: &mut TestWorld<'_>) -> Vec<ResidentId> {
    // The script acts faster than residents normally do.
    w.set_param("society.opinion_min_age", 0);
    w.set_param("society.org_create_fee", 500);
    w.set_param("kernel.free_actions_per_tick", 50);
    let ids: Vec<ResidentId> = (0..RESIDENTS)
        .map(|i| {
            let id = spawn_owned(w, &format!("R{i}"), i + 1);
            w.give_money(id, Money(START_CASH));
            w.give_item(id, "bread", START_BREAD);
            let (x, y) = (20 + 30 * (i % 4) as i32, 20 + 40 * (i / 4) as i32);
            w.set_pos(id, Pos::new(x, y));
            id
        })
        .collect();
    give_site(w, 0, site(w, 0, "home"), ids[0]);
    w.step(1);
    ids
}

/// Returns the id of an organization of a resident, if it has one.
fn org_of(w: &TestWorld<'_>, r: ResidentId, rng: &mut Rng) -> u64 {
    let orgs = w.global().res::<Orgs>();
    let own: Vec<u64> = orgs.of_member(r).map(|org| org.id().0).collect();
    own.get(rng.below(own.len() as u64) as usize).copied().unwrap_or(1)
}

/// Returns the id of a contract that a resident can see, if there is one.
fn contract_of(w: &TestWorld<'_>, r: ResidentId, rng: &mut Rng) -> u64 {
    let section = w.observe(r).sections.get("contracts").cloned().unwrap_or(Value::Null);
    let list = section["list"].as_array().cloned().unwrap_or_default();
    let pick = list.get(rng.below(list.len() as u64) as usize);
    pick.and_then(|entry| entry["contract"].as_u64()).unwrap_or(0)
}

/// Makes the clauses of a random contract.
fn clauses(w: &TestWorld<'_>, rng: &mut Rng, target: ResidentId) -> Value {
    let depot = site(w, 0, "warehouse");
    let mut out = Vec::new();
    for _ in 0..rng.range(1, 5) {
        let by = if rng.chance(1, 2) { "offerer" } else { "counterparty" };
        let amount = rng.range(1, 400);
        let due_in = rng.range(2, 60);
        out.push(match rng.below(6) {
            0 => json!({ "kind": "pay_at_signing", "by": by, "amount": amount }),
            1 => json!({ "kind": "pay_at_deadline", "by": by, "amount": amount, "due_in": due_in }),
            2 => json!({ "kind": "pay_on_event", "by": by, "amount": amount, "event": "crime.arrest_made", "fields": { "target": target }, "due_in": due_in }),
            3 => json!({ "kind": "deliver", "by": by, "item": "bread", "qty": rng.range(1, 3), "site": depot, "due_in": due_in }),
            4 => json!({ "kind": "deliver", "by": by, "item": "bread", "qty": 1, "due_in": due_in }),
            _ => json!({ "kind": "penalty", "by": by, "amount": amount }),
        });
    }
    Value::Array(out)
}

/// One frame of the script: a few residents act at random, then the world
/// steps. Returns the kinds of the public events of the frame.
fn script_frame(
    w: &mut TestWorld<'_>,
    ids: &[ResidentId],
    rng: &mut Rng,
    frame: u32,
) -> Vec<String> {
    for _ in 0..5 {
        let who = ids[rng.below(ids.len() as u64) as usize];
        let other = ids[rng.below(ids.len() as u64) as usize];
        if !w.is_alive(who) {
            continue;
        }
        let (verb, args) = match rng.below(30) {
            0 => ("say", json!({ "text": format!("frame {frame}") })),
            1 => ("message", json!({ "to": other, "text": "psst" })),
            2 => ("post", json!({ "text": format!("news of frame {frame}") })),
            3 => (
                "org_create",
                json!({ "name": format!("Org {}", rng.below(8)), "open": rng.chance(1, 2) }),
            ),
            4 => ("org_invite", json!({ "org": org_of(w, who, rng), "resident": other })),
            5 | 6 => ("org_join", json!({ "org": org_of(w, other, rng) })),
            7 => ("org_leave", json!({ "org": org_of(w, who, rng) })),
            8 => ("org_kick", json!({ "org": org_of(w, who, rng), "resident": other })),
            9 => {
                ("org_deposit", json!({ "org": org_of(w, who, rng), "amount": rng.range(1, 300) }))
            }
            10 => (
                "org_pay",
                json!({ "org": org_of(w, who, rng), "amount": rng.range(1, 200), "to_resident": other }),
            ),
            11 => ("org_say", json!({ "org": org_of(w, who, rng), "text": "meeting" })),
            12 => (
                "org_define_role",
                json!({ "org": org_of(w, who, rng), "role": "clerk", "permissions": ["treasury.spend", "org.invite"] }),
            ),
            13 => (
                "org_set_role",
                json!({ "org": org_of(w, who, rng), "resident": other, "role": "clerk" }),
            ),
            14..=16 => {
                let to = if rng.chance(1, 4) { Value::Null } else { json!(other) };
                (
                    "contract_offer",
                    json!({ "to": to, "clauses": clauses(w, rng, other), "expires_in": rng.range(2, 40) }),
                )
            }
            17..=19 => (
                "contract_sign",
                json!({ "contract": contract_of(w, who, rng), "stake": rng.range(0, 900) }),
            ),
            20 => ("contract_claim", json!({ "contract": contract_of(w, who, rng) })),
            21 => ("contract_cancel", json!({ "contract": contract_of(w, who, rng) })),
            22 => ("contract_reject", json!({ "contract": contract_of(w, who, rng) })),
            23 => ("propose", json!({ "resident": other })),
            24 => ("accept_proposal", json!({ "resident": other })),
            25 => ("divorce", json!({})),
            26 => ("relate", json!({ "resident": other, "label": "rival" })),
            27 => (
                "opine",
                json!({ "resident": other, "verdict": if rng.chance(1, 2) { "praise" } else { "condemn" } }),
            ),
            28 => ("arrest_test", json!({ "target": other })),
            _ => {
                // Walk somewhere: to the depot, to the home, or to any tile.
                let pos = match rng.below(4) {
                    0 => Pos::new(DEPOT.x0 + 1, DEPOT.y0 + 1),
                    1 => Pos::new(HOME.x0 + 1, HOME.y0 + 1),
                    _ => Pos::new(rng.range(0, 128) as i32, rng.range(0, 128) as i32),
                };
                w.set_pos(who, pos);
                ("household_invite", json!({ "resident": other }))
            }
        };
        // Many requests are refused: that is part of the script.
        w.act(who, verb, args);
    }
    if frame == 150 {
        // One resident leaves the world in the middle of everything.
        w.despawn(ids[3]);
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

#[test]
fn money_and_goods_are_conserved_after_every_frame_of_a_long_script() {
    let mut w = TestWorld::new(&plugins());
    let ids = populate(&mut w);
    let bread = w.world.blueprint().item_id("bread").expect("the fixture has bread");
    let mut rng = Rng::seeded(1);
    let mut seen: BTreeMap<String, u32> = BTreeMap::new();
    for frame in 0..600 {
        for kind in script_frame(&mut w, &ids, &mut rng, frame) {
            *seen.entry(kind).or_insert(0) += 1;
        }
        let totals = w.assert_conserved();
        assert_eq!(
            totals.money.held + totals.money.in_flight,
            START_CASH * RESIDENTS as i64,
            "frame {frame}"
        );
        assert_eq!(totals.money.burned, 0);
        let loaves = totals.items[&bread];
        assert_eq!(loaves.held + loaves.in_flight, START_BREAD * RESIDENTS as i64, "frame {frame}");
    }
    // The script really used the module: every kind of thing happened.
    for kind in [
        "society.said",
        "society.posted",
        "society.org_created",
        "society.org_joined",
        "society.org_left",
        "society.org_deposited",
        "society.org_paid",
        "society.contract_signed",
        "society.contract_fulfilled",
        "society.contract_breached",
        "society.contract_expired",
        "society.married",
        "society.opinion_given",
        "kernel.despawned",
    ] {
        assert!(
            seen.get(kind).copied().unwrap_or(0) > 0,
            "the script never caused `{kind}`: {seen:?}"
        );
    }
    // Let every open contract and offer run out. Then no escrow holds money.
    w.step(120);
    assert!(w.global().res::<Contracts>().is_empty());
    let escrowed: i64 = w
        .global()
        .ledger()
        .balances()
        .filter(|(holder, _)| matches!(holder, unsup_kernel::Holder::Escrow(_)))
        .map(|(_, money)| money.cents())
        .sum();
    assert_eq!(escrowed, 0);
    w.assert_conserved();
}

#[test]
fn the_same_script_gives_the_same_hash_and_another_script_another_one() {
    let run = |seed: u64| {
        let mut w = TestWorld::new(&plugins());
        let ids = populate(&mut w);
        drive(&mut w, &ids, seed, 300, None);
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
        let seen = drive(&mut w, &ids, 5, 300, None);
        assert!(seen.contains_key("society.contract_signed"), "{seen:?}");
        w.hash()
    };
    // The copy is saved and restored in the middle of the script: with open
    // offers, signed contracts, organizations, marriages and messages in
    // flight. `snapshot_restore` itself checks that the hash stays the same.
    for at in [1, 77, 150, 299] {
        let mut w = TestWorld::new(&plugins());
        let ids = populate(&mut w);
        drive(&mut w, &ids, 5, 300, Some(at));
        assert_eq!(w.hash(), original, "restored at frame {at}");
        w.assert_conserved();
    }
}

#[test]
fn a_scripted_scenario_is_deterministic_with_the_kernel_check() {
    // `assert_deterministic` runs the setup and the run in two fresh worlds,
    // and once more with a snapshot between them.
    let hash = assert_deterministic(
        &plugins(),
        &WorldConfig::with_seed(3),
        |w| {
            let ids = populate(w);
            let mut rng = Rng::seeded(21);
            for frame in 0..120 {
                script_frame(w, &ids, &mut rng, frame);
            }
        },
        |w| {
            let ids: Vec<ResidentId> = (1..=RESIDENTS).map(ResidentId).collect();
            let mut rng = Rng::seeded(22);
            for frame in 120..260 {
                script_frame(w, &ids, &mut rng, frame);
            }
        },
    );
    assert_eq!(hash.len(), 64);
}
