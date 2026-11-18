//! The nodes of a zone: what is left to gather at each farm, mine and
//! forest.
//!
//! A node gets units back as time passes. The stock is brought up to date
//! only when somebody looks at it, so no system runs each tick.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Resource, SiteId};

use crate::rules::KindRule;

/// Game minutes in one hour.
const HOUR: u64 = 60;

/// The stock of one node at a point in time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Node {
    /// Units that were left at tick `at`.
    pub(crate) stock: i64,
    /// The tick up to which the regeneration is counted.
    pub(crate) at: u64,
    /// Sixtieths of a unit that the node has got back and that do not make
    /// a whole unit yet.
    pub(crate) part: u64,
}

impl Node {
    /// Returns the node at tick `now`: the stock with the units that it got
    /// back, at most `node_max`. The part of a unit that is left stays for
    /// the next call.
    fn advance(self, now: u64, rule: &KindRule) -> Node {
        let regen = u64::try_from(rule.node_regen_per_hour).unwrap_or(0);
        if self.stock >= rule.node_max || regen == 0 {
            return Node { stock: self.stock.min(rule.node_max), at: now, part: 0 };
        }
        let elapsed = now.saturating_sub(self.at);
        let sixtieths = elapsed.saturating_mul(regen).saturating_add(self.part);
        let gained = i64::try_from(sixtieths / HOUR).unwrap_or(i64::MAX);
        let stock = self.stock.saturating_add(gained);
        if stock >= rule.node_max {
            return Node { stock: rule.node_max, at: now, part: 0 };
        }
        Node { stock, at: now, part: sixtieths % HOUR }
    }
}

/// The nodes of one zone, by site. A site without an entry has a full node.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Nodes {
    nodes: BTreeMap<SiteId, Node>,
}

impl Resource for Nodes {
    const NAME: &'static str = "economy.nodes";
}

impl Nodes {
    /// Returns the stock of a node at tick `now`. It does not change the
    /// state.
    pub(crate) fn stock(&self, site: SiteId, now: u64, rule: &KindRule) -> i64 {
        match self.nodes.get(&site) {
            Some(node) => node.advance(now, rule).stock,
            None => rule.node_max,
        }
    }

    /// Takes up to `wanted` units from a node at tick `now` and returns how
    /// many it gave.
    pub(crate) fn take(&mut self, site: SiteId, now: u64, rule: &KindRule, wanted: i64) -> i64 {
        let node = match self.nodes.get(&site) {
            Some(node) => node.advance(now, rule),
            None => Node { stock: rule.node_max, at: now, part: 0 },
        };
        let taken = wanted.clamp(0, node.stock.max(0));
        self.nodes.insert(site, Node { stock: node.stock - taken, ..node });
        taken
    }
}

#[cfg(test)]
mod tests {
    use unsup_kernel::AttrId;

    use super::*;
    use crate::rules::Skill;

    fn rule(max: i64, regen: i64) -> KindRule {
        KindRule {
            skill: Skill { attr: AttrId(0), name: "farming".into() },
            business: false,
            retail: false,
            lettable: false,
            units: 1,
            public_works: false,
            depot: false,
            gather: Vec::new(),
            node_max: max,
            node_regen_per_hour: regen,
        }
    }

    #[test]
    fn a_new_node_is_full_and_gives_what_it_has() {
        let mut nodes = Nodes::default();
        let rule = rule(100, 30);
        assert_eq!(nodes.stock(SiteId(1), 0, &rule), 100);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 40), 40);
        assert_eq!(nodes.stock(SiteId(1), 0, &rule), 60);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 500), 60);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 5), 0);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, -5), 0);
    }

    #[test]
    fn a_node_regenerates_without_losing_part_hours() {
        let mut nodes = Nodes::default();
        let rule = rule(100, 30);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 100), 100);
        // 30 per hour is one unit every two minutes.
        assert_eq!(nodes.stock(SiteId(1), 1, &rule), 0);
        assert_eq!(nodes.stock(SiteId(1), 2, &rule), 1);
        assert_eq!(nodes.stock(SiteId(1), 60, &rule), 30);
        // Taking at minute 3 keeps the odd minute for later.
        assert_eq!(nodes.take(SiteId(1), 3, &rule, 1), 1);
        assert_eq!(nodes.stock(SiteId(1), 3, &rule), 0);
        assert_eq!(nodes.stock(SiteId(1), 4, &rule), 1);
        // It never grows past the greatest stock.
        assert_eq!(nodes.stock(SiteId(1), 100_000, &rule), 100);
    }

    #[test]
    fn a_fast_node_gives_no_unit_twice() {
        let mut nodes = Nodes::default();
        // 90 per hour is one and a half units each minute.
        let rule = rule(1_000, 90);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 1_000), 1_000);
        assert_eq!(nodes.take(SiteId(1), 1, &rule, 10), 1);
        assert_eq!(nodes.take(SiteId(1), 1, &rule, 10), 0);
        assert_eq!(nodes.take(SiteId(1), 2, &rule, 10), 2);
        assert_eq!(nodes.stock(SiteId(1), 62, &rule), 90);
    }

    #[test]
    fn a_node_without_regeneration_stays_empty() {
        let mut nodes = Nodes::default();
        let rule = rule(10, 0);
        assert_eq!(nodes.take(SiteId(1), 0, &rule, 10), 10);
        assert_eq!(nodes.stock(SiteId(1), 10_000, &rule), 0);
    }
}
