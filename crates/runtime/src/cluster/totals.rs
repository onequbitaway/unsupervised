//! The totals of money and goods of all processes of a cluster.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::WorldTotals;
use unsup_protocol::ItemId;

/// Totals of money, in cents, or of one item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueTotals {
    /// Sum of all balances in all realms.
    pub held: i64,
    /// Created through faucets.
    pub created: i64,
    /// Destroyed through sinks.
    pub destroyed: i64,
    /// In messages that wait in the inbox or the outbox of a realm.
    pub in_flight: i64,
    /// Sent to other worlds minus received from them, summed over all
    /// worlds. It is 0 when no message is on the way between two worlds.
    pub external: i64,
}

impl ValueTotals {
    fn add(&mut self, held: i64, created: i64, destroyed: i64, in_flight: i64, external: i64) {
        self.held = self.held.wrapping_add(held);
        self.created = self.created.wrapping_add(created);
        self.destroyed = self.destroyed.wrapping_add(destroyed);
        self.in_flight = self.in_flight.wrapping_add(in_flight);
        self.external = self.external.wrapping_add(external);
    }

    fn merge(&mut self, other: &ValueTotals) {
        self.add(other.held, other.created, other.destroyed, other.in_flight, other.external);
    }

    /// Checks the totals. `what` names them in the error.
    fn check(&self, what: &str) -> Result<(), String> {
        if self.external != 0 {
            return Err(format!(
                "{what}: the worlds sent {} more to each other than they received",
                self.external
            ));
        }
        let have = self.held.wrapping_add(self.in_flight);
        let must = self.created.wrapping_sub(self.destroyed);
        if have != must {
            return Err(format!(
                "{what}: {have} is held or on the way, but {must} was created and not destroyed"
            ));
        }
        Ok(())
    }
}

/// The totals of money and goods of the union of all worlds of a cluster.
///
/// Each world audits itself and counts what it sent to other worlds. The
/// union balances when, for money and for each item,
/// `held + in_flight == created - destroyed` and `external == 0`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterTotals {
    /// Money.
    pub money: ValueTotals,
    /// Every item that a world has seen.
    pub items: BTreeMap<ItemId, ValueTotals>,
}

impl ClusterTotals {
    /// Adds the totals of one world.
    pub(crate) fn add_world(&mut self, world: &WorldTotals) {
        let m = &world.money;
        self.money.add(m.held, m.minted, m.burned, m.in_flight, m.external);
        for (&item, t) in &world.items {
            self.items.entry(item).or_default().add(
                t.held,
                t.produced,
                t.consumed,
                t.in_flight,
                t.external,
            );
        }
    }

    /// Adds the totals of another process.
    pub(crate) fn merge(&mut self, other: &ClusterTotals) {
        self.money.merge(&other.money);
        for (&item, totals) in &other.items {
            self.items.entry(item).or_default().merge(totals);
        }
    }

    /// Checks that nothing was created or lost outside faucets and sinks,
    /// also not between two processes.
    pub(crate) fn check(&self) -> Result<(), String> {
        self.money.check("money")?;
        for (item, totals) in &self.items {
            totals.check(&format!("item {item}"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use unsup_kernel::{WorldItem, WorldMoney};

    use super::*;

    fn world(held: i64, minted: i64, external: i64) -> WorldTotals {
        let mut totals = WorldTotals {
            money: WorldMoney { held, minted, burned: 0, in_flight: 0, external },
            ..WorldTotals::default()
        };
        totals.items.insert(
            ItemId(4),
            WorldItem { held, produced: minted, consumed: 0, in_flight: 0, external },
        );
        totals
    }

    #[test]
    fn the_union_balances_when_what_one_world_sent_the_other_received() {
        let mut union = ClusterTotals::default();
        // The first world made 100 and sent 30 away. The second took them.
        union.add_world(&world(70, 100, 30));
        assert!(union.check().unwrap_err().contains("30 more"));
        let mut other = ClusterTotals::default();
        other.add_world(&world(30, 0, -30));
        union.merge(&other);
        assert_eq!(union.check(), Ok(()));
        assert_eq!((union.money.held, union.money.created, union.money.external), (100, 100, 0));
        assert_eq!(union.items[&ItemId(4)].held, 100);
    }

    #[test]
    fn value_that_is_lost_between_two_worlds_is_found() {
        let mut union = ClusterTotals::default();
        union.add_world(&world(70, 100, 30));
        // The receiver counts the arrival but holds nothing.
        union.add_world(&world(0, 0, -30));
        let error = union.check().unwrap_err();
        assert!(error.contains("money: 70 is held or on the way, but 100"), "{error}");
    }
}
