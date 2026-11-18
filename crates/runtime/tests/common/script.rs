//! A fixed script for a world with the town module and the roads module.
//!
//! The same seed gives the same inputs in every run, for one process and
//! for a cluster: what the script does next depends only on its seed and
//! on what the residents observe.
//!
//! The script stays inside what the kernel can do in a cluster today (see
//! "Limits" in `README.md`):
//!
//! - A message to a living resident is sent only from a zone in which
//!   that resident is or has been. The world of another zone has never
//!   seen the resident and treats it as gone.
//! - The global realm talks only to residents that have changed zone at
//!   least once. So a resident uses a verb of the global realm, and gets a
//!   decree, only after it has left the zone in which it was made.
//! - A zone sends no module message to itself.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use unsup_kernel::WorldConfig;
use unsup_protocol::{
    AdminCommand, Detail, ObserveOpts, ResidentId, SimError, ViewQuery, WorldView, ZoneId,
};
use unsup_runtime::SimHandle;

use super::{Decree, act, spec};

/// A small random number generator with a fixed sequence per seed.
#[derive(Clone, Debug)]
pub struct Dice(u64);

impl Dice {
    /// Makes a generator.
    pub fn new(seed: u64) -> Dice {
        Dice(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1)
    }

    /// Returns the next number.
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// Returns a number below `limit`.
    pub fn below(&mut self, limit: u64) -> u64 {
        (self.next() >> 16) % limit.max(1)
    }
}

/// What the script knows about one resident, from its observations.
#[derive(Clone, Debug)]
struct Known {
    id: ResidentId,
    alive: bool,
    zone: ZoneId,
    /// Every zone in which the resident was seen.
    visited: BTreeSet<ZoneId>,
    /// The resident does nothing and has nothing planned.
    idle: bool,
}

/// The script and what it has seen.
#[derive(Clone)]
pub struct Script {
    dice: Dice,
    residents: Vec<Known>,
    /// Tiles per side of the world.
    side: u64,
    /// Number of zones.
    zones: u64,
    /// What every input answered and what every resident observed, frame
    /// by frame. Two runs of the same world give the same text.
    pub record: Vec<String>,
    /// The text of the current frame.
    page: String,
}

impl Script {
    /// Makes the script for a world of one city.
    pub fn new(seed: u64, world: &WorldConfig) -> Script {
        let districts = u64::from(world.cities[0].districts);
        Script {
            dice: Dice::new(seed),
            residents: Vec::new(),
            side: districts * u64::from(world.zone_size),
            zones: districts * districts,
            record: Vec::new(),
            page: String::new(),
        }
    }

    fn note(&mut self, what: &str, value: &impl std::fmt::Debug) {
        self.page.push_str(&format!("{what}: {value:?}\n"));
    }

    /// Reads where a resident is and what it does.
    async fn refresh(&mut self, handle: &SimHandle, at: usize) {
        let id = self.residents[at].id;
        let opts = ObserveOpts { detail: Detail::Full, ..ObserveOpts::default() };
        match handle.observe(id, opts).await {
            Ok(observation) => {
                let me = &observation.sections["self"];
                let zone = ZoneId(me["zone"].as_u64().expect("a zone") as u32);
                let idle = me["activity"].is_null() && me["queue"] == json!(0);
                let known = &mut self.residents[at];
                known.zone = zone;
                known.visited.insert(zone);
                known.idle = idle;
                let text = serde_json::to_string(&observation).expect("an observation is JSON");
                self.page.push_str(&text);
                self.page.push('\n');
            }
            Err(SimError::UnknownResident) => {
                self.residents[at].alive = false;
                self.note("gone", &id);
            }
            Err(error) => panic!("the runtime did not answer an observation: {error}"),
        }
    }

    /// Creates `count` residents. They all appear in the same zone.
    pub async fn spawn(&mut self, handle: &SimHandle, count: usize) {
        for n in 0..count {
            let id = handle.spawn(spec(&format!("R{n}"))).await.expect("spawn");
            self.residents.push(Known {
                id,
                alive: true,
                zone: ZoneId(0),
                visited: BTreeSet::new(),
                idle: true,
            });
            let at = self.residents.len() - 1;
            self.refresh(handle, at).await;
        }
    }

    /// Returns a resident that a resident in `zone` can send something to:
    /// one that is dead, or one that is in the zone or has been in it.
    fn target(&mut self, from: usize) -> Option<ResidentId> {
        let zone = self.residents[from].zone;
        let known: Vec<ResidentId> = self
            .residents
            .iter()
            .enumerate()
            .filter(|(at, other)| {
                *at != from && (!other.alive || other.zone == zone || other.visited.contains(&zone))
            })
            .map(|(_, other)| other.id)
            .collect();
        if known.is_empty() {
            return None;
        }
        Some(known[self.dice.below(known.len() as u64) as usize])
    }

    /// Returns true when every other living resident is in the zone of
    /// `from` or has been in it. A shout can then reach any of them.
    fn all_known(&self, from: usize) -> bool {
        let zone = self.residents[from].zone;
        self.residents.iter().enumerate().all(|(at, other)| {
            at == from || !other.alive || other.zone == zone || other.visited.contains(&zone)
        })
    }

    /// Chooses what a resident does in this round.
    fn choose(&mut self, at: usize) -> Option<(&'static str, Value)> {
        let me = self.residents[at].clone();
        let roll = self.dice.below(100);
        let tile = json!({ "x": self.dice.below(self.side), "y": self.dice.below(self.side) });
        let zone = json!({ "zone": self.dice.below(self.zones) });
        let amount = 1 + self.dice.below(9);
        // A resident that never left its first zone: the global realm and
        // the other zones do not know it yet.
        let fresh = me.visited.len() <= 1;
        let request = match roll {
            0..=27 => ("walk", tile),
            28..=35 => ("travel", zone),
            36..=41 => ("chop", json!({})),
            42..=47 => ("work", json!({ "ticks": 1 + amount % 3 })),
            48..=55 => ("deposit", json!({ "amount": amount })),
            56..=62 if !fresh => ("withdraw", json!({ "amount": 1 + amount % 4 })),
            63..=70 => ("tell", json!({ "to": self.target(at)?, "kind": "town.greeting" })),
            // `pay` takes time, so a busy resident would do it later, in
            // a zone that the script cannot know now.
            71..=78 if me.idle => ("pay", json!({ "to": self.target(at)?, "amount": amount })),
            79..=84 => ("poke", json!({ "to": self.target(at)?, "n": amount })),
            85..=89 if self.all_known(at) => ("shout", json!({})),
            // A call and a signal are sent in a tick, some frames later.
            // The resident stays in its zone until then.
            90..=95 if me.idle => {
                ("call", json!({ "to": self.target(at)?, "ticks": 1 + amount % 3 }))
            }
            96..=99 if me.idle => {
                // Another zone than its own: the world of a zone gives a
                // message of the zone to itself out of turn.
                let other = 1 + self.dice.below(self.zones - 1);
                let to = (u64::from(me.zone.0) + other) % self.zones;
                ("signal", json!({ "zone": to, "n": amount, "ticks": 1 + amount % 2 }))
            }
            _ => return None,
        };
        Some(request)
    }

    /// Plays one round: every living resident can act, then the special
    /// inputs of the round, then one frame.
    pub async fn round(&mut self, handle: &SimHandle, round: u64) {
        for at in 0..self.residents.len() {
            if !self.residents[at].alive {
                continue;
            }
            // The choice needs the zone and the activity as they are now.
            self.refresh(handle, at).await;
            if !self.residents[at].alive {
                continue;
            }
            let Some((verb, args)) = self.choose(at) else {
                continue;
            };
            let id = self.residents[at].id;
            let answer = handle.act(id, act(verb, args.clone())).await;
            self.note(&format!("act {id} {verb} {args}"), &answer);
            // `travel` moves the resident at once.
            self.refresh(handle, at).await;
        }
        self.special(handle, round).await;
        self.frame(handle).await;
    }

    /// The inputs that are not actions of a resident.
    async fn special(&mut self, handle: &SimHandle, round: u64) {
        let pick = self.dice.below(self.residents.len() as u64) as usize;
        let id = self.residents[pick].id;
        let seen_elsewhere = self.residents[pick].visited.len() > 1;
        match round % 16 {
            3 => {
                let command = AdminCommand::SetParam {
                    name: "roads.shout_radius".into(),
                    value: 4 + (round % 7) as i64,
                    city: None,
                };
                let answer = handle.admin(command).await;
                self.note("set_param", &answer);
            }
            6 if seen_elsewhere || !self.residents[pick].alive => {
                // The global realm wires a bonus to a resident.
                let decree = Decree { title: format!("Round {round}"), to: id, bonus: 25 };
                let answer = handle.send_global(&decree).await;
                self.note(&format!("decree {id}"), &answer);
            }
            9 => {
                let answer = handle.admin(AdminCommand::Freeze { resident: id }).await;
                self.note(&format!("freeze {id}"), &answer);
            }
            11 => {
                let answer = handle.admin(AdminCommand::Unfreeze { resident: id }).await;
                self.note(&format!("unfreeze {id}"), &answer);
            }
            13 if round % 32 == 13 && self.residents[pick].alive => {
                // One resident leaves the world. Its money goes to another
                // resident, or to nobody.
                self.refresh(handle, pick).await;
                if self.residents[pick].idle {
                    let heir = self.target(pick);
                    let answer = handle.act(id, act("leave", json!({ "heir": heir }))).await;
                    self.note(&format!("leave {id} heir {heir:?}"), &answer);
                } else {
                    let answer = handle.admin(AdminCommand::Despawn { resident: id }).await;
                    self.note(&format!("despawn {id}"), &answer);
                }
                self.refresh(handle, pick).await;
            }
            _ => {}
        }
    }

    /// Runs one frame and writes down what every resident sees after it.
    pub async fn frame(&mut self, handle: &SimHandle) {
        let frame = handle.step(1).await.expect("step");
        self.note("frame", &frame);
        for at in 0..self.residents.len() {
            if self.residents[at].alive {
                self.refresh(handle, at).await;
            }
        }
        match handle.view(ViewQuery::Layout).await.expect("the layout") {
            WorldView::Layout { zones, .. } => {
                let residents: Vec<u32> = zones.iter().map(|zone| zone.residents).collect();
                self.note("residents per zone", &residents);
            }
            other => panic!("not a layout: {other:?}"),
        }
        let page = std::mem::take(&mut self.page);
        self.record.push(page);
    }

    /// Runs frames until no resident does anything, then three more, so
    /// that every message has arrived. Returns the number of frames.
    pub async fn rest(&mut self, handle: &SimHandle) -> u64 {
        let mut frames = 0;
        loop {
            self.frame(handle).await;
            frames += 1;
            assert!(frames < 2_000, "the residents never come to rest");
            if self.residents.iter().all(|known| !known.alive || known.idle) {
                break;
            }
        }
        for _ in 0..3 {
            self.frame(handle).await;
        }
        frames + 3
    }

    /// Returns the residents that are alive.
    pub fn living(&self) -> Vec<ResidentId> {
        self.residents.iter().filter(|known| known.alive).map(|known| known.id).collect()
    }

    /// Returns how many residents were seen in more than one zone.
    pub fn travelled(&self) -> usize {
        self.residents.iter().filter(|known| known.visited.len() > 1).count()
    }
}
