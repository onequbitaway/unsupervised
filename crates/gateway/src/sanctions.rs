//! Sanctions: mute and freeze a resident, ban an owner.
//!
//! The store keeps every sanction, so that it is in force after a
//! restart. This module keeps a copy in memory, because each request
//! asks whether its owner is banned and each text asks whether its
//! resident is muted.
//!
//! - A **mute** refuses every action request that carries speech.
//! - A **freeze** is the condition `frozen` of the simulation. The
//!   simulation enforces it. The store has it so that the server can put
//!   it back after a rollback of the world.
//! - A **ban** refuses every token of the owner and every key of its
//!   residents, and ends their open connections and waiting calls.

use std::collections::HashSet;
use std::sync::{PoisonError, RwLock};

use serde_json::{Value, json};
use tokio::sync::watch;
use unsup_protocol::{ActionRequest, AdminCommand, AdminReply, OwnerId, ResidentId, Routine};
use unsup_store::{Sanction, SanctionRecord, StoreError, unix_now};

use crate::actions::set_routines;
use crate::error::ApiError;
use crate::moderation::carries_speech;
use crate::state::Gateway;

#[derive(Debug, Default)]
struct State {
    muted: HashSet<ResidentId>,
    frozen: HashSet<ResidentId>,
    banned: HashSet<OwnerId>,
}

/// The sanctions in force, in memory.
#[derive(Debug)]
pub(crate) struct Sanctions {
    state: RwLock<State>,
    /// Counts the changes of the bans. An open connection watches it.
    bans: watch::Sender<u64>,
}

impl Sanctions {
    /// Makes the copy from the list of the store.
    pub(crate) fn load(records: &[SanctionRecord]) -> Sanctions {
        let sanctions =
            Sanctions { state: RwLock::new(State::default()), bans: watch::channel(0).0 };
        for record in records {
            sanctions.set(record.sanction, true);
        }
        sanctions
    }

    /// Puts a sanction into the copy, or takes it out.
    pub(crate) fn set(&self, sanction: Sanction, on: bool) {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        let changed = match sanction {
            Sanction::Mute(resident) if on => state.muted.insert(resident),
            Sanction::Mute(resident) => state.muted.remove(&resident),
            Sanction::Freeze(resident) if on => state.frozen.insert(resident),
            Sanction::Freeze(resident) => state.frozen.remove(&resident),
            Sanction::Ban(owner) if on => state.banned.insert(owner),
            Sanction::Ban(owner) => state.banned.remove(&owner),
        };
        drop(state);
        if changed && matches!(sanction, Sanction::Ban(_)) {
            self.bans.send_modify(|count| *count += 1);
        }
    }

    /// Forgets the mute and the freeze of a resident that left the world.
    pub(crate) fn forget(&self, resident: ResidentId) {
        let mut state = self.state.write().unwrap_or_else(PoisonError::into_inner);
        state.muted.remove(&resident);
        state.frozen.remove(&resident);
    }

    pub(crate) fn is_muted(&self, resident: ResidentId) -> bool {
        self.state.read().unwrap_or_else(PoisonError::into_inner).muted.contains(&resident)
    }

    pub(crate) fn is_banned(&self, owner: OwnerId) -> bool {
        self.state.read().unwrap_or_else(PoisonError::into_inner).banned.contains(&owner)
    }

    /// Returns the residents with a freeze, in ascending order.
    pub(crate) fn frozen(&self) -> Vec<ResidentId> {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        let mut frozen: Vec<ResidentId> = state.frozen.iter().copied().collect();
        frozen.sort_unstable();
        frozen
    }

    /// Returns the numbers of mutes, freezes and bans.
    pub(crate) fn counts(&self) -> Value {
        let state = self.state.read().unwrap_or_else(PoisonError::into_inner);
        json!({
            "muted": state.muted.len(),
            "frozen": state.frozen.len(),
            "banned": state.banned.len(),
        })
    }

    /// Waits until `owner` is banned. A caller that holds a connection or
    /// a waiting call of the owner ends it then.
    pub(crate) async fn banned(&self, owner: OwnerId) {
        let mut bans = self.bans.subscribe();
        while !self.is_banned(owner) {
            if bans.changed().await.is_err() {
                // The gateway is gone. Nothing bans anybody any more.
                std::future::pending::<()>().await;
            }
        }
    }
}

impl Gateway {
    /// Puts a sanction in force: in the store, then in memory. Returns
    /// false when it was in force already.
    pub(crate) async fn impose(
        &self,
        sanction: Sanction,
        reason: String,
    ) -> Result<bool, ApiError> {
        let stored = self
            .store(move |store| match store.impose(sanction, &reason, unix_now()) {
                Ok(new) => Ok(Some(new)),
                Err(StoreError::NotFound(_)) => Ok(None),
                Err(error) => Err(error),
            })
            .await?;
        let Some(new) = stored else {
            return Err(match sanction {
                Sanction::Ban(_) => ApiError::not_found("No owner has this id."),
                _ => ApiError::not_found("No resident with this id has an owner record."),
            });
        };
        self.inner.sanctions.set(sanction, true);
        Ok(new)
    }

    /// Lifts a sanction: in the store, then in memory. Returns false when
    /// it was not in force.
    pub(crate) async fn lift(&self, sanction: Sanction) -> Result<bool, ApiError> {
        let lifted = self.store(move |store| store.lift(sanction)).await?;
        self.inner.sanctions.set(sanction, false);
        Ok(lifted)
    }

    /// Stops the speech that a resident planned before its mute: the
    /// routines that speak go, and a queue with a verb that carries text
    /// is cancelled. The simulation runs routines and the queue without
    /// the gateway, so the mute would not reach them.
    pub(crate) async fn silence(&self, resident: ResidentId) {
        let config = &self.inner.moderation.config;
        let opts = crate::actions::observe_opts(
            Some(unsup_protocol::Detail::Full),
            Some(u64::MAX),
            Some(vec!["self".to_string()]),
        );
        let Ok(observation) = self.sim().observe(resident, opts).await else {
            return;
        };
        let myself = observation.sections.get("self");
        let routines: Vec<Routine> = myself
            .and_then(|myself| myself.get("routines"))
            .and_then(|routines| serde_json::from_value(routines.clone()).ok())
            .unwrap_or_default();
        let quiet: Vec<Routine> =
            routines.iter().filter(|r| !carries_speech(config, &r.then)).cloned().collect();
        if quiet.len() < routines.len() {
            let removed = routines.len() - quiet.len();
            match set_routines(self.sim(), resident, &quiet).await {
                Ok(ack) if ack.status.is_accepted() => {
                    tracing::info!(%resident, removed, "mute: routines that speak were removed");
                }
                _ => {
                    tracing::warn!(%resident, "mute: the routines that speak could not be removed")
                }
            }
        }
        let verb_of = |entry: &Value| entry.get("verb").and_then(Value::as_str).map(str::to_string);
        let planned = myself
            .and_then(|myself| myself.get("queued"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(verb_of);
        let running = myself.and_then(|myself| myself.get("activity")).and_then(verb_of);
        if planned.chain(running).any(|verb| config.text_verbs.contains(&verb)) {
            let cancel = ActionRequest::new("cancel", Value::Null);
            if self.sim().act(resident, cancel).await.is_err() {
                tracing::warn!(%resident, "mute: the planned speech could not be cancelled");
            }
        }
    }

    /// Puts every stored freeze back into the simulation. The server calls
    /// it after a rollback: the world of the snapshot can be from before
    /// the freeze. Returns the number of residents that it froze.
    pub(crate) async fn refreeze(&self) -> u32 {
        let mut done = 0;
        for resident in self.inner.sanctions.frozen() {
            match self.sim().admin(AdminCommand::Freeze { resident }).await {
                Ok(AdminReply::Ok) => done += 1,
                Ok(AdminReply::Error { message }) => {
                    tracing::warn!(%resident, message, "a stored freeze could not be put back");
                }
                Err(error) => {
                    tracing::warn!(%resident, %error, "a stored freeze could not be put back");
                }
            }
        }
        done
    }
}
