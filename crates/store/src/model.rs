//! The records that the store keeps.

use std::time::{SystemTime, UNIX_EPOCH};

use unsup_protocol::{OwnerId, ResidentId};

use crate::token::Secret;

/// Returns the wall clock as seconds since 1970. The store never reads the
/// clock itself: every call that writes a time takes it as an argument.
#[must_use]
pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// A human or an organization that controls residents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    /// Id of the owner. The store gives the ids, from 1.
    pub id: OwnerId,
    /// Display name.
    pub name: String,
    /// Name of the identity provider that knows the owner.
    pub provider: String,
    /// The id of the owner at the provider. The pair of provider and
    /// subject is unique.
    pub subject: String,
    /// When the owner signed up, in seconds since 1970.
    pub created_at: u64,
}

/// The data of a new owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOwner {
    /// Display name.
    pub name: String,
    /// Name of the identity provider.
    pub provider: String,
    /// The id of the owner at the provider.
    pub subject: String,
}

/// What the store knows about one resident of the running world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResidentRecord {
    /// Id of the resident in the simulation.
    pub resident: ResidentId,
    /// The owner.
    pub owner: OwnerId,
    /// Display name, as the owner gave it.
    pub name: String,
    /// Name of the model that controls the resident.
    pub model: Option<String>,
    /// When the resident was made, in seconds since 1970.
    pub created_at: u64,
    /// Hint of the key that works now: its prefix and first characters.
    pub key_hint: String,
}

/// The data of a new resident record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewResident {
    /// Id that the simulation gave to the resident.
    pub resident: ResidentId,
    /// The owner.
    pub owner: OwnerId,
    /// Display name.
    pub name: String,
    /// Name of the model that controls the resident.
    pub model: Option<String>,
}

/// Who a token stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Principal {
    /// The token of an owner.
    Owner(OwnerId),
    /// The key of a resident.
    Resident {
        /// The resident.
        resident: ResidentId,
        /// The owner of the resident.
        owner: OwnerId,
    },
}

/// Where the server sends the notices of one resident.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Webhook {
    /// The resident.
    pub resident: ResidentId,
    /// The address that gets the deliveries.
    pub url: String,
    /// The secret that signs the deliveries.
    pub secret: Secret,
    /// When the webhook was set, in seconds since 1970.
    pub created_at: u64,
}

/// How many records the store holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StoreCounts {
    /// Owners.
    pub owners: u64,
    /// Residents of the running world.
    pub residents: u64,
    /// Residents that left the world, in this world or an earlier one.
    pub retired: u64,
    /// Webhooks.
    pub webhooks: u64,
}
