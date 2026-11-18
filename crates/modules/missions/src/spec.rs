//! The definition of a mission as a user writes it. It is data only: a
//! mission never carries code.
//!
//! A resident sends a [`MissionSpec`] as the arguments of `mission_post`. A
//! human sponsor sends one inside a `SponsorMission` message. The checks
//! are in `check.rs`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::{OrgId, Value};

/// Who can take a mission.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// One resident takes the mission for itself.
    #[default]
    Solo,
    /// An organization takes the mission. Its members work together and
    /// share the reward.
    Team,
}

/// Whose events count for an objective.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub enum ActorRule {
    /// Only the assignee: the resident that took the mission, or the member
    /// that speaks for its team.
    #[default]
    Assignee,
    /// Every member of the team that joined the mission. Only a team
    /// mission can use it.
    Team,
}

fn one() -> u32 {
    1
}

/// One objective of a mission: a number of events of one kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSpec {
    /// Kind of the event, for example `economy.goods_produced`.
    pub event: String,
    /// Data fields that the event must have, for example
    /// `{"item": "bread"}`. Values are whole numbers, booleans or short
    /// names without spaces.
    #[serde(default)]
    pub fields: BTreeMap<String, Value>,
    /// How many such events the objective needs. The default is 1.
    #[serde(default = "one")]
    pub count: u32,
    /// Who must cause the events: `assignee` (the default) or `team`.
    #[serde(default)]
    pub actor: ActorRule,
}

/// The definition of a mission.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
pub struct MissionSpec {
    /// A short title.
    pub title: String,
    /// A longer description. It can be empty.
    #[serde(default)]
    pub text: String,
    /// What the taker must do. The engine counts the events and decides by
    /// itself when the mission is complete.
    pub objectives: Vec<ObjectiveSpec>,
    /// `solo` (the default) or `team`.
    #[serde(default)]
    pub mode: Mode,
    /// For a team mission: the only organization that can take it. Every
    /// organization can take it when absent.
    #[serde(default)]
    pub org: Option<OrgId>,
    /// Cents that each taker gets when it completes the mission. A team
    /// shares this amount.
    pub reward: i64,
    /// Game minutes from the publication to the deadline.
    pub deadline_in: u32,
    /// How many takers can take the mission. The default is 1. The poster
    /// locks `reward * takers` cents.
    #[serde(default = "one")]
    pub takers: u32,
}
