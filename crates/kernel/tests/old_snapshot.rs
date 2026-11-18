//! A snapshot that an older kernel wrote must still restore, and the world
//! must go on as it did in the older kernel.
//!
//! `fixtures/before_compact_inbox.snapshot` is the snapshot of the town of
//! [`town`], written by the kernel as it was before these changes: inbox
//! entries, action arguments and remembered answers packed as bytes, the
//! supply cap, the search for the nearest residents, newcomers spread over
//! the tiles of a spawn site.
//! `fixtures/before_compact_inbox.jsonl` holds what every resident of that
//! town observed in the older kernel: at the snapshot, and after
//! [`go_on`]. One line is one observation.
//!
//! The older kernel put every newcomer on the centre tile of the plaza.
//! [`town`] names the tile of every resident, so that this kernel builds
//! the same town from the same inputs.

mod common;

use common::{Demo, PLAZA};
use serde_json::{Value, json};
use unsup_kernel::protocol::{ActionRequest, ActionStatus, Detail, ObserveOpts};
use unsup_kernel::testing::TestWorld;
use unsup_kernel::{Pos, ResidentId, WorldConfig, ZoneId};

const SNAPSHOT: &[u8] = include_bytes!("fixtures/before_compact_inbox.snapshot");
const OBSERVED: &str = include_str!("fixtures/before_compact_inbox.jsonl");

/// Residents of the town.
const RESIDENTS: u64 = 24;

/// Residents that stay on the centre tile of the plaza: more than
/// `kernel.nearby_max`, so that the nearby list is cut.
const ON_THE_CENTRE: u64 = 14;

fn r(n: u64) -> ResidentId {
    ResidentId(n)
}

/// Sends a request with an idempotency key.
fn act_with_id(w: &mut TestWorld<'_>, who: ResidentId, key: &str, verb: &str, args: Value) {
    let ack = w.act_request(who, ActionRequest::new(verb, args).with_id(key));
    assert!(ack.status.is_accepted(), "{verb}: {:?}", ack.status);
}

/// Builds a town that uses every kind of state that a snapshot holds:
/// residents in all four zones and near the borders, running, waiting and
/// planned actions, routines, a condition with an end, notices that many
/// residents share, full inboxes, reports with results and with errors,
/// money in the bank, a resident that left the world, and messages on the
/// way.
fn town() -> TestWorld<'static> {
    let mut w = TestWorld::with_config(&[&Demo], WorldConfig::with_seed(7));
    let centre = PLAZA.centre();
    for n in 1..=RESIDENTS {
        let id = w.spawn(&format!("R{n}"));
        assert_eq!(id, r(n));
        w.set_pos(id, centre);
    }
    // Residents 15 to 24 stand elsewhere: near the borders, across them,
    // and at the edge of sight of the plaza.
    let places = [
        (15, Pos::new(60, 30)),
        (16, Pos::new(64, 31)),
        (17, Pos::new(70, 40)),
        (18, Pos::new(62, 66)),
        (19, Pos::new(66, 66)),
        (20, Pos::new(100, 100)),
        (21, Pos::new(30, 100)),
        (22, Pos::new(33, 33)),
        (23, Pos::new(35, 30)),
        (24, Pos::new(40, 44)),
    ];
    for (n, pos) in places {
        w.set_pos(r(n), pos);
    }
    assert_eq!(ON_THE_CENTRE + places.len() as u64, RESIDENTS);
    w.step(2);

    // Work, with and without a key. Some shifts end before the snapshot.
    // Residents 17 and 22 to 24 stay idle for what follows.
    for n in (1..=21).filter(|&n| n != 17) {
        let ticks = 3 + (n % 5) * 9;
        if n % 3 == 0 {
            act_with_id(&mut w, r(n), &format!("shift-{n}"), "work", json!({ "ticks": ticks }));
        } else {
            w.act_ok(r(n), "work", json!({ "ticks": ticks }));
        }
    }
    // Planned actions behind the running one. The last one will fail.
    for ticks in [2, 4, 6] {
        w.act_ok(r(2), "nap", json!({ "ticks": ticks }));
    }
    w.act_ok(r(4), "pay", json!({ "to": 5, "amount": 999_999 }));
    // Speech. `shout` is free, at most 4 in one tick: resident 22 next to
    // the plaza fills the inboxes of everybody on it.
    for round in 0..10 {
        for part in 0..4 {
            let text = format!("round {round}, part {part}: fresh bread");
            w.act_ok(r(22), "shout", json!({ "text": text }));
        }
        // Across the border between zone 0 and zone 1.
        w.act_ok(r(15), "shout", json!({ "text": format!("east gate, round {round}") }));
        w.act_ok(r(16), "shout", json!({ "text": format!("west gate, round {round}") }));
        w.step(1);
    }
    w.act_ok(r(1), "whisper", json!({ "to": [3, 17, 20], "text": "meet me at noon" }));
    // Money and goods: gifts on the plaza and to another zone, the bank.
    w.give_item(r(4), "bread", 3);
    w.give_item(r(24), "bread", 2);
    w.give_item(r(18), "coal", 2);
    for (from, to, amount) in [(23, 1, 150), (23, 20, 75), (24, 22, 20)] {
        w.act_ok(r(from), "pay", json!({ "to": to, "amount": amount }));
    }
    w.act_ok(r(24), "eat", json!({}));
    w.step(1);
    w.act_ok(r(23), "deposit", json!({ "amount": 300 }));
    w.act_ok(r(23), "ping", json!({ "n": 5 }));
    w.step(2);
    // Routines.
    let routine = json!({
        "name": "rest",
        "when": { "type": "attr_below", "attr": "energy", "value": 40 },
        "then": { "verb": "nap", "args": { "ticks": 4 } },
    });
    for n in [5, 20, 21] {
        w.act_ok(r(n), "set_routines", json!({ "routines": [routine] }));
    }
    w.step(6);
    // Resident 18 leaves the world; resident 19 across the border inherits
    // its money and its coal.
    w.act_ok(r(18), "cancel", json!({}));
    w.act_ok(r(18), "leave", json!({ "heir": 19 }));
    w.step(3);
    // A condition with an end and an action that waits.
    w.act_ok(r(17), "jail_me", json!({}));
    w.act_ok(r(24), "offer", json!({ "ticks": 40 }));
    // On the way at the snapshot: a wire to another zone, a deposit and a
    // question to the global realm.
    w.act_ok(r(23), "pay", json!({ "to": 21, "amount": 40 }));
    w.act_ok(r(22), "deposit", json!({ "amount": 25 }));
    act_with_id(&mut w, r(22), "ask-1", "ping", json!({ "n": 9 }));
    w
}

/// What happens after the snapshot: frames, more speech and money, the
/// answer to the action that waits, a walk and a journey to another zone
/// with a full inbox, and a newcomer.
fn go_on(w: &mut TestWorld<'_>) {
    w.step(3);
    w.act_ok(r(22), "shout", json!({ "text": "the market closes soon" }));
    w.act_ok(r(1), "pay", json!({ "to": 16, "amount": 60 }));
    w.act_ok(r(19), "withdraw", json!({ "amount": 100 }));
    w.act_ok(r(23), "pay_tax", json!({ "amount": 110 }));
    let offer = w.observe(r(24)).sections["self"]["activity"]["action_id"].clone();
    w.act_ok(r(1), "accept", json!({ "resident": 24, "action_id": offer }));
    w.act_ok(r(15), "cancel", json!({}));
    for _ in 0..6 {
        w.act_ok(r(15), "step", json!({ "dx": 1, "dy": 0 }));
        w.step(1);
    }
    w.act_ok(r(3), "cancel", json!({}));
    w.act_ok(r(3), "jump", json!({ "zone": 3, "x": 90, "y": 90 }));
    let late = w.spawn_at("Late", Pos::new(34, 34));
    w.act_ok(late, "shout", json!({ "text": "hello, I am new here" }));
    w.step(40);
}

/// Returns what every resident observes, one JSON line per resident, in id
/// order. Every fourth resident observes with full detail and reads its
/// whole inbox, and so do the first two. The others read their last 6
/// entries. The last line has the totals of the world.
fn observed(w: &TestWorld<'_>) -> Vec<String> {
    let mut lines = Vec::new();
    let directory = w.global().directory();
    for (id, entry) in directory.iter() {
        if !entry.alive {
            lines.push(json!({ "resident": id, "name": entry.name, "alive": false }).to_string());
            continue;
        }
        let opts = if id.0 % 4 == 0 {
            ObserveOpts { detail: Detail::Full, since_notice: 0, sections: None }
        } else if id.0 <= 2 {
            ObserveOpts::default()
        } else {
            let next = w.world.next_notice(id).expect("the resident lives");
            ObserveOpts { since_notice: next.saturating_sub(6), ..ObserveOpts::default() }
        };
        let observation = w.observe_with(id, &opts);
        lines.push(serde_json::to_string(&observation).expect("an observation is JSON"));
    }
    let totals = w.assert_conserved();
    let zone_ticks = [0, 1, 2, 3].map(|z| w.zone(ZoneId(z)).tick());
    let global_tick = w.global().tick();
    lines.push(
        json!({
            "frame": w.world.frame(),
            "residents": w.world.residents(),
            "money_held": totals.money.held,
            "money_minted": totals.money.minted,
            "money_burned": totals.money.burned,
            "money_in_flight": totals.money.in_flight,
            "items": totals.items.iter().map(|(item, t)| (item.0, t.held, t.produced, t.consumed, t.in_flight)).collect::<Vec<_>>(),
            "bank": w.global().ledger().money(common::BANK),
            "pings": w.global().res::<common::GlobalLog>().pings,
            "zone_ticks": zone_ticks,
            "global_tick": global_tick,
        })
        .to_string(),
    );
    lines
}

/// Splits the file of observations into its two parts: at the snapshot,
/// and after [`go_on`].
fn expected() -> (Vec<&'static str>, Vec<&'static str>) {
    let lines: Vec<&str> = OBSERVED.lines().collect();
    let cut = lines.iter().position(|line| *line == "--").expect("the file has two parts");
    (lines[..cut].to_vec(), lines[cut + 1..].to_vec())
}

/// Checks that the world is observed as the older kernel observed it.
fn assert_observed(w: &TestWorld<'_>, expected: &[&str], when: &str) {
    let got = observed(w);
    assert_eq!(got.len(), expected.len(), "{when}: the number of residents");
    for (got, expected) in got.iter().zip(expected) {
        let got: Value = serde_json::from_str(got).expect("JSON");
        let expected: Value = serde_json::from_str(expected).expect("JSON");
        assert_eq!(got, expected, "{when}: resident {}", expected["resident"]);
    }
}

#[test]
fn a_snapshot_of_an_older_kernel_restores_and_the_world_goes_on_as_before() {
    let (at_snapshot, later) = expected();
    assert_eq!((at_snapshot.len(), later.len()), (25, 26), "24 residents, then 25, and the totals");
    let mut w = TestWorld::restore(&[&Demo], SNAPSHOT);
    assert_eq!((w.world.frame(), w.world.residents()), (24, 23));
    assert_observed(&w, &at_snapshot, "at the snapshot");

    // What the older kernel wrote as values, this kernel holds as bytes.
    // The snapshot of this kernel is another one, and restores to the same
    // world.
    let again = w.world.snapshot();
    assert_ne!(again.as_slice(), SNAPSHOT);
    w.snapshot_restore();
    assert_observed(&w, &at_snapshot, "after a snapshot of this kernel");

    // The older kernel remembered the answer to the request `ask-1` as
    // values; this kernel holds it as bytes. A repeat of the key gets the
    // first answer, the one that this kernel gives in its own town, and
    // starts nothing.
    let repeat = ActionRequest::new("ping", json!({ "n": 9 })).with_id("ask-1");
    let before = w.hash();
    let first = w.act_request(r(22), repeat.clone());
    assert_eq!((first.verb.as_str(), &first.status), ("ping", &ActionStatus::Pending));
    assert_eq!(first, town().act_request(r(22), repeat));
    assert_eq!(w.hash(), before);

    go_on(&mut w);
    assert_observed(&w, &later, "after the snapshot");
    w.assert_conserved();

    // A world that goes on from the snapshot of this kernel ends in the
    // same state.
    let mut second = TestWorld::restore(&[&Demo], &again);
    go_on(&mut second);
    assert_eq!(second.hash(), w.hash());
}

/// The older kernel and this kernel get the same inputs and show the same
/// town: the nearby lists, the notices, the reports and the totals. The
/// state hashes are not compared: a world that is restored from the old
/// snapshot keeps the parameter ids of that snapshot.
#[test]
fn this_kernel_builds_the_town_of_the_older_kernel() {
    let (at_snapshot, later) = expected();
    let mut w = town();
    assert_observed(&w, &at_snapshot, "at the snapshot");
    go_on(&mut w);
    assert_observed(&w, &later, "after the snapshot");
}

/// Writes the file of observations again, from the old snapshot. The
/// snapshot itself cannot be made again: only the older kernel wrote that
/// form.
///
/// Run it only after a deliberate change of what an observation shows,
/// and read the difference of the file before you keep it.
#[test]
#[ignore = "rewrites the expected observations"]
fn rewrite_the_observations() {
    let mut w = TestWorld::restore(&[&Demo], SNAPSHOT);
    let mut lines = observed(&w);
    lines.push("--".to_string());
    go_on(&mut w);
    lines.extend(observed(&w));
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/before_compact_inbox.jsonl");
    std::fs::write(path, lines.join("\n") + "\n").expect("the file can be written");
}
