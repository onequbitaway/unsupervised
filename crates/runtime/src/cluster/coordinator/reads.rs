//! Calls that do not change the world: observations, views and the
//! questions about the state of all worlds.

use unsup_protocol::{
    Observation, ObserveOpts, RealmId, ResidentId, SimError, ViewQuery, WorldView, ZoneId,
};

use super::super::error::ClusterError;
use super::super::lookup::Where;
use super::super::proto::{FromWorker, Inspect, Inspection, Peeked, ToWorker};
use super::super::session::inspect;
use super::super::totals::ClusterTotals;
use super::core::{Core, unexpected};

/// A fatal failure of the cluster, or the answer for the caller.
type Answer<T> = Result<Result<T, SimError>, ClusterError>;

impl Core {
    /// Returns the zone of a living resident.
    pub(crate) fn zone_of(&self, resident: ResidentId) -> Option<ZoneId> {
        match self.lookup.find(&self.global.world, resident) {
            Where::Alive(zone) => Some(zone),
            Where::Gone(_) | Where::Unknown => None,
        }
    }

    /// Builds what a resident can see now, in the process that hosts it.
    pub(crate) fn observe(
        &mut self,
        resident: ResidentId,
        opts: ObserveOpts,
    ) -> Answer<Observation> {
        let Some(zone) = self.zone_of(resident) else {
            return Ok(Err(SimError::UnknownResident));
        };
        let slot = self.slot(zone)?;
        match self.ask(slot, &ToWorker::Observe { zone, resident, opts })? {
            FromWorker::Observed(answer) => Ok(answer),
            other => Err(unexpected(slot, "Observed", &other)),
        }
    }

    /// Asks the process of a resident for the start of a `wait` call.
    pub(crate) fn peek(
        &mut self,
        resident: ResidentId,
        since: Option<u64>,
        now: bool,
    ) -> Answer<Peeked> {
        let Some(zone) = self.zone_of(resident) else {
            return Ok(Err(SimError::UnknownResident));
        };
        let slot = self.slot(zone)?;
        match self.ask(slot, &ToWorker::Peek { zone, resident, since, now })? {
            FromWorker::Peeked(answer) => Ok(answer),
            other => Err(unexpected(slot, "Peeked", &other)),
        }
    }

    /// Builds the observations of waiting calls: each resident with its
    /// notice cursor. One question goes to each worker that hosts one of
    /// the residents. The answers are in the order of `items`.
    pub(crate) fn observe_many(
        &mut self,
        items: &[(ResidentId, u64)],
    ) -> Result<Vec<Result<Observation, SimError>>, ClusterError> {
        let mut answers: Vec<Result<Observation, SimError>> =
            items.iter().map(|_| Err(SimError::UnknownResident)).collect();
        // Per worker: the places of its residents in `items`, and the
        // question for it.
        let mut places: Vec<Vec<usize>> = self.links.iter().map(|_| Vec::new()).collect();
        let mut questions: Vec<Vec<(ZoneId, ResidentId, u64)>> =
            self.links.iter().map(|_| Vec::new()).collect();
        for (at, &(resident, since)) in items.iter().enumerate() {
            if let Some(zone) = self.zone_of(resident) {
                let slot = self.slot(zone)?;
                places[slot].push(at);
                questions[slot].push((zone, resident, since));
            }
        }
        for (slot, (places, items)) in places.into_iter().zip(questions).enumerate() {
            if items.is_empty() {
                continue;
            }
            let observed = match self.ask(slot, &ToWorker::ObserveMany { items })? {
                FromWorker::ObservedMany(observed) if observed.len() == places.len() => observed,
                other => return Err(unexpected(slot, "ObservedMany", &other)),
            };
            for (at, answer) in places.into_iter().zip(observed) {
                answers[at] = answer;
            }
        }
        Ok(answers)
    }

    /// Answers a question of a viewer.
    pub(crate) fn view(&mut self, query: &ViewQuery) -> Answer<WorldView> {
        match query {
            ViewQuery::Layout => {
                // The world of the global realm knows the layout. The
                // workers report the number of residents of each zone
                // with every change.
                let mut view = match self.global.world.view(query) {
                    Ok(view) => view,
                    Err(error) => return Ok(Err(error)),
                };
                if let WorldView::Layout { zones, .. } = &mut view {
                    for zone in zones {
                        zone.residents =
                            self.residents.get(zone.id.0 as usize).copied().unwrap_or(0);
                    }
                }
                Ok(Ok(view))
            }
            ViewQuery::Zone { zone } => {
                let zone = *zone;
                if self.global.world.layout().zone(zone).is_none() {
                    return Ok(Err(SimError::Invalid));
                }
                let slot = self.slot(zone)?;
                match self.ask(slot, &ToWorker::View { zone })? {
                    FromWorker::Viewed(answer) => Ok(answer),
                    other => Err(unexpected(slot, "Viewed", &other)),
                }
            }
        }
    }

    /// Asks every process the same question about its worlds. The answer
    /// of the global realm comes first, then those of the workers by slot.
    fn inspect_all(&mut self, what: Inspect) -> Result<Vec<Inspection>, ClusterError> {
        let mut answers = vec![inspect(std::iter::once(&self.global), what)];
        for slot in 0..self.links.len() {
            match self.ask(slot, &ToWorker::Inspect(what))? {
                FromWorker::Inspected(answer) => answers.push(answer),
                other => return Err(unexpected(slot, "Inspected", &other)),
            }
        }
        Ok(answers)
    }

    /// Returns `World::state_hash` of the world of every realm: the global
    /// realm first, then the zones in zone order.
    pub(crate) fn hashes(&mut self) -> Result<Vec<(RealmId, [u8; 32])>, ClusterError> {
        let mut hashes = Vec::with_capacity(self.zones() + 1);
        for answer in self.inspect_all(Inspect::Hashes)? {
            if let Inspection::Hashes(list) = answer {
                hashes.extend(list.into_iter().map(|(realm, hash)| (realm, hash.0)));
            }
        }
        hashes.sort_by_key(|&(realm, _)| realm);
        self.complete(hashes.len())?;
        Ok(hashes)
    }

    /// Returns `World::snapshot` of the world of every realm: the global
    /// realm first, then the zones in zone order.
    pub(crate) fn snapshots(&mut self) -> Result<Vec<(RealmId, Vec<u8>)>, ClusterError> {
        let mut snapshots = Vec::with_capacity(self.zones() + 1);
        for answer in self.inspect_all(Inspect::Snapshots)? {
            if let Inspection::Snapshots(list) = answer {
                snapshots.extend(list.into_iter().map(|(realm, blob)| (realm, blob.0)));
            }
        }
        snapshots.sort_by_key(|&(realm, _)| realm);
        self.complete(snapshots.len())?;
        Ok(snapshots)
    }

    /// Checks that an answer covers every realm.
    fn complete(&self, realms: usize) -> Result<(), ClusterError> {
        if realms == self.zones() + 1 {
            return Ok(());
        }
        Err(ClusterError::Protocol(format!(
            "the processes answered for {realms} realms, the world has {}",
            self.zones() + 1
        )))
    }

    /// Audits every world and the union of all of them.
    pub(crate) fn audit(&mut self) -> Result<Result<ClusterTotals, ClusterError>, ClusterError> {
        let mut union = ClusterTotals::default();
        for answer in self.inspect_all(Inspect::Audit)? {
            match answer {
                Inspection::Audit(Ok(totals)) => union.merge(&totals),
                Inspection::Audit(Err(message)) => return Ok(Err(ClusterError::Audit(message))),
                Inspection::Hashes(_) | Inspection::Snapshots(_) => {}
            }
        }
        Ok(union.check().map(|()| union).map_err(ClusterError::Audit))
    }

    /// Returns the messages that the kernel dropped in all worlds.
    pub(crate) fn dropped_messages(&self) -> u64 {
        self.global.world.dropped_messages() + self.dropped.iter().sum::<u64>()
    }
}
