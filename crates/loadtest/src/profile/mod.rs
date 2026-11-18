//! Behaviour profiles: what residents do under load.
//!
//! A profile is a TOML file. It lists actions with weights, says how the
//! arguments of each action are made, and gives the think time between two
//! actions. The verbs of a profile are matched with the catalogue of the
//! world at run time ([`Profile::resolve`]): a verb that the world does not
//! have is skipped with a warning, so one profile works while the game
//! grows.
//!
//! ```toml
//! name = "walkers"
//! summary = "Residents walk to random tiles near them."
//! observe_share = 0.25      # chance of an observation after an action
//! wait_share = 0.5          # swarm: chance to wait for the end of an action
//!
//! [think]                   # ticks between two actions, in one process
//! kind = "uniform"
//! min = 0
//! max = 10
//!
//! [[action]]
//! verb = "move_to"
//! weight = 9
//! args = { gen = "near_tile", radius = 12 }
//! ```

mod args;
mod resolve;

use std::fmt;
use std::path::Path;

use serde::Deserialize;

pub use args::{ArgSpec, Generator, ItemAs};
pub use resolve::{Choice, Resolved};

use crate::think::TickThink;

/// The profiles that are part of the binary: name and TOML text.
const BUILTIN: [(&str, &str); 3] = [
    ("walkers", include_str!("../../profiles/walkers.toml")),
    ("rush", include_str!("../../profiles/rush.toml")),
    ("daily_life", include_str!("../../profiles/daily_life.toml")),
];

/// A profile could not be read or is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileError {
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid behaviour profile: {}", self.message)
    }
}

impl std::error::Error for ProfileError {}

fn invalid(message: impl Into<String>) -> ProfileError {
    ProfileError { message: message.into() }
}

/// One weighted action of a profile.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActionSpec {
    /// Name of the verb.
    pub verb: String,
    /// How often the action is chosen, relative to the other actions. An
    /// action with weight 0 is never chosen.
    pub weight: u32,
    /// How the arguments are made.
    #[serde(default)]
    pub args: ArgSpec,
}

/// A behaviour profile, as it is written in the file.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// Name of the profile, for reports.
    pub name: String,
    /// One sentence that says what the residents do.
    #[serde(default)]
    pub summary: String,
    /// Ticks between the end of one action and the next action, for the
    /// commands that run in one process.
    pub think: TickThink,
    /// Chance, from 0 to 1, that a resident observes after an action.
    #[serde(default)]
    pub observe_share: f64,
    /// Chance, from 0 to 1, that an agent of the swarm waits for the end of
    /// an action that takes time before it acts again.
    #[serde(default)]
    pub wait_share: f64,
    /// The actions.
    #[serde(rename = "action")]
    pub actions: Vec<ActionSpec>,
}

impl Profile {
    /// Reads a profile from TOML text and checks it.
    pub fn parse(text: &str) -> Result<Profile, ProfileError> {
        let profile: Profile = toml::from_str(text).map_err(|error| invalid(error.to_string()))?;
        profile.validate()?;
        Ok(profile)
    }

    /// Reads a profile by the name of a built-in profile or by the path of
    /// a file. A built-in name wins.
    pub fn load(name_or_path: &str) -> Result<Profile, ProfileError> {
        if let Some((_, text)) = BUILTIN.iter().find(|(name, _)| *name == name_or_path) {
            return Profile::parse(text);
        }
        let path = Path::new(name_or_path);
        let text = std::fs::read_to_string(path).map_err(|error| {
            invalid(format!(
                "cannot read `{}`: {error}. Built-in profiles: {}",
                path.display(),
                Profile::builtin_names().join(", ")
            ))
        })?;
        Profile::parse(&text)
            .map_err(|error| invalid(format!("{}: {}", path.display(), error.message)))
    }

    /// Returns the names of the profiles that are part of the binary.
    #[must_use]
    pub fn builtin_names() -> Vec<&'static str> {
        BUILTIN.iter().map(|(name, _)| *name).collect()
    }

    fn validate(&self) -> Result<(), ProfileError> {
        if self.name.trim().is_empty() {
            return Err(invalid("the profile has no name"));
        }
        self.think.validate().map_err(invalid)?;
        for (name, share) in
            [("observe_share", self.observe_share), ("wait_share", self.wait_share)]
        {
            if !(0.0..=1.0).contains(&share) {
                return Err(invalid(format!("{name} is {share}, it must be from 0 to 1")));
            }
        }
        if self.actions.is_empty() {
            return Err(invalid("the profile has no action"));
        }
        if self.actions.iter().all(|action| action.weight == 0) {
            return Err(invalid("every action has weight 0"));
        }
        if let Some(action) = self.actions.iter().find(|action| action.verb.trim().is_empty()) {
            return Err(invalid(format!("an action with weight {} has no verb", action.weight)));
        }
        Ok(())
    }
}
