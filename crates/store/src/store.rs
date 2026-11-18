//! The interface between the gateway and a storage engine.

use unsup_protocol::{OwnerId, ResidentId};

use crate::error::StoreError;
use crate::model::{NewOwner, NewResident, Owner, Principal, ResidentRecord, StoreCounts, Webhook};
use crate::sanction::{Sanction, SanctionRecord};
use crate::token::{Secret, TokenHash};

/// A storage engine for owners, resident records, API keys, webhooks and
/// settings.
///
/// Every method is one atomic step: it happens completely or not at all.
/// The calls block, so an asynchronous caller runs them on a thread that
/// may block. A call that writes a time takes it as `now`, in seconds
/// since 1970.
///
/// A resident record is *live* while its resident is in the running world.
/// [`Store::retire_resident`] ends that. A retired record stays in the
/// store as history, its key stops working, its webhook is removed and its
/// mute and its freeze end. The methods that read residents see only live
/// records.
///
/// A setting whose key starts with `sanction:` is reserved for the engine.
pub trait Store: Send + Sync + 'static {
    /// Makes an owner and its first token.
    ///
    /// # Errors
    ///
    /// `Conflict` when the pair of provider and subject exists, or when
    /// the token exists.
    fn create_owner(
        &self,
        new: &NewOwner,
        token: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<Owner, StoreError>;

    /// Returns an owner by id.
    fn owner(&self, id: OwnerId) -> Result<Option<Owner>, StoreError>;

    /// Returns the owner that an identity provider knows under `subject`.
    fn owner_by_identity(&self, provider: &str, subject: &str)
    -> Result<Option<Owner>, StoreError>;

    /// Gives an owner one more token. The earlier tokens keep working.
    ///
    /// # Errors
    ///
    /// `NotFound` when the owner does not exist. `Conflict` when the token
    /// exists.
    fn add_owner_token(
        &self,
        owner: OwnerId,
        token: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<(), StoreError>;

    /// Finds who a token stands for. Returns `None` for a token that the
    /// store does not know or that no longer works.
    fn authenticate(&self, token: &TokenHash) -> Result<Option<Principal>, StoreError>;

    /// Makes the record of a new resident and its first key.
    ///
    /// # Errors
    ///
    /// `NotFound` when the owner does not exist. `Conflict` when a live
    /// record has the same resident id, or when the key exists.
    fn create_resident(
        &self,
        new: &NewResident,
        key: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<ResidentRecord, StoreError>;

    /// Returns the live record of a resident.
    fn resident(&self, resident: ResidentId) -> Result<Option<ResidentRecord>, StoreError>;

    /// Returns the live records of an owner, by ascending resident id.
    fn residents_of(&self, owner: OwnerId) -> Result<Vec<ResidentRecord>, StoreError>;

    /// Counts the live records of an owner.
    fn count_residents(&self, owner: OwnerId) -> Result<u64, StoreError>;

    /// Returns the ids of all residents with a live record, in ascending
    /// order.
    fn resident_ids(&self) -> Result<Vec<ResidentId>, StoreError>;

    /// Replaces the key of a resident. The old key stops working in the
    /// same step.
    ///
    /// # Errors
    ///
    /// `NotFound` when the resident has no live record. `Conflict` when
    /// the new key exists.
    fn rotate_resident_key(
        &self,
        resident: ResidentId,
        key: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<(), StoreError>;

    /// Ends the live record of a resident that left the world: the key
    /// stops working, the webhook is removed, and the mute and the freeze
    /// of the resident end. Returns false when the resident has no live
    /// record.
    fn retire_resident(
        &self,
        resident: ResidentId,
        reason: &str,
        now: u64,
    ) -> Result<bool, StoreError>;

    /// Ends every live record. The server calls it when it starts a new
    /// world: the resident ids of the old world mean nothing in the new
    /// one. Every mute and every freeze ends too; a ban stays. Returns the
    /// number of records that it ended.
    fn retire_all_residents(&self, reason: &str, now: u64) -> Result<u64, StoreError>;

    /// Puts a sanction in force. Returns false when it was in force
    /// already; its first reason and time stay.
    ///
    /// # Errors
    ///
    /// `NotFound` when the resident of a mute or a freeze has no live
    /// record, or when the owner of a ban does not exist.
    fn impose(&self, sanction: Sanction, reason: &str, now: u64) -> Result<bool, StoreError>;

    /// Lifts a sanction. Returns false when it was not in force.
    fn lift(&self, sanction: Sanction) -> Result<bool, StoreError>;

    /// Returns every sanction that is in force, in the order of
    /// [`Sanction`].
    fn sanctions(&self) -> Result<Vec<SanctionRecord>, StoreError>;

    /// Sets or replaces the webhook of a resident.
    ///
    /// # Errors
    ///
    /// `NotFound` when the resident has no live record.
    fn set_webhook(
        &self,
        resident: ResidentId,
        url: &str,
        secret: &Secret,
        now: u64,
    ) -> Result<(), StoreError>;

    /// Removes the webhook of a resident. Returns false when there was
    /// none.
    fn clear_webhook(&self, resident: ResidentId) -> Result<bool, StoreError>;

    /// Returns the webhook of a resident.
    fn webhook(&self, resident: ResidentId) -> Result<Option<Webhook>, StoreError>;

    /// Returns every webhook, by ascending resident id.
    fn webhooks(&self) -> Result<Vec<Webhook>, StoreError>;

    /// Returns the value of a setting.
    fn setting(&self, key: &str) -> Result<Option<String>, StoreError>;

    /// Sets or replaces a setting.
    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError>;

    /// Removes a setting. Returns false when there was none.
    fn clear_setting(&self, key: &str) -> Result<bool, StoreError>;

    /// Counts the records, for the statistics of an administrator.
    fn counts(&self) -> Result<StoreCounts, StoreError>;
}
