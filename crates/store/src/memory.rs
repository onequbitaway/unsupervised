//! A store that keeps everything in memory.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use unsup_protocol::{OwnerId, ResidentId};

use crate::error::StoreError;
use crate::model::{NewOwner, NewResident, Owner, Principal, ResidentRecord, StoreCounts, Webhook};
use crate::store::Store;
use crate::token::{Secret, TokenHash};

/// A store in memory. It forgets everything when it is dropped. Tests and
/// the development mode of the server use it.
#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    last_owner: u64,
    owners: BTreeMap<OwnerId, Owner>,
    tokens: BTreeMap<TokenHash, Principal>,
    residents: BTreeMap<ResidentId, ResidentRecord>,
    /// The key of each live resident.
    keys: BTreeMap<ResidentId, TokenHash>,
    webhooks: BTreeMap<ResidentId, Webhook>,
    retired: u64,
    settings: BTreeMap<String, String>,
}

impl Inner {
    /// Ends one live record. Returns false when there was none.
    fn retire(&mut self, resident: ResidentId) -> bool {
        if self.residents.remove(&resident).is_none() {
            return false;
        }
        if let Some(key) = self.keys.remove(&resident) {
            self.tokens.remove(&key);
        }
        self.webhooks.remove(&resident);
        self.retired += 1;
        true
    }
}

impl MemoryStore {
    /// Makes an empty store.
    #[must_use]
    pub fn new() -> MemoryStore {
        MemoryStore::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Every method leaves the maps consistent before it can panic, so
        // the data behind a poisoned lock is still good.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn token_exists() -> StoreError {
    StoreError::Conflict("the token exists already".into())
}

fn no_owner(owner: OwnerId) -> StoreError {
    StoreError::NotFound(format!("owner {owner}"))
}

fn no_resident(resident: ResidentId) -> StoreError {
    StoreError::NotFound(format!("resident {resident}"))
}

impl Store for MemoryStore {
    fn create_owner(
        &self,
        new: &NewOwner,
        token: &TokenHash,
        _hint: &str,
        now: u64,
    ) -> Result<Owner, StoreError> {
        let mut inner = self.lock();
        if inner.owners.values().any(|o| o.provider == new.provider && o.subject == new.subject) {
            return Err(StoreError::Conflict(format!(
                "an owner of the provider `{}` with this subject",
                new.provider
            )));
        }
        if inner.tokens.contains_key(token) {
            return Err(token_exists());
        }
        inner.last_owner += 1;
        let owner = Owner {
            id: OwnerId(inner.last_owner),
            name: new.name.clone(),
            provider: new.provider.clone(),
            subject: new.subject.clone(),
            created_at: now,
        };
        inner.owners.insert(owner.id, owner.clone());
        inner.tokens.insert(*token, Principal::Owner(owner.id));
        Ok(owner)
    }

    fn owner(&self, id: OwnerId) -> Result<Option<Owner>, StoreError> {
        Ok(self.lock().owners.get(&id).cloned())
    }

    fn owner_by_identity(
        &self,
        provider: &str,
        subject: &str,
    ) -> Result<Option<Owner>, StoreError> {
        let inner = self.lock();
        Ok(inner.owners.values().find(|o| o.provider == provider && o.subject == subject).cloned())
    }

    fn add_owner_token(
        &self,
        owner: OwnerId,
        token: &TokenHash,
        _hint: &str,
        _now: u64,
    ) -> Result<(), StoreError> {
        let mut inner = self.lock();
        if !inner.owners.contains_key(&owner) {
            return Err(no_owner(owner));
        }
        if inner.tokens.contains_key(token) {
            return Err(token_exists());
        }
        inner.tokens.insert(*token, Principal::Owner(owner));
        Ok(())
    }

    fn authenticate(&self, token: &TokenHash) -> Result<Option<Principal>, StoreError> {
        let inner = self.lock();
        // The map finds the entry; the constant-time check confirms it.
        Ok(inner
            .tokens
            .get_key_value(token)
            .filter(|(stored, _)| stored.matches(token))
            .map(|(_, principal)| *principal))
    }

    fn create_resident(
        &self,
        new: &NewResident,
        key: &TokenHash,
        hint: &str,
        now: u64,
    ) -> Result<ResidentRecord, StoreError> {
        let mut inner = self.lock();
        if !inner.owners.contains_key(&new.owner) {
            return Err(no_owner(new.owner));
        }
        if inner.residents.contains_key(&new.resident) {
            return Err(StoreError::Conflict(format!("resident {}", new.resident)));
        }
        if inner.tokens.contains_key(key) {
            return Err(token_exists());
        }
        let record = ResidentRecord {
            resident: new.resident,
            owner: new.owner,
            name: new.name.clone(),
            model: new.model.clone(),
            created_at: now,
            key_hint: hint.to_string(),
        };
        inner.residents.insert(record.resident, record.clone());
        inner.keys.insert(record.resident, *key);
        inner
            .tokens
            .insert(*key, Principal::Resident { resident: record.resident, owner: record.owner });
        Ok(record)
    }

    fn resident(&self, resident: ResidentId) -> Result<Option<ResidentRecord>, StoreError> {
        Ok(self.lock().residents.get(&resident).cloned())
    }

    fn residents_of(&self, owner: OwnerId) -> Result<Vec<ResidentRecord>, StoreError> {
        Ok(self.lock().residents.values().filter(|r| r.owner == owner).cloned().collect())
    }

    fn count_residents(&self, owner: OwnerId) -> Result<u64, StoreError> {
        Ok(self.lock().residents.values().filter(|r| r.owner == owner).count() as u64)
    }

    fn rotate_resident_key(
        &self,
        resident: ResidentId,
        key: &TokenHash,
        hint: &str,
        _now: u64,
    ) -> Result<(), StoreError> {
        let mut inner = self.lock();
        let owner = inner.residents.get(&resident).ok_or_else(|| no_resident(resident))?.owner;
        if inner.tokens.contains_key(key) {
            return Err(token_exists());
        }
        if let Some(old) = inner.keys.insert(resident, *key) {
            inner.tokens.remove(&old);
        }
        inner.tokens.insert(*key, Principal::Resident { resident, owner });
        if let Some(record) = inner.residents.get_mut(&resident) {
            record.key_hint = hint.to_string();
        }
        Ok(())
    }

    fn retire_resident(
        &self,
        resident: ResidentId,
        _reason: &str,
        _now: u64,
    ) -> Result<bool, StoreError> {
        Ok(self.lock().retire(resident))
    }

    fn retire_all_residents(&self, _reason: &str, _now: u64) -> Result<u64, StoreError> {
        let mut inner = self.lock();
        let live: Vec<ResidentId> = inner.residents.keys().copied().collect();
        for resident in &live {
            inner.retire(*resident);
        }
        Ok(live.len() as u64)
    }

    fn set_webhook(
        &self,
        resident: ResidentId,
        url: &str,
        secret: &Secret,
        now: u64,
    ) -> Result<(), StoreError> {
        let mut inner = self.lock();
        if !inner.residents.contains_key(&resident) {
            return Err(no_resident(resident));
        }
        let webhook =
            Webhook { resident, url: url.to_string(), secret: secret.clone(), created_at: now };
        inner.webhooks.insert(resident, webhook);
        Ok(())
    }

    fn clear_webhook(&self, resident: ResidentId) -> Result<bool, StoreError> {
        Ok(self.lock().webhooks.remove(&resident).is_some())
    }

    fn webhook(&self, resident: ResidentId) -> Result<Option<Webhook>, StoreError> {
        Ok(self.lock().webhooks.get(&resident).cloned())
    }

    fn webhooks(&self) -> Result<Vec<Webhook>, StoreError> {
        Ok(self.lock().webhooks.values().cloned().collect())
    }

    fn setting(&self, key: &str) -> Result<Option<String>, StoreError> {
        Ok(self.lock().settings.get(key).cloned())
    }

    fn set_setting(&self, key: &str, value: &str) -> Result<(), StoreError> {
        self.lock().settings.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn clear_setting(&self, key: &str) -> Result<bool, StoreError> {
        Ok(self.lock().settings.remove(key).is_some())
    }

    fn counts(&self) -> Result<StoreCounts, StoreError> {
        let inner = self.lock();
        Ok(StoreCounts {
            owners: inner.owners.len() as u64,
            residents: inner.residents.len() as u64,
            retired: inner.retired,
            webhooks: inner.webhooks.len() as u64,
        })
    }
}
