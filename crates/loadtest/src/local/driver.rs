//! The driver: makes every resident of a world in this process act by a
//! behaviour profile.
//!
//! The driver keeps a turn for each resident. At its turn a resident
//! chooses an action from the profile and the driver gives it to
//! `World::apply`, like the server does with a request. An action that
//! works only at a site of some kind is a plan of two requests: the walk
//! to the site, then the action. The next turn of the resident comes when
//! the action has ended and the think time has passed.
//!
//! The driver is deterministic: the same world, profile and seed give the
//! same requests in the same order. The soak test needs that.

use std::collections::BTreeMap;

use unsup_kernel::{Input, Output, Rng, World};
use unsup_protocol::{
    ActionAck, ActionRequest, ActionStatus, Observation, ObserveOpts, RealmId, ResidentId, SimError,
};

use super::ActionCounts;
use super::scene::{LocalScene, Scratch};
use crate::defaults::defaults;
use crate::map::WorldMap;
use crate::profile::{Choice, Memory, Resolved};
use crate::random;
use crate::stats;

/// Stream of the generator that chooses actions.
const DRIVER_STREAM: u64 = 0xd21_7e12;

/// Options of a driver.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DriverOptions {
    /// Seed of the choices of the residents.
    pub seed: u64,
    /// True: every request carries an idempotency key, as a careful client
    /// sends it. The kernel then remembers the last keys of each resident.
    pub request_ids: bool,
    /// The sections that a resident asks for in an observation. Empty: all
    /// sections, as a client does by default. The reports of ended actions
    /// are counted only when the list is empty or has `reports`.
    pub observe_sections: Vec<String>,
    /// How the observations of one frame are built. 0: one call of
    /// `World::observe` for each observation, on one thread. N: calls of
    /// `World::observe_batch` with at most N requests each, as the runtime
    /// builds the observations of the `observe` calls of one batch of its
    /// mailbox. The answers are the same in both ways.
    pub observe_batch: usize,
}

/// What the world answered to one request.
enum Sent {
    /// The acknowledgement.
    Ack(ActionAck),
    /// The resident is no longer in the world.
    Gone,
    /// The simulation refused the request before the action engine.
    Refused,
}

/// Makes the residents of one world act.
pub struct Driver {
    ids: Vec<ResidentId>,
    profile: Resolved,
    map: WorldMap,
    rng: Rng,
    /// Residents by the frame of their next turn. The value is the place of
    /// the resident in `ids`.
    turns: BTreeMap<u64, Vec<u32>>,
    planned: Vec<(u32, Choice)>,
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
    /// What each resident remembers about its own actions.
    memory: Vec<Memory>,
    /// The sections of an observation. `None`: all sections.
    sections: Option<Vec<String>>,
    /// Greatest number of requests in one call of `World::observe_batch`.
    /// 0: one call of `World::observe` for each observation.
    observe_batch: usize,
}

impl Driver {
    /// Makes a driver for the residents `ids` of `world`. `map` is the map
    /// of that world. The first turn of each resident comes after one
    /// think time, so the residents do not all start in the same frame.
    #[must_use]
    pub fn new(
        world: &World,
        map: WorldMap,
        ids: Vec<ResidentId>,
        profile: Resolved,
        options: DriverOptions,
    ) -> Driver {
        let mut driver = Driver {
            cursors: vec![0; ids.len()],
            memory: vec![Memory::default(); ids.len()],
            ids,
            profile,
            map,
            rng: Rng::stream(options.seed, DRIVER_STREAM),
            turns: BTreeMap::new(),
            planned: Vec::new(),
            acted: Vec::new(),
            counts: ActionCounts::default(),
            request_ids: options.request_ids,
            requests: 0,
            scratch: Scratch::default(),
            pending_recheck: u64::from(defaults().driver.pending_recheck_ticks.max(1)),
            sections: Some(options.observe_sections).filter(|sections| !sections.is_empty()),
            observe_batch: options.observe_batch,
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

    /// Returns the map of the world.
    #[must_use]
    pub fn map(&self) -> &WorldMap {
        &self.map
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
    /// the world changes. Returns the number of actions that are ready for
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
            // The resident is still busy: with the rest of its plan, with a
            // routine, or with an action that waits for an answer. Look
            // again when it ends.
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
        let memory = self.memory[slot as usize];
        let mut scene = LocalScene::at(&self.map, realm, idx, &mut self.scratch);
        match self.profile.pick(&mut self.rng, &mut scene, memory) {
            Some(choice) => self.planned.push((slot, choice)),
            None => {
                self.counts.idle_turns += 1;
                let think = self.think().max(1);
                self.turn_at(slot, now + think);
            }
        }
    }

    /// Gives one request to `World::apply` and counts the answer.
    fn send(
        &mut self,
        world: &mut World,
        resident: ResidentId,
        mut request: ActionRequest,
    ) -> Sent {
        if self.request_ids {
            self.requests += 1;
            request.id = Some(format!("lt-{}", self.requests));
        }
        match world.apply(Input::Act { resident, request }) {
            Output::Ack(ack) => {
                self.counts.record(&ack);
                Sent::Ack(ack)
            }
            Output::Error(error) => {
                self.counts.sent += 1;
                stats::bump(&mut self.counts.refused_by_sim, error.as_str(), 1);
                if error == SimError::UnknownResident {
                    self.counts.gone += 1;
                    Sent::Gone
                } else {
                    Sent::Refused
                }
            }
            // `Input::Act` has no other answer.
            _ => Sent::Refused,
        }
    }

    /// Returns the frames until the next turn of a resident whose request
    /// got this acknowledgement.
    fn wait_after(&self, status: &ActionStatus, think: u64) -> u64 {
        match status {
            // The result is there. A think time of zero must not give a
            // second turn in the same frame.
            ActionStatus::Done { .. } | ActionStatus::Rejected { .. } => think.max(1),
            ActionStatus::Started { ticks, .. } => u64::from(*ticks) + think,
            ActionStatus::Pending => self.pending_recheck + think,
            // The resident was busy after all. Its turn finds out when the
            // running action ends.
            ActionStatus::Queued { .. } => 1 + think,
        }
    }

    /// Gives the planned actions to `World::apply`, in the order in which
    /// they were planned, and sets the next turn of each resident. An
    /// action with a walk is sent like a plan of a client: the walk first,
    /// and the action only when the world did not reject the walk. Returns
    /// the number of actions.
    pub fn apply(&mut self, world: &mut World) -> usize {
        let now = world.frame();
        let planned = std::mem::take(&mut self.planned);
        let count = planned.len();
        for (slot, mut choice) in planned {
            let resident = self.ids[slot as usize];
            let think = self.think();
            // The time of the walk, when the plan has one.
            let mut walk_wait = None;
            if let Some(walk) = choice.walk() {
                match self.send(world, resident, walk) {
                    Sent::Ack(ack) if matches!(ack.status, ActionStatus::Rejected { .. }) => {
                        self.counts.not_sent += 1;
                        self.turn_at(slot, now + think.max(1));
                        self.acted.push(slot);
                        continue;
                    }
                    Sent::Ack(ack) => walk_wait = Some(self.wait_after(&ack.status, think)),
                    Sent::Gone => continue,
                    Sent::Refused => {
                        self.turn_at(slot, now + think.max(1));
                        continue;
                    }
                }
            }
            let request = choice.take_request();
            match self.send(world, resident, request) {
                Sent::Ack(ack) => {
                    if !matches!(ack.status, ActionStatus::Rejected { .. }) {
                        choice.remember(&mut self.memory[slot as usize]);
                    }
                    // An action behind its walk is in the queue. The walk
                    // tells when to look again.
                    let wait = match (&ack.status, walk_wait) {
                        (ActionStatus::Queued { .. }, Some(walk)) => walk,
                        (status, _) => self.wait_after(status, think),
                    };
                    self.turn_at(slot, now + wait);
                    self.acted.push(slot);
                }
                Sent::Gone => {}
                Sent::Refused => self.turn_at(slot, now + think.max(1)),
            }
        }
        count
    }

    /// Lets a share of the residents that just acted observe, as a client
    /// does after an action. The share is `observe_share` of the profile.
    /// Each resident sends the notice cursor of its last observation, so it
    /// gets only the notices and reports that are new. The reports of
    /// ended actions in these observations are counted. Returns the number
    /// of observations.
    ///
    /// The observations are built one by one on this thread, or in calls
    /// of `World::observe_batch` on all processor cores
    /// ([`DriverOptions::observe_batch`]). The residents, the order and
    /// the answers are the same in both ways.
    pub fn observe(&mut self, world: &mut World) -> usize {
        let share = self.profile.observe_share;
        let mut slots = Vec::new();
        for slot in std::mem::take(&mut self.acted) {
            if random::chance(&mut self.rng, share) {
                slots.push(slot);
            }
        }
        let request = |driver: &Driver, slot: u32| {
            let opts = ObserveOpts {
                sections: driver.sections.clone(),
                since_notice: driver.cursors[slot as usize],
                ..ObserveOpts::default()
            };
            (driver.ids[slot as usize], opts)
        };
        let mut made = 0;
        if self.observe_batch == 0 {
            for slot in slots {
                let (resident, opts) = request(self, slot);
                made += usize::from(self.take_observation(slot, world.observe(resident, &opts)));
            }
        } else {
            for chunk in slots.chunks(self.observe_batch) {
                let requests: Vec<(ResidentId, ObserveOpts)> =
                    chunk.iter().map(|&slot| request(self, slot)).collect();
                let answers = world.observe_batch(&requests);
                for (&slot, answer) in chunk.iter().zip(answers) {
                    made += usize::from(self.take_observation(slot, answer));
                }
            }
        }
        self.counts.observes += made as u64;
        made
    }

    /// Takes the answer to the observation request of one resident: keeps
    /// the notice cursor and counts the reports. Returns true when the
    /// answer is an observation. A resident that is gone gives an error;
    /// that is counted at its next turn.
    fn take_observation(&mut self, slot: u32, answer: Result<Observation, SimError>) -> bool {
        let Ok(observation) = answer else {
            return false;
        };
        self.cursors[slot as usize] = observation.next_notice;
        self.counts.record_reports(&observation);
        // The observation is built and dropped: its cost is what counts
        // here.
        std::hint::black_box(&observation);
        true
    }

    /// Does one whole turn before a frame: plan, apply and observe. Returns
    /// the number of actions that were applied.
    pub fn turn(&mut self, world: &mut World) -> usize {
        self.plan(world);
        let applied = self.apply(world);
        self.observe(world);
        applied
    }
}
