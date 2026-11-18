//! The state of the mission engine: missions, takers and the indexes that
//! the engine needs to act in time.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::{CityId, Holder, OrgId, OwnerId, ResidentId, Resource, Value};

/// A value that an objective compares with a data field of an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Scalar {
    /// A whole number.
    Int(i64),
    /// A short name without spaces.
    Text(String),
    /// A boolean.
    Bool(bool),
}

impl Scalar {
    /// Returns true when a data field of an event has this value.
    pub(crate) fn matches(&self, field: &Value) -> bool {
        match self {
            Scalar::Int(want) => field.as_i64() == Some(*want),
            Scalar::Text(want) => field.as_str() == Some(want.as_str()),
            Scalar::Bool(want) => field.as_bool() == Some(*want),
        }
    }

    /// Returns the value as JSON.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Scalar::Int(v) => Value::from(*v),
            Scalar::Text(v) => Value::from(v.as_str()),
            Scalar::Bool(v) => Value::from(*v),
        }
    }
}

/// One objective of a mission, after the checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Objective {
    /// Kind of the event that counts.
    pub(crate) event: String,
    /// Data fields that the event must have, with their values.
    pub(crate) fields: Vec<(String, Scalar)>,
    /// How many events the objective needs.
    pub(crate) count: u32,
    /// True when the events of every member of the team count. False when
    /// only the events of the assignee count.
    pub(crate) team: bool,
}

/// The definition of a mission, after the checks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Definition {
    /// The title. A user wrote it.
    pub(crate) title: String,
    /// The description. A user wrote it.
    pub(crate) text: String,
    /// The objectives, at least one.
    pub(crate) objectives: Vec<Objective>,
    /// True for a team mission.
    pub(crate) team: bool,
    /// The only organization that can take a team mission, if the poster
    /// named one.
    pub(crate) org: Option<OrgId>,
    /// Cents for each taker that completes the mission.
    pub(crate) reward: i64,
    /// How many takers can take the mission.
    pub(crate) takers: u32,
    /// Game minutes from the publication to the deadline.
    pub(crate) deadline_in: u32,
}

impl Definition {
    /// Returns the cents that the mission locks: the reward for every
    /// taker. The checks made sure that the product fits.
    pub(crate) fn total(&self) -> i64 {
        self.reward.saturating_mul(i64::from(self.takers))
    }
}

/// Who published a mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Poster {
    /// A resident. The reward came from its cash.
    Resident {
        /// The resident.
        id: ResidentId,
        /// The owner of the resident.
        owner: OwnerId,
    },
    /// A human sponsor. The reward is new money.
    Sponsor {
        /// The name that the sponsor gave. A human wrote it.
        name: String,
    },
}

impl Poster {
    /// Returns the resident that published the mission, if a resident did.
    pub(crate) fn resident(&self) -> Option<ResidentId> {
        match self {
            Poster::Resident { id, .. } => Some(*id),
            Poster::Sponsor { .. } => None,
        }
    }
}

/// The payment of a taker that completed a mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Receipt {
    /// The frame of the payment.
    pub(crate) frame: u64,
    /// Who got how many cents, ordered by resident.
    pub(crate) shares: Vec<(ResidentId, i64)>,
}

/// One taker of a mission: a resident, or the team of an organization.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Taker {
    /// The resident that took the mission, or the member that speaks for
    /// its team.
    pub(crate) assignee: ResidentId,
    /// The frame in which the taker took the mission.
    pub(crate) accepted: u64,
    /// Events counted for each objective, in the order of the objectives.
    pub(crate) progress: Vec<u32>,
    /// The residents that work on the mission, with the events that each
    /// one added. A solo taker is its only member.
    pub(crate) members: BTreeMap<ResidentId, u32>,
    /// The payment, when the taker completed the mission.
    pub(crate) done: Option<Receipt>,
}

impl Taker {
    /// Returns true when every objective has its count.
    pub(crate) fn is_complete(&self, objectives: &[Objective]) -> bool {
        objectives.iter().zip(&self.progress).all(|(objective, done)| *done >= objective.count)
            && self.progress.len() == objectives.len()
    }
}

/// One open mission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Mission {
    /// Id of the mission. The escrow of the mission has the same number.
    pub(crate) id: u64,
    /// Who published it.
    pub(crate) poster: Poster,
    /// What it asks for and what it pays.
    pub(crate) def: Definition,
    /// The frame of the publication.
    pub(crate) posted: u64,
    /// The last frame in which an event counts. The mission closes in this
    /// frame.
    pub(crate) deadline: u64,
    /// The city where a resident published it. A sponsor mission has no
    /// place.
    pub(crate) city: Option<CityId>,
    /// The takers, by resident id for a solo mission and by organization id
    /// for a team mission. A taker that completed the mission stays in the
    /// list.
    pub(crate) takers: BTreeMap<u64, Taker>,
}

impl Mission {
    /// Returns the ledger holder that keeps the reward.
    pub(crate) fn escrow(&self) -> Holder {
        Missions::escrow(self.id)
    }

    /// Returns the number of takers that still work on the mission.
    pub(crate) fn active(&self) -> usize {
        self.takers.values().filter(|taker| taker.done.is_none()).count()
    }

    /// Returns the number of takers that completed the mission.
    pub(crate) fn completed(&self) -> usize {
        self.takers.len() - self.active()
    }

    /// Returns true when the mission takes no new taker.
    pub(crate) fn is_full(&self) -> bool {
        self.takers.len() >= self.def.takers as usize
    }

    /// Returns the resident whose text the mission carries, for the
    /// `Untrusted` wrapper: the poster, or nobody for a sponsor.
    pub(crate) fn author(&self) -> ResidentId {
        self.poster.resident().unwrap_or(crate::text::NOBODY)
    }
}

/// The missions of the world. A resource of the global realm.
///
/// The reward of a mission is the money of `Holder::Escrow(mission id)` in
/// the ledger of the global realm. This resource holds what the ledger does
/// not know.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Missions {
    /// The open missions, by id.
    pub(crate) open: BTreeMap<u64, Mission>,
    /// The deadlines: frame and mission.
    pub(crate) wake: BTreeSet<(u64, u64)>,
    /// The missions that each resident works on, with the key of its taker
    /// in the mission.
    pub(crate) enrolled: BTreeMap<ResidentId, BTreeMap<u64, u64>>,
    /// The number of open missions of each resident that published one.
    pub(crate) posted: BTreeMap<ResidentId, u32>,
    /// The heirs of the residents that left the world with open missions.
    pub(crate) heirs: BTreeMap<ResidentId, Holder>,
    /// The game day of `sponsor_minted`.
    pub(crate) sponsor_day: u64,
    /// Cents that the sponsor faucet created on that day.
    pub(crate) sponsor_minted: i64,
}

impl Resource for Missions {
    const NAME: &'static str = "missions.missions";
}

impl Missions {
    /// Returns the ledger holder that keeps the reward of a mission, in the
    /// global realm.
    #[must_use]
    pub fn escrow(mission: u64) -> Holder {
        Holder::Escrow(mission)
    }

    /// Returns the number of open missions.
    #[must_use]
    pub fn open_count(&self) -> usize {
        self.open.len()
    }

    /// Returns the ids of the open missions, oldest first.
    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        self.open.keys().copied()
    }

    /// Returns true when the mission is open.
    #[must_use]
    pub fn is_open(&self, mission: u64) -> bool {
        self.open.contains_key(&mission)
    }

    /// Returns the ids of the missions that a resident works on, in id
    /// order.
    #[must_use]
    pub fn accepted_by(&self, resident: ResidentId) -> Vec<u64> {
        self.enrolled.get(&resident).map(|list| list.keys().copied().collect()).unwrap_or_default()
    }

    /// Returns the cents that the sponsor faucet created on a game day.
    #[must_use]
    pub fn sponsor_minted_on(&self, day: u64) -> i64 {
        if self.sponsor_day == day { self.sponsor_minted } else { 0 }
    }

    /// Returns the number of open missions that a resident published.
    pub(crate) fn posted_by(&self, resident: ResidentId) -> usize {
        self.posted.get(&resident).copied().unwrap_or(0) as usize
    }

    /// Returns the number of missions that a resident works on.
    pub(crate) fn enrolled_count(&self, resident: ResidentId) -> usize {
        self.enrolled.get(&resident).map_or(0, BTreeMap::len)
    }

    /// Returns the key of the taker through which a resident works on a
    /// mission.
    pub(crate) fn key_of(&self, resident: ResidentId, mission: u64) -> Option<u64> {
        self.enrolled.get(&resident).and_then(|list| list.get(&mission)).copied()
    }

    /// Adds a new mission to the list and to the indexes.
    pub(crate) fn insert(&mut self, mission: Mission) {
        self.wake.insert((mission.deadline, mission.id));
        if let Some(poster) = mission.poster.resident() {
            *self.posted.entry(poster).or_default() += 1;
        }
        self.open.insert(mission.id, mission);
    }

    /// Takes a mission out of the list and out of every index. The heir of
    /// its poster stays until [`Missions::settle_heir`].
    pub(crate) fn remove(&mut self, id: u64) -> Option<Mission> {
        let mission = self.open.remove(&id)?;
        self.wake.remove(&(mission.deadline, id));
        if let Some(poster) = mission.poster.resident() {
            let left = self.posted.get(&poster).copied().unwrap_or(0).saturating_sub(1);
            if left == 0 {
                self.posted.remove(&poster);
            } else {
                self.posted.insert(poster, left);
            }
        }
        for taker in mission.takers.values().filter(|taker| taker.done.is_none()) {
            for &member in taker.members.keys() {
                self.unenrol(member, id);
            }
        }
        Some(mission)
    }

    /// Forgets the heir of a resident that has no open mission any more.
    pub(crate) fn settle_heir(&mut self, poster: ResidentId) {
        if self.posted_by(poster) == 0 {
            self.heirs.remove(&poster);
        }
    }

    /// Records that a resident works on a mission.
    pub(crate) fn enrol(&mut self, resident: ResidentId, mission: u64, key: u64) {
        self.enrolled.entry(resident).or_default().insert(mission, key);
    }

    /// Records that a resident no longer works on a mission.
    pub(crate) fn unenrol(&mut self, resident: ResidentId, mission: u64) {
        if let Some(list) = self.enrolled.get_mut(&resident) {
            list.remove(&mission);
            if list.is_empty() {
                self.enrolled.remove(&resident);
            }
        }
    }
}
