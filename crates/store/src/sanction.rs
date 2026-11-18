//! Sanctions: what an administrator put on a resident or on an owner.

use unsup_protocol::{OwnerId, ResidentId};

/// A sanction and its subject.
///
/// A mute and a freeze belong to one resident of the running world. They
/// end when the record of the resident ends. A ban belongs to an owner and
/// stays until an administrator lifts it.
///
/// The order of two sanctions is: mutes, then freezes, then bans, each by
/// ascending id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Sanction {
    /// The resident cannot send text.
    Mute(ResidentId),
    /// The resident is stopped in the simulation.
    Freeze(ResidentId),
    /// No token of the owner and no key of its residents works.
    Ban(OwnerId),
}

impl Sanction {
    /// Returns the name of the kind: `mute`, `freeze` or `ban`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Sanction::Mute(_) => "mute",
            Sanction::Freeze(_) => "freeze",
            Sanction::Ban(_) => "ban",
        }
    }

    /// Returns the id of the subject: a resident id for a mute and a
    /// freeze, an owner id for a ban.
    #[must_use]
    pub const fn subject(&self) -> u64 {
        match self {
            Sanction::Mute(resident) | Sanction::Freeze(resident) => resident.0,
            Sanction::Ban(owner) => owner.0,
        }
    }

    /// Returns the resident of a mute or a freeze.
    #[must_use]
    pub const fn resident(&self) -> Option<ResidentId> {
        match self {
            Sanction::Mute(resident) | Sanction::Freeze(resident) => Some(*resident),
            Sanction::Ban(_) => None,
        }
    }

    /// Makes a sanction from the name of its kind and the id of its
    /// subject. Returns `None` for a name that [`Sanction::kind`] does not
    /// give.
    #[must_use]
    pub fn from_parts(kind: &str, subject: u64) -> Option<Sanction> {
        match kind {
            "mute" => Some(Sanction::Mute(ResidentId(subject))),
            "freeze" => Some(Sanction::Freeze(ResidentId(subject))),
            "ban" => Some(Sanction::Ban(OwnerId(subject))),
            _ => None,
        }
    }
}

/// A sanction that is in force.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SanctionRecord {
    /// The sanction and its subject.
    pub sanction: Sanction,
    /// Why the administrator imposed it. It can be empty.
    pub reason: String,
    /// When it was imposed, in seconds since 1970.
    pub created_at: u64,
}
