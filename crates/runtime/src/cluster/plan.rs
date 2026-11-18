//! The router of the cluster: which zone gets which message, in which
//! order.
//!
//! One process routes at the end of a frame like this (`World::apply` in
//! the kernel): residents that left a zone arrive in their new zone, in
//! the order of the zones that they left. Then every message goes to the
//! inbox of its target: first the messages of the global realm, then those
//! of the zones in zone order, each realm in its sending order. A message
//! for a resident goes to the zone in which the resident is after the
//! arrivals.
//!
//! A [`Plan`] gives every zone the same arrivals and the same messages in
//! the same order, whatever process hosts the zone.

use unsup_kernel::Dest;
use unsup_protocol::{RealmId, ResidentId, ZoneId};

use super::lookup::Where;
use super::proto::{Routed, ZoneBatch};
use super::wire::Blob;

/// What every zone gets at one routing point.
pub(crate) struct Plan {
    /// One batch per zone of the world, by zone number.
    batches: Vec<ZoneBatch>,
    /// Position of the next message in the routing order.
    next: u32,
    /// Residents that changed zone.
    pub migrations: u64,
    /// Messages that were routed. A message for all zones counts once per
    /// zone.
    pub messages: u64,
}

/// A message has no target that the router can find.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PlanError {
    /// The message is for a zone that the world does not have.
    NoZone(ZoneId),
    /// The message is for a resident that the directory never saw.
    NoResident(ResidentId),
    /// The message is for the global realm. The coordinator gives those to
    /// the global realm itself.
    Global,
}

impl std::fmt::Display for PlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PlanError::NoZone(zone) => {
                write!(f, "a message is for zone {zone}, which is not in the world")
            }
            PlanError::NoResident(resident) => write!(
                f,
                "a message is for resident {resident}, which is not in the directory of the global realm"
            ),
            PlanError::Global => {
                f.write_str("a message for the global realm reached the zone router")
            }
        }
    }
}

impl Plan {
    /// Makes an empty plan for a world with `zones` zones.
    pub(crate) fn new(zones: usize) -> Plan {
        let batches = (0..zones)
            .map(|zone| ZoneBatch { zone: ZoneId(zone as u32), ..ZoneBatch::default() })
            .collect();
        Plan { batches, next: 0, migrations: 0, messages: 0 }
    }

    fn batch(&mut self, zone: ZoneId) -> Result<&mut ZoneBatch, PlanError> {
        self.batches.get_mut(zone.0 as usize).ok_or(PlanError::NoZone(zone))
    }

    /// Adds a resident that arrives in `zone`. Call it in the order of the
    /// zones that the residents left.
    pub(crate) fn arrival(&mut self, zone: ZoneId, bytes: Vec<u8>) -> Result<(), PlanError> {
        self.batch(zone)?.arrivals.push(Blob(bytes));
        self.migrations += 1;
        Ok(())
    }

    fn push(&mut self, zone: ZoneId, probe: bool, bytes: Vec<u8>) -> Result<(), PlanError> {
        let index = self.next;
        self.batch(zone)?.messages.push(Routed { index, probe, bytes: Blob(bytes) });
        self.messages += 1;
        Ok(())
    }

    /// Adds one message that `from` sent. Call it in the routing order:
    /// the messages of the global realm first, then those of the zones in
    /// zone order, each in sending order. `find` is the directory.
    pub(crate) fn message(
        &mut self,
        from: RealmId,
        to: Dest,
        bytes: Vec<u8>,
        find: impl Fn(ResidentId) -> Where,
    ) -> Result<(), PlanError> {
        let result = match to {
            Dest::Global => Err(PlanError::Global),
            Dest::Zone(zone) => self.push(zone, false, bytes),
            Dest::Resident(resident) => match find(resident) {
                Where::Alive(zone) => self.push(zone, false, bytes),
                // The world of the zone in which the resident left the
                // world has never seen it go to another zone. So it
                // applies the rule of the kernel for a resident that is
                // gone: a wire goes to the unclaimed account of the global
                // realm, everything else is dropped.
                Where::Gone(zone) => self.push(zone, true, bytes),
                Where::Unknown => Err(PlanError::NoResident(resident)),
            },
            Dest::AllZones => {
                // The world of a zone gives its own zone the message by
                // itself.
                let sender = from.zone();
                let zones = self.batches.len() as u32;
                (0..zones)
                    .map(ZoneId)
                    .filter(|&zone| Some(zone) != sender)
                    .try_for_each(|zone| self.push(zone, false, bytes.clone()))
            }
        };
        // One message has one place in the order, also when it goes to
        // many zones.
        self.next += 1;
        result
    }

    /// Returns the batches of the zones in `zones` that hold something.
    pub(crate) fn take(&mut self, zones: &[ZoneId]) -> Vec<ZoneBatch> {
        let mut taken = Vec::new();
        for &zone in zones {
            if let Some(batch) = self.batches.get_mut(zone.0 as usize)
                && !batch.is_empty()
            {
                taken.push(std::mem::replace(batch, ZoneBatch { zone, ..ZoneBatch::default() }));
            }
        }
        taken
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(resident: ResidentId) -> Where {
        match resident.0 {
            1 => Where::Alive(ZoneId(2)),
            2 => Where::Gone(ZoneId(0)),
            _ => Where::Unknown,
        }
    }

    fn summary(batch: &ZoneBatch) -> (u32, Vec<u8>, Vec<(u32, bool, u8)>) {
        (
            batch.zone.0,
            batch.arrivals.iter().map(|blob| blob.0[0]).collect(),
            batch.messages.iter().map(|m| (m.index, m.probe, m.bytes.0[0])).collect(),
        )
    }

    #[test]
    fn every_zone_gets_arrivals_first_and_messages_in_the_routing_order() {
        let mut plan = Plan::new(3);
        let zone = |z| RealmId::Zone(ZoneId(z));
        plan.arrival(ZoneId(2), vec![90]).unwrap();
        plan.arrival(ZoneId(0), vec![91]).unwrap();
        plan.arrival(ZoneId(2), vec![92]).unwrap();
        // The global realm first, then zone 0, then zone 1.
        plan.message(RealmId::Global, Dest::Resident(ResidentId(1)), vec![10], find).unwrap();
        plan.message(RealmId::Global, Dest::AllZones, vec![11], find).unwrap();
        plan.message(zone(0), Dest::Zone(ZoneId(2)), vec![12], find).unwrap();
        plan.message(zone(0), Dest::Resident(ResidentId(2)), vec![13], find).unwrap();
        plan.message(zone(1), Dest::AllZones, vec![14], find).unwrap();
        plan.message(zone(1), Dest::Zone(ZoneId(1)), vec![15], find).unwrap();
        assert_eq!((plan.migrations, plan.messages), (3, 9));

        let batches = plan.take(&[ZoneId(0), ZoneId(1), ZoneId(2)]);
        let got: Vec<_> = batches.iter().map(summary).collect();
        assert_eq!(
            got,
            [
                // Zone 0: the message for the resident that is gone is a probe.
                (0, vec![91], vec![(1, false, 11), (3, true, 13), (4, false, 14)]),
                // Zone 1 sent message 14 to all zones: the router does not
                // give it back to zone 1.
                (1, vec![], vec![(1, false, 11), (5, false, 15)]),
                (
                    2,
                    vec![90, 92],
                    vec![(0, false, 10), (1, false, 11), (2, false, 12), (4, false, 14)]
                ),
            ]
        );
        // What was taken is gone.
        assert_eq!(plan.take(&[ZoneId(0), ZoneId(1), ZoneId(2)]).len(), 0);
    }

    #[test]
    fn a_message_without_a_target_is_an_error() {
        let mut plan = Plan::new(2);
        let from = RealmId::Zone(ZoneId(0));
        assert_eq!(
            plan.message(from, Dest::Zone(ZoneId(2)), vec![1], find),
            Err(PlanError::NoZone(ZoneId(2)))
        );
        assert_eq!(
            plan.message(from, Dest::Resident(ResidentId(9)), vec![1], find),
            Err(PlanError::NoResident(ResidentId(9)))
        );
        assert_eq!(plan.message(from, Dest::Global, vec![1], find), Err(PlanError::Global));
        assert_eq!(plan.arrival(ZoneId(5), vec![1]), Err(PlanError::NoZone(ZoneId(5))));
        assert_eq!(plan.take(&[ZoneId(0), ZoneId(1)]).len(), 0);
    }
}
