//! The start of a cluster: a new world or the recovery of a stored one.

use std::time::Duration;

use unsup_protocol::{RealmId, ZoneId};

use super::super::cell::{Cell, CellStore, Start};
use super::super::config::ClusterConfig;
use super::super::error::ClusterError;
use super::super::lookup::Where;
use super::super::proto::{FromWorker, ToWorker, ZoneReady, ZoneState};
use super::super::seals::SealPoint;
use super::core::{Core, unexpected};
use crate::durable::Plugins;

/// Returns the newest seal that every realm has, or `None` when no realm
/// has a seal.
fn common_seal(realms: &[(RealmId, &[SealPoint])]) -> Result<Option<(u64, u64)>, ClusterError> {
    if realms.iter().all(|(_, seals)| seals.is_empty()) {
        return Ok(None);
    }
    let fail = |message: String| Err(ClusterError::Recovery { realm: None, message });
    let Some((first_realm, first)) = realms.first() else {
        return Ok(None);
    };
    for seal in first.iter().rev() {
        let everywhere =
            realms.iter().all(|(_, seals)| seals.iter().any(|other| other.epoch == seal.epoch));
        if !everywhere {
            continue;
        }
        // One seal is one frame in every realm.
        for (realm, seals) in realms {
            if let Some(other) = seals.iter().find(|other| other.epoch == seal.epoch)
                && other.frame != seal.frame
            {
                return fail(format!(
                    "seal {} is at frame {} in {first_realm} and at frame {} in {realm}",
                    seal.epoch, seal.frame, other.frame
                ));
            }
        }
        return Ok(Some((seal.epoch, seal.frame)));
    }
    // A crash in the first exchange of a new world: some realms wrote
    // seal 0 and the others no seal. No input came before that seal was
    // in every realm, so nothing is lost when the world starts as new.
    if realms.iter().all(|(_, seals)| seals.iter().all(|seal| seal.epoch == 0)) {
        return Ok(None);
    }
    let without: Vec<String> = realms
        .iter()
        .filter(|(_, seals)| seals.is_empty())
        .map(|(realm, _)| realm.to_string())
        .collect();
    if without.is_empty() {
        fail("no seal is in the storage of every realm".into())
    } else {
        fail(format!("some realms have stored state and these have none: {}", without.join(", ")))
    }
}

/// Decides how the cluster starts and makes or recovers the world of the
/// global realm. Returns that world and the number of the seal to which
/// every realm recovers; `None` is a new world.
pub(crate) fn open_global(
    plugins: &Plugins,
    config: &ClusterConfig,
    states: &[Vec<ZoneState>],
) -> Result<(Cell, Option<u64>), ClusterError> {
    let global = RealmId::Global;
    let store = config.durable.as_ref().map(|durable| CellStore::of(durable, global));
    let global_seals = match &store {
        Some(store) => store
            .seals()
            .map_err(|message| ClusterError::Recovery { realm: Some(global), message })?,
        None => Vec::new(),
    };
    let mut realms: Vec<(RealmId, &[SealPoint])> = vec![(global, global_seals.as_slice())];
    for state in states.iter().flatten() {
        realms.push((RealmId::Zone(state.zone), state.seals.as_slice()));
    }
    let recover = common_seal(&realms)?.map(|(epoch, _)| epoch);
    let start = recover
        .and_then(|epoch| global_seals.iter().copied().find(|seal| seal.epoch == epoch))
        .map_or(Start::Fresh, Start::Recover);
    let cell = Cell::open(global, plugins, &config.world, store, start)?;
    Ok((cell, recover))
}

impl Core {
    /// Tells every worker to make or recover its worlds and waits until
    /// all are ready. A new world then gets its first exchange. A stored
    /// world gets its directory back.
    pub(crate) fn begin(
        &mut self,
        recover: Option<u64>,
        patience: Duration,
    ) -> Result<(), ClusterError> {
        let begin = ToWorker::Begin { recover };
        for link in &mut self.links {
            link.send(&begin)?;
        }
        let mut ready: Vec<ZoneReady> = Vec::with_capacity(self.zones());
        for (slot, link) in self.links.iter_mut().enumerate() {
            match link.recv(patience)? {
                FromWorker::Ready { zones } => ready.extend(zones),
                other => return Err(unexpected(slot, "Ready", &other)),
            }
        }
        ready.sort_by_key(|zone| zone.zone);
        if !ready.iter().map(|zone| zone.zone).eq((0..self.zones() as u32).map(ZoneId)) {
            return Err(ClusterError::Protocol("the workers are not ready for every zone".into()));
        }
        let frame = self.global.world.frame();
        if let Some(zone) = ready.iter().find(|zone| zone.frame != frame) {
            return Err(ClusterError::Recovery {
                realm: Some(RealmId::Zone(zone.zone)),
                message: format!(
                    "the zone is at frame {}, the global realm at frame {frame}",
                    zone.frame
                ),
            });
        }
        self.residents = ready.iter().map(|zone| zone.residents.len() as u32).collect();
        if recover.is_none() {
            // A new world: every realm gets what the others sent when
            // they were made. That ends with seal 0.
            return self
                .first_exchange(ready.into_iter().map(|zone| (zone.zone, zone.packets)).collect());
        }
        // A stored world: the directory of the global realm does not have
        // the inputs since its last tick. The zones tell where their
        // residents are.
        for zone in &ready {
            for &resident in &zone.residents {
                if self.lookup.find(&self.global.world, resident) != Where::Alive(zone.zone) {
                    self.lookup.moved(resident, zone.zone);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seal(epoch: u64, frame: u64) -> SealPoint {
        SealPoint { epoch, frame, seq: epoch * 3 }
    }

    #[test]
    fn the_cluster_recovers_to_the_newest_seal_that_every_realm_has() {
        let zone = |z| RealmId::Zone(ZoneId(z));
        let a = [seal(3, 3), seal(4, 4), seal(5, 5)];
        let b = [seal(2, 2), seal(3, 3), seal(4, 4)];
        let c = [seal(4, 4), seal(5, 5), seal(6, 6)];
        let all: [(RealmId, &[SealPoint]); 3] =
            [(RealmId::Global, &a), (zone(0), &b), (zone(1), &c)];
        assert_eq!(common_seal(&all), Ok(Some((4, 4))));
        // No realm has a seal: a new world.
        let none: [(RealmId, &[SealPoint]); 2] = [(RealmId::Global, &[]), (zone(0), &[])];
        assert_eq!(common_seal(&none), Ok(None));
        // The first seal is in some realms only: the start of the world
        // was cut off, and it starts again as new.
        let first = [seal(0, 0)];
        let cut: [(RealmId, &[SealPoint]); 3] =
            [(RealmId::Global, &[]), (zone(0), &first), (zone(1), &[])];
        assert_eq!(common_seal(&cut), Ok(None));
        let whole: [(RealmId, &[SealPoint]); 2] = [(RealmId::Global, &first), (zone(0), &first)];
        assert_eq!(common_seal(&whole), Ok(Some((0, 0))));
    }

    #[test]
    fn stored_state_without_a_common_seal_is_an_error() {
        let zone = |z| RealmId::Zone(ZoneId(z));
        let message = |realms: &[(RealmId, &[SealPoint])]| match common_seal(realms) {
            Err(ClusterError::Recovery { realm: None, message }) => message,
            other => panic!("no recovery error: {other:?}"),
        };
        let a = [seal(3, 3), seal(4, 4)];
        let b = [seal(7, 7)];
        assert_eq!(
            message(&[(RealmId::Global, &a), (zone(0), &b)]),
            "no seal is in the storage of every realm"
        );
        assert_eq!(
            message(&[(RealmId::Global, &a), (zone(0), &[]), (zone(1), &a), (zone(2), &[])]),
            "some realms have stored state and these have none: zone 0, zone 2"
        );
        let other_frame = [seal(4, 9)];
        assert_eq!(
            message(&[(RealmId::Global, &a), (zone(1), &other_frame)]),
            "seal 4 is at frame 4 in global and at frame 9 in zone 1"
        );
    }
}
