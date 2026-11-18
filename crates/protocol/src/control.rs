//! Control messages: spawn, administration, waiting and simulation errors.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{CityId, OwnerId, ResidentId, ZoneId};

/// A request to create a resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SpawnSpec {
    /// The owner of the new resident.
    pub owner: OwnerId,
    /// Display name, 1 to 64 bytes.
    pub name: String,
    /// Name of the model that controls the resident. It is shown to viewers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The city to start in. The city with the fewest residents when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<CityId>,
}

/// A command from an administrator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminCommand {
    /// Stop a resident: its action fails, its queue is cleared and every
    /// later request is refused.
    Freeze {
        /// The resident.
        resident: ResidentId,
    },
    /// Undo `freeze`.
    Unfreeze {
        /// The resident.
        resident: ResidentId,
    },
    /// Change a parameter.
    SetParam {
        /// Name of the parameter.
        name: String,
        /// New value. The server clamps it to the allowed range.
        value: i64,
        /// Change the value for one city only. The national value when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        city: Option<CityId>,
    },
    /// Remove a resident from the world.
    Despawn {
        /// The resident.
        resident: ResidentId,
    },
    /// Return one zone to an earlier state.
    RollbackZone {
        /// The zone.
        zone: ZoneId,
        /// How many frames to go back.
        frames: u64,
    },
}

/// The answer to an [`AdminCommand`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminReply {
    /// The command was applied.
    Ok,
    /// The command was refused.
    Error {
        /// Why the command was refused.
        message: String,
    },
}

/// What a waiting client wants to be woken for.
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WaitFilter {
    /// Any notice or action report.
    #[default]
    Any,
    /// A notice whose kind is in the list.
    Kinds {
        /// The kinds.
        kinds: Vec<String>,
    },
    /// An action report.
    ActionDone,
}

/// Why the simulation refused a call.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SimError {
    /// No living resident has this id.
    UnknownResident,
    /// The realm that the call needs is not hosted by this process.
    NotHosted,
    /// The simulation has too much work. Try again later.
    Overloaded,
    /// The simulation has stopped.
    Shutdown,
    /// The request is not valid.
    Invalid,
}

impl SimError {
    /// Returns the wire name of the error.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SimError::UnknownResident => "unknown_resident",
            SimError::NotHosted => "not_hosted",
            SimError::Overloaded => "overloaded",
            SimError::Shutdown => "shutdown",
            SimError::Invalid => "invalid",
        }
    }
}

impl fmt::Display for SimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            SimError::UnknownResident => "no living resident has this id",
            SimError::NotHosted => "the realm is not hosted by this process",
            SimError::Overloaded => "the simulation is overloaded",
            SimError::Shutdown => "the simulation has stopped",
            SimError::Invalid => "the request is not valid",
        })
    }
}

impl std::error::Error for SimError {}
