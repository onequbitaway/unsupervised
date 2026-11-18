//! Brings the store back in line with the world after the world changed
//! behind it: a recovery at the start of the server, or a rollback.
//!
//! The store and the world are two things on the disk. Each write goes
//! to one of them first. A crash between the two, a loss of the last
//! inputs of the log, or a rollback of the world leaves records for
//! residents that the world does not have. Such a record must end before
//! the world gives its resident id to a new resident: else the old key
//! would control the new resident.

use std::collections::BTreeSet;

use serde::Serialize;
use serde_json::{Value, json};
use unsup_protocol::{ResidentId, SimError, ViewQuery, WorldView};
use unsup_runtime::RollbackError;

use crate::error::ApiError;
use crate::push::cursor_now;
use crate::state::Gateway;

/// What [`Gateway::reconcile`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Reconciled {
    /// Residents whose record ended, because the world does not have
    /// them. Their keys stop.
    pub retired: Vec<ResidentId>,
    /// Residents of the world without a record. Nobody has a key for
    /// them. An administrator can remove them with `despawn`.
    pub orphans: Vec<ResidentId>,
    /// Number of residents that got their stored freeze again.
    pub refrozen: u32,
}

impl Gateway {
    /// Returns the id of every resident of the world.
    async fn world_residents(&self) -> Result<BTreeSet<ResidentId>, SimError> {
        let WorldView::Layout { zones, .. } = self.sim().view(ViewQuery::Layout).await? else {
            return Err(SimError::Invalid);
        };
        let mut residents = BTreeSet::new();
        for zone in zones.iter().filter(|zone| zone.residents > 0) {
            if let WorldView::Zone { residents: here, .. } =
                self.sim().view(ViewQuery::Zone { zone: zone.id }).await?
            {
                residents.extend(here.iter().map(|resident| resident.id));
            }
        }
        Ok(residents)
    }

    /// Compares the records of the store with the residents of the world.
    /// The caller holds the lock that makes resident creation wait.
    async fn reconcile_now(&self) -> Result<Reconciled, ApiError> {
        let in_world = self.world_residents().await?;
        let recorded = self.store(|store| store.resident_ids()).await?;
        let mut retired = Vec::new();
        for resident in recorded.iter().filter(|resident| !in_world.contains(resident)) {
            self.retire(*resident, "lost").await;
            retired.push(*resident);
        }
        let recorded: BTreeSet<ResidentId> = recorded.into_iter().collect();
        let orphans: Vec<ResidentId> = in_world.difference(&recorded).copied().collect();
        let refrozen = self.refreeze().await;
        if !retired.is_empty() || !orphans.is_empty() {
            tracing::warn!(
                retired = retired.len(),
                orphans = orphans.len(),
                "the store and the world did not match"
            );
        }
        Ok(Reconciled { retired, orphans, refrozen })
    }

    /// Brings the store in line with the world. The server calls it after
    /// it recovered a world from its data directory, before it takes
    /// requests.
    ///
    /// - A record whose resident is not in the world ends. Its key stops.
    /// - A resident of the world without a record is reported.
    /// - Every stored freeze is put into the world again.
    ///
    /// # Errors
    ///
    /// Fails when the simulation or the store does not answer. Nothing is
    /// lost then: call it again.
    pub async fn reconcile(&self) -> Result<Reconciled, ApiError> {
        let _no_creation = self.inner.creation.lock().await;
        self.reconcile_now().await
    }

    /// Moves the notice cursors of the gateway back with the world. The
    /// inbox of a resident is shorter after a rollback, and its next news
    /// gets sequence numbers that an old cursor already passed. Without
    /// this step such news would never be pushed.
    async fn rewind_cursors(&self) -> Result<(), ApiError> {
        let inner = &self.inner;
        // Open WebSocket connections move their own cursors.
        inner.rollbacks.send_modify(|count| *count += 1);
        // The cursors that the MCP layer keeps for its clients.
        for resident in inner.sessions.residents() {
            match cursor_now(self.sim(), resident).await {
                Ok(end) => inner.sessions.clamp(resident, end),
                Err(SimError::UnknownResident) => inner.sessions.forget(resident),
                // The simulation is busy. The rollback is done, so this
                // is no reason to fail: the next call of the client finds
                // the shorter inbox and moves its cursor then.
                Err(_) => {}
            }
        }
        // A webhook starts again: it delivers the news from now on.
        let saved = self
            .store(|store| {
                let mut saved = Vec::new();
                for webhook in store.webhooks()? {
                    let owner = store.resident(webhook.resident)?.map(|record| record.owner);
                    saved.push((webhook, owner));
                }
                Ok(saved)
            })
            .await?;
        for (webhook, owner) in saved {
            if owner.is_some_and(|owner| inner.sanctions.is_banned(owner)) {
                continue;
            }
            inner.webhooks.set(webhook.resident, &webhook.url, webhook.secret).await;
        }
        Ok(())
    }

    /// Rolls the whole world back to the newest snapshot at or before
    /// `frame`, then brings the store in line with that world.
    pub(crate) async fn rollback(&self, frame: u64) -> Result<Value, ApiError> {
        // No resident is made between the rollback and the end of the
        // records that the old world line made.
        let _no_creation = self.inner.creation.lock().await;
        let report = self.sim().rollback(frame).await.map_err(|error| match error {
            RollbackError::Sim(error) => ApiError::from(error),
            RollbackError::Failed { .. } => {
                self.inner.ops.alerts.raise("rollback_failed", error.to_string());
                ApiError::internal()
            }
            refused @ (RollbackError::Unsupported | RollbackError::NoSnapshot { .. }) => {
                let mut message = refused.to_string();
                if let Some(first) = message.get_mut(0..1) {
                    first.make_ascii_uppercase();
                }
                ApiError::new(axum::http::StatusCode::CONFLICT, "refused", format!("{message}."))
            }
        })?;
        tracing::warn!(
            frame = report.frame,
            dropped_inputs = report.dropped_inputs,
            "admin: the world was rolled back"
        );
        let reconciled = self.reconcile_now().await?;
        self.rewind_cursors().await?;
        Ok(json!({
            "ok": true,
            "frame": report.frame,
            "dropped_inputs": report.dropped_inputs,
            "retired": reconciled.retired,
            "orphans": reconciled.orphans,
            "refrozen": reconciled.refrozen,
        }))
    }
}
