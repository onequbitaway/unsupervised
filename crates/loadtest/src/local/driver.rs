//! The driver: makes every resident of a world in this process act by a
//! behaviour profile.
//!
//! The driver keeps a turn for each resident. At its turn a resident
//! chooses an action from the profile and the driver gives it to
//! `World::apply`, like the server does with a request. The next turn of
//! the resident comes when the action has ended and the think time has
//! passed.
//!
//! The driver is deterministic: the same world, profile and seed give the
//! same requests in the same order. The soak test needs that.

use std::collections::BTreeMap;

use unsup_kernel::{Input, Output, Rng, World};
use unsup_protocol::{ActionRequest, ActionStatus, ObserveOpts, RealmId, ResidentId, SimError};

use super::ActionCounts;
use super::scene::{LocalScene, Scratch};
use crate::defaults::defaults;
use crate::profile::Resolved;
use crate::random;
use crate::stats;

/// Stream of the generator that chooses actions.
const DRIVER_STREAM: u64 = 0xd21_7e12;

/// Options of a driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DriverOptions {
    /// Seed of the choices of the residents.
    pub seed: u64,
    /// True: every request carries an idempotency key, as a careful client
    /// sends it. The kernel then remembers the last keys of each resident.
    pub request_ids: bool,
}

/// Makes the residents of one world act.
pub struct Driver {
    ids: Vec<ResidentId>,
    profile: Resolved,
    rng: Rng,
    /// Residents by the frame of their next turn. The value is the place of
    /// the resident in `ids`.
    turns: BTreeMap<u64, Vec<u32>>,
    planned: Vec<(u32, ActionRequest)>,
    acted: Vec<u32>,
    counts: ActionCounts,
    request_ids: bool,
    requests: u64,
    scratch: Scratch,
    pending_recheck: u64,
    /// The notice cursor of each resident: the `next_notice` of its last
    /// observation. A client sends it back, so that an observation holds
    /// only what is new.
    cursors: Vec<u64>,
}

impl Driver {
    /// Makes a driver for the residents `ids` of `world`. The first turn of
    /// each resident comes after one think time, so the residents do not
    /// all start in the same frame.
    #[must_use]
    pub fn new(
        world: &World,
        ids: Vec<ResidentId>,
        profile: Resolved,
        options: DriverOptions,
    ) -> Driver {
        let mut driver = Driver {
            cursors: vec![0; ids.len()],
            ids,
            profile,
            rng: Rng::stream(options.seed, DRIVER_STREAM),
            turns: BTreeMap::new(),
            planned: Vec::new(),
            acted: Vec::new(),
            counts: ActionCounts::default(),
            request_ids: options.request_ids,
            requests: 0,
            scratch: Scratch::default(),
            pending_recheck: u64::from(defaults().driver.pending_recheck_ticks.max(1)),
        };
        let now = world.frame();
        for slot in 0..driver.ids.len() {
            let think = driver.think();
            driver.turn_at(slot as u32, now + think);
        }
        driver
    }

    /// Returns the residents that the driver moves, in ascending id order.
    #[must_use]
    pub fn residents(&self) -> &[ResidentId] {
        &self.ids
    }

    /// Returns the profile that the residents follow.
    #[must_use]
    pub fn profile(&self) -> &Resolved {
        &self.profile
    }

    /// Returns the counts since the last call of [`Driver::take_counts`].
    #[must_use]
    pub fn counts(&self) -> &ActionCounts {
        &self.counts
    }

    /// Returns the counts and starts again from zero.
    pub fn take_counts(&mut self) -> ActionCounts {
        std::mem::take(&mut self.counts)
    }

    fn think(&mut self) -> u64 {
        u64::from(self.profile.think.sample(&mut self.rng))
    }

    fn turn_at(&mut self, slot: u32, frame: u64) {
        self.turns.entry(frame).or_default().push(slot);
    }

    /// Lets every resident whose turn has come choose an action. Nothing in
    /// the world changes. Returns the number of requests that are ready for
    /// [`Driver::apply`].
    pub fn plan(&mut self, world: &World) -> usize {
        let now = world.frame();
        while let Some(entry) = self.turns.first_entry() {
            if *entry.key() > now {
                break;
            }
            for slot in entry.remove() {
                self.plan_one(world, slot, now);
            }
        }
        self.planned.len()
    }

    fn plan_one(&mut self, world: &World, slot: u32, now: u64) {
        let id = self.ids[slot as usize];
        let place = world.locate(id).and_then(|zone| {
            let realm = world.realm(RealmId::Zone(zone))?;
            Some((realm, realm.idx_of(id)?))
        });
        let Some((realm, idx)) = place else {
            // The resident left the world (it died or was removed). It gets
            // no more turns.
            self.counts.gone += 1;
            return;
        };
        if let Some(activity) = realm.activity(idx) {
            // Something else keeps the resident busy: a routine, or an
            // action that waits for an answer. Look again when it ends.
            let left = if activity.waiting {
                self.pending_recheck
            } else {
                activity.ends.since(realm.tick()).max(1)
            };
            self.counts.busy_turns += 1;
            let think = self.think();
            self.turn_at(slot, now + left + think);
            return;
        }
        let mut scene = LocalScene::at(world, realm, idx, &mut self.scratch);
        match self.profile.pick(&mut self.rng, &mut scene) {
            Some(choice) => {
                let id = self.request_ids.then(|| {
                    self.requests += 1;
                    format!("lt-{}", self.requests)
                });
                self.planned
                    .push((slot, ActionRequest { id, verb: choice.verb, args: choice.args }));
            }
            None => {
                self.counts.idle_turns += 1;
                let think = self.think().max(1);
                self.turn_at(slot, now + think);
            }
        }
    }

    /// Gives the planned requests to `World::apply`, in the order in which
    /// they were planned, and sets the next turn of each resident. Returns
    /// the number of requests.
    pub fn apply(&mut self, world: &mut World) -> usize {
        let now = world.frame();
        let planned = std::mem::take(&mut self.planned);
        let count = planned.len();
        for (slot, request) in planned {
            let resident = self.ids[slot as usize];
            let think = self.think();
            match world.apply(Input::Act { resident, request }) {
                Output::Ack(ack) => {
                    let wait = match &ack.status {
                        // The result is there. A think time of zero must not
                        // give a second turn in the same frame.
                        ActionStatus::Done { .. } | ActionStatus::Rejected { .. } => think.max(1),
                        ActionStatus::Started { ticks, .. } => u64::from(*ticks) + think,
                        ActionStatus::Pending => self.pending_recheck + think,
                        // The resident was busy after all. Its turn finds
                        // out when the running action ends.
                        ActionStatus::Queued { .. } => 1 + think,
                    };
                    self.counts.record(&ack);
                    self.turn_at(slot, now + wait);
                    self.acted.push(slot);
                }
                Output::Error(error) => {
                    self.counts.sent += 1;
                    stats::bump(&mut self.counts.refused_by_sim, error.as_str(), 1);
                    if error == SimError::UnknownResident {
                        self.counts.gone += 1;
                    } else {
                        self.turn_at(slot, now + think.max(1));
                    }
                }
                // `Input::Act` has no other answer.
                _ => {}
            }
        }
        count
    }

    /// Lets a share of the residents that just acted observe, as a client
    /// does after an action. The share is `observe_share` of the profile.
    /// Each resident sends the notice cursor of its last observation, so it
    /// gets only the notices and reports that are new. Returns the number
    /// of observations.
    pub fn observe(&mut self, world: &World) -> usize {
        let share = self.profile.observe_share;
        let mut opts = ObserveOpts::default();
        let mut made = 0;
        for slot in std::mem::take(&mut self.acted) {
            if !random::chance(&mut self.rng, share) {
                continue;
            }
            opts.since_notice = self.cursors[slot as usize];
            // A resident that is gone gives an error. That is counted at
            // its next turn.
            if let Ok(observation) = world.observe(self.ids[slot as usize], &opts) {
                self.cursors[slot as usize] = observation.next_notice;
                // The observation is built and dropped: its cost is what
                // counts here.
                std::hint::black_box(&observation);
                made += 1;
            }
        }
        self.counts.observes += made as u64;
        made
    }

    /// Does one whole turn before a frame: plan, apply and observe. Returns
    /// the number of requests that were applied.
    pub fn turn(&mut self, world: &mut World) -> usize {
        self.plan(world);
        let applied = self.apply(world);
        self.observe(world);
        applied
    }
}
