//! Measures the memory of residents whose inboxes are full.
//!
//! Builds a world with N residents (first argument, default 100,000) in
//! zones of about 2,000 residents. A frame runs after every 1,000 spawns, as
//! in a server. Then every resident gets a routine that works a shift of
//! one tick again and again, and every tick gives each resident the typical
//! news of a busy town:
//!
//! - one speech notice with 80 bytes of text (`speech`), and
//! - one action report with a small result (`reports`).
//!
//! The second argument chooses the news: `mixed` (both, the default),
//! `speech`, `reports` or `none`. The example steps frames until every
//! inbox holds `kernel.inbox_max` entries, and prints the bytes per
//! resident two ways: the estimate of the kernel, and the growth of the
//! resident set size of the process, as the operating system reports it.
//!
//! The news `keys` measures something else that a resident keeps: the
//! answers to its last 32 requests with an idempotency key. Every resident
//! sends 32 requests with a key of 36 bytes, as the client libraries do,
//! for an action that ends at once with a small result.
//! Then it steps 64 more frames (third argument), so that every entry is
//! replaced a few times, and prints the numbers again: a world that has
//! run for a while.
//!
//! A speech notice is written for one resident, so no two inboxes share a
//! body. Speech that a crowd hears is shared and costs less.
//!
//! ```text
//! cargo run --release -p unsup-kernel --example bench_memory -- 100000
//! cargo run --release -p unsup-kernel --example bench_memory -- 100000 speech
//! ```

use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::protocol::{ActionRequest, Condition, ObserveOpts, Routine, SpawnSpec};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{
    Begin, Builder, CityConfig, CityId, Holder, Money, NoArgs, OwnerId, Plugin, Pos, RealmId,
    ResidentId, Resource, Scope, Stage, Untrusted, VerbSpec, World, WorldConfig,
};

/// Residents per zone that the benchmark aims for.
const PER_ZONE: usize = 2_000;

/// Bytes of text in one speech notice.
const SPEECH_BYTES: usize = 80;

/// Residents that join between two frames while the world is built.
const SPAWNS_PER_FRAME: u64 = 1_000;

/// Requests with an idempotency key that each resident sends for the news
/// `keys`: as many as the kernel remembers.
const KEYED_REQUESTS: u32 = 32;

/// The frame at which the measurement starts. See `main`.
const OLD_WORLD_FRAME: u64 = 130;

/// Frames that run after the inboxes are full, to show a world that has run
/// for a while.
const CHURN_FRAMES: usize = 64;

/// The data override of the benchmark: a routine action writes a report
/// like an action that an agent sent.
const DATA: &str = r#"
[params."kernel.routine_reports"]
value = 1
"#;

/// What the benchmark module does in each tick.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Switches {
    /// Every resident gets a speech notice.
    speech: bool,
}

impl Resource for Switches {
    const NAME: &'static str = "bench.switches";
}

/// Arguments of `work`.
#[derive(Serialize, Deserialize, JsonSchema)]
struct WorkArgs {
    /// Duration in ticks.
    ticks: u32,
}

struct Town;

impl Plugin for Town {
    fn name(&self) -> &'static str {
        "town"
    }

    fn build(&self, b: &mut Builder) {
        let wages = b.faucet("bench.wages");
        if b.scope() == Scope::Zone {
            b.resource::<Switches>();
            // Spread new residents over all zones.
            b.on_spawn(0, move |realm, idx| {
                let id = realm.id_of(idx);
                let zones = realm.layout().zones();
                let zone = &zones[(id.0 % zones.len() as u64) as usize];
                let (x, y) = {
                    let mut rng = realm.rng();
                    (rng.range(0, 64) as i32, rng.range(0, 64) as i32)
                };
                let pos = Pos::new(zone.bounds.x0 + x, zone.bounds.y0 + y);
                realm.teleport(idx, zone.id, pos).expect("the position is inside the zone");
            });
            // The speech: one notice per resident per tick, from the
            // resident in the next row.
            b.system(Stage::Update, 0, "bench.speech", |realm| {
                if !realm.res::<Switches>().speech {
                    return;
                }
                let ids: Vec<ResidentId> = realm.ids().to_vec();
                let tick = realm.tick().0;
                for (i, &to) in ids.iter().enumerate() {
                    let from = ids[(i + 1) % ids.len()];
                    let mut text = format!("tick {tick}: resident {from} tells resident {to} ");
                    while text.len() < SPEECH_BYTES {
                        text.push_str("the news of the town ");
                    }
                    text.truncate(SPEECH_BYTES);
                    realm.notify(
                        to,
                        "society.speech",
                        Some(from),
                        json!({ "text": Untrusted::new(from, text).to_value() }),
                    );
                }
            });
        }
        b.instant_verb::<NoArgs>(
            VerbSpec {
                verb: "greet",
                module: "town",
                summary: "Greet the town.",
                free: false,
                allowed_in: &[],
            },
            |realm, ctx, _args| Ok(json!({ "greeted": realm.len(), "by": ctx.resident })),
        );
        b.verb::<WorkArgs>(
            VerbSpec {
                verb: "work",
                module: "town",
                summary: "Work a shift.",
                free: false,
                allowed_in: &[],
            },
            |_realm, _ctx, args| Ok(Begin::Busy { ticks: args.ticks }),
            move |realm, ctx, args| {
                let pay = i64::from(args.ticks) * 10;
                realm.ledger_mut().mint(wages, Holder::Resident(ctx.resident), Money(pay));
                Ok(json!({ "pay": pay }))
            },
        );
    }
}

/// Returns the resident set size of this process in bytes, as `ps` reports
/// it.
fn resident_set_size() -> usize {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .expect("`ps` runs");
    let text = String::from_utf8_lossy(&output.stdout);
    let kilobytes: usize = text.trim().parse().expect("`ps` prints a number");
    kilobytes * 1024
}

/// Returns the number of entries in the inbox of a resident.
fn inbox_len(world: &World, resident: ResidentId) -> usize {
    let observation = world.observe(resident, &ObserveOpts::default()).expect("the resident lives");
    let reports = observation.sections["reports"].as_array().map_or(0, Vec::len);
    observation.notices.len() + reports
}

/// Prints the memory per resident: the estimate, and the growth of the
/// resident set size since `start`.
fn print_memory(label: &str, world: &World, residents: usize, start: usize, base: usize) {
    let now = resident_set_size();
    let estimate = world.memory_estimate();
    println!(
        "{label}: estimate {} bytes per resident; resident set size {} bytes per resident ({} more than idle), {:.1} MiB for the process",
        estimate.per_resident(),
        now.saturating_sub(start) / residents,
        now.saturating_sub(base) / residents,
        now as f64 / (1024.0 * 1024.0)
    );
}

fn usage() -> ! {
    eprintln!(
        "usage: bench_memory [RESIDENTS] [mixed|speech|reports|none|keys] [FRAMES]   (defaults: 100000, mixed and 64)"
    );
    std::process::exit(2);
}

fn main() {
    let residents = match std::env::args().nth(1) {
        Some(arg) => arg.replace('_', "").parse::<usize>().unwrap_or_else(|_| usage()).max(1),
        None => 100_000,
    };
    let (speech, reports, keys) = match std::env::args().nth(2).as_deref() {
        None | Some("mixed") => (true, true, false),
        Some("speech") => (true, false, false),
        Some("reports") => (false, true, false),
        Some("none") => (false, false, false),
        Some("keys") => (false, false, true),
        Some(_) => usage(),
    };
    let start = resident_set_size();

    let districts = (residents.div_ceil(PER_ZONE) as f64).sqrt().ceil().max(1.0) as u32;
    let config = WorldConfig {
        seed: 42,
        cities: vec![CityConfig::new("Bench", Pos::new(0, 0), districts)],
        data: vec![DATA.to_string()],
        ..WorldConfig::default()
    };
    let mut world = World::new(&[&Town], config);
    let mut ids = Vec::with_capacity(residents);
    for i in 0..residents as u64 {
        let spec = SpawnSpec {
            owner: OwnerId(1 + i % 1_000),
            name: format!("r{i}"),
            model: None,
            city: Some(CityId(0)),
        };
        ids.push(world.spawn(spec).expect("spawn"));
        // A server steps frames while residents join. Each frame hands the
        // events and the deltas of the spawns to the viewers and frees
        // them, so they do not count as memory of the residents.
        if (i + 1) % SPAWNS_PER_FRAME == 0 {
            world.step();
        }
    }
    // An entry holds the tick at which it was written, and a tick below 128
    // takes one byte less than a later one. The world runs past that tick
    // first, so that every entry of the measurement has the length that
    // entries have in a world that is older than two game hours.
    while world.frame() < OLD_WORLD_FRAME {
        world.step();
    }
    let zones = world.layout().zones().len();
    let inbox_max = world.blueprint().param_id("kernel.inbox_max").expect("a kernel parameter");
    let inbox_max = world.realm(RealmId::Global).map_or(0, |realm| realm.param(inbox_max)) as usize;
    println!(
        "world: {residents} residents in {zones} zones ({} per zone), inbox_max {inbox_max}, news per tick: {}{}",
        residents / zones,
        if speech { "one speech notice " } else { "" },
        if reports { "one action report" } else { "" },
    );
    let base = resident_set_size();
    print_memory("idle residents, empty inboxes", &world, residents, start, base);

    if keys {
        for round in 0..KEYED_REQUESTS {
            for &id in &ids {
                // The shape of a UUID: 36 bytes.
                let key = format!("{:08x}-0000-4000-8000-{:012x}", round, id.0);
                let ack = world.act(id, ActionRequest::new("greet", json!({})).with_id(key));
                assert!(ack.expect("act").status.is_accepted());
            }
        }
        print_memory(
            "the answers to 32 requests with a key each, empty inboxes",
            &world,
            residents,
            start,
            base,
        );
        return;
    }
    if reports {
        for &id in &ids {
            let routine = Routine {
                name: "work".into(),
                when: Condition::Always,
                then: ActionRequest::new("work", json!({ "ticks": 1 })),
            };
            world
                .act(id, ActionRequest::new("set_routines", json!({ "routines": [routine] })))
                .expect("act");
        }
        // The routines fire at the end of this frame. The first reports
        // come in the next one.
        world.step();
        print_memory(
            "one routine and one running action each, empty inboxes",
            &world,
            residents,
            start,
            base,
        );
    }
    for zone in world.layout().zones() {
        if let Some(realm) = world.realm(RealmId::Zone(zone.id)) {
            realm.res_mut::<Switches>().speech = speech;
        }
    }
    let per_tick = usize::from(speech) + usize::from(reports);
    if per_tick == 0 {
        return;
    }
    let fill = inbox_max.div_ceil(per_tick);
    for _ in 0..fill {
        world.step();
    }
    let sample = ResidentId(1 + residents as u64 / 2);
    println!(
        "after {fill} frames the inbox of resident {sample} holds {} entries",
        inbox_len(&world, sample)
    );
    print_memory("full inboxes", &world, residents, start, base);

    let churn = match std::env::args().nth(3) {
        Some(arg) => arg.parse::<usize>().unwrap_or_else(|_| usage()),
        None => CHURN_FRAMES,
    };
    for _ in 0..churn {
        world.step();
    }
    println!(
        "after {churn} more frames the inbox of resident {sample} holds {} entries",
        inbox_len(&world, sample)
    );
    print_memory("full inboxes, every entry replaced", &world, residents, start, base);
    world.audit().expect("the ledgers are balanced");
}
