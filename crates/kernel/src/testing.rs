//! Testing support: a small world for module tests.
//!
//! [`TestWorld`] wraps a [`World`] with one city of 2 by 2 zones and gives
//! short helpers to spawn residents, act, step frames, read attributes and
//! the ledger, and check conservation and determinism. Every module uses it
//! for its tests.
//!
//! The helpers panic with a message when something that the test takes for
//! granted is not true (an unknown resident, an unknown attribute name, a
//! rejected action in `act_ok`). A test wants exactly that.

use serde_json::Value;
use unsup_protocol::{
    ActionAck, ActionError, ActionReport, ActionRequest, ActionStatus, AdminCommand, AdminReply,
    FrameOutput, Money, Notice, Observation, ObserveOpts, OwnerId, Pos, RealmId, ResidentId,
    SpawnSpec, ZoneId,
};

use crate::config::WorldConfig;
use crate::ids::Idx;
use crate::kernel_plugin::GRANT_FAUCET;
use crate::ledger::{Holder, UNCLAIMED};
use crate::plugin::Plugin;
use crate::realm::Realm;
use crate::world::{World, WorldTotals};

/// A small world for tests.
pub struct TestWorld<'a> {
    /// The world. Use it directly for anything that the helpers do not
    /// cover.
    pub world: World,
    plugins: Vec<&'a dyn Plugin>,
}

/// Writes bytes as lower-case hexadecimal.
fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

impl<'a> TestWorld<'a> {
    /// Makes a world with the default configuration: one city of 2 by 2
    /// zones of 64 tiles, seed 0.
    #[must_use]
    pub fn new(plugins: &[&'a dyn Plugin]) -> TestWorld<'a> {
        TestWorld::with_config(plugins, WorldConfig::default())
    }

    /// Makes a world with the default configuration and a seed.
    #[must_use]
    pub fn with_seed(plugins: &[&'a dyn Plugin], seed: u64) -> TestWorld<'a> {
        TestWorld::with_config(plugins, WorldConfig::with_seed(seed))
    }

    /// Makes a world with a configuration.
    #[must_use]
    pub fn with_config(plugins: &[&'a dyn Plugin], config: WorldConfig) -> TestWorld<'a> {
        TestWorld { world: World::new(plugins, config), plugins: plugins.to_vec() }
    }

    /// Makes a test world from a snapshot, with a set of plugins that may
    /// differ from the one that made the snapshot.
    ///
    /// # Panics
    ///
    /// Panics when the snapshot cannot be restored.
    #[must_use]
    pub fn restore(plugins: &[&'a dyn Plugin], bytes: &[u8]) -> TestWorld<'a> {
        match World::restore(plugins, bytes) {
            Ok(world) => TestWorld { world, plugins: plugins.to_vec() },
            Err(e) => panic!("{e}"),
        }
    }

    // ---- Residents ------------------------------------------------------------

    /// Spawns a resident of owner 1 at the spawn point of the first city.
    pub fn spawn(&mut self, name: &str) -> ResidentId {
        let spec = SpawnSpec {
            owner: OwnerId(1),
            name: name.to_string(),
            model: None,
            city: Some(unsup_protocol::CityId(0)),
        };
        match self.world.spawn(spec) {
            Ok(id) => id,
            Err(e) => panic!("cannot spawn `{name}`: {e}"),
        }
    }

    /// Spawns a resident and puts it at a position, in whatever zone that
    /// is.
    pub fn spawn_at(&mut self, name: &str, pos: Pos) -> ResidentId {
        let id = self.spawn(name);
        self.set_pos(id, pos);
        id
    }

    /// Moves a resident to a position, in whatever zone that is. The
    /// resident is in the new zone when the call returns.
    pub fn set_pos(&mut self, r: ResidentId, pos: Pos) {
        self.with_resident(r, |realm, idx| {
            if let Err(e) = realm.set_pos(idx, pos) {
                panic!("cannot move resident {r} to {pos}: {e}");
            }
        });
        self.world.settle();
    }

    /// Removes a resident from the world. Its holdings go to the unclaimed
    /// account of the global realm.
    pub fn despawn(&mut self, r: ResidentId) {
        self.with_resident(r, |realm, idx| realm.despawn(idx, UNCLAIMED, "test"));
        self.world.settle();
    }

    /// Runs `f` with the zone realm and the dense index of a resident.
    ///
    /// # Panics
    ///
    /// Panics when the resident is not in the world.
    pub fn with_resident<R>(&self, r: ResidentId, f: impl FnOnce(&Realm, Idx) -> R) -> R {
        let zone = match self.world.locate(r) {
            Some(zone) => zone,
            None => panic!("resident {r} is not in the world"),
        };
        let realm = self.zone(zone);
        match realm.idx_of(r) {
            Some(idx) => f(realm, idx),
            None => panic!("resident {r} is not in zone {zone}"),
        }
    }

    /// Returns the realm of a zone.
    ///
    /// # Panics
    ///
    /// Panics when the world has no such zone.
    #[must_use]
    pub fn zone(&self, zone: ZoneId) -> &Realm {
        match self.world.realm(RealmId::Zone(zone)) {
            Some(realm) => realm,
            None => panic!("the world has no zone {zone}"),
        }
    }

    /// Returns the global realm.
    #[must_use]
    pub fn global(&self) -> &Realm {
        match self.world.realm(RealmId::Global) {
            Some(realm) => realm,
            None => panic!("the world does not host the global realm"),
        }
    }

    // ---- Actions --------------------------------------------------------------

    /// Sends an action request of a resident and returns the
    /// acknowledgement.
    pub fn act(&mut self, r: ResidentId, verb: &str, args: Value) -> ActionAck {
        self.act_request(r, ActionRequest::new(verb, args))
    }

    /// Sends an action request with every field as given.
    pub fn act_request(&mut self, r: ResidentId, request: ActionRequest) -> ActionAck {
        match self.world.act(r, request) {
            Ok(ack) => ack,
            Err(e) => panic!("resident {r} cannot act: {e}"),
        }
    }

    /// Sends an action request that the test expects to be accepted, and
    /// returns its status.
    ///
    /// # Panics
    ///
    /// Panics when the request is rejected.
    pub fn act_ok(&mut self, r: ResidentId, verb: &str, args: Value) -> ActionStatus {
        match self.act(r, verb, args).status {
            ActionStatus::Rejected { error } => {
                panic!("`{verb}` of resident {r} was rejected: {error}")
            }
            status => status,
        }
    }

    /// Sends an action request that the test expects to be rejected, and
    /// returns the error.
    ///
    /// # Panics
    ///
    /// Panics when the request is accepted.
    pub fn act_err(&mut self, r: ResidentId, verb: &str, args: Value) -> ActionError {
        match self.act(r, verb, args).status {
            ActionStatus::Rejected { error } => error,
            status => panic!("`{verb}` of resident {r} was accepted: {status:?}"),
        }
    }

    // ---- Time -----------------------------------------------------------------

    /// Runs frames and returns what each one changed.
    pub fn step(&mut self, frames: u32) -> Vec<FrameOutput> {
        (0..frames).map(|_| self.world.step()).collect()
    }

    /// Runs frames until a resident is idle with an empty queue, at most
    /// `max_frames`. Returns how many frames ran.
    ///
    /// # Panics
    ///
    /// Panics when the resident is still busy after `max_frames`.
    pub fn run_until_idle(&mut self, r: ResidentId, max_frames: u32) -> u32 {
        for frames in 0..=max_frames {
            let idle = self.with_resident(r, |realm, idx| {
                realm.activity(idx).is_none() && realm.queue_len(idx) == 0
            });
            if idle {
                return frames;
            }
            if frames < max_frames {
                self.world.step();
            }
        }
        panic!("resident {r} is still busy after {max_frames} frames");
    }

    // ---- Reads ----------------------------------------------------------------

    /// Returns the zone of a resident.
    #[must_use]
    pub fn zone_of(&self, r: ResidentId) -> ZoneId {
        match self.world.locate(r) {
            Some(zone) => zone,
            None => panic!("resident {r} is not in the world"),
        }
    }

    /// Returns true when the resident is in the world.
    #[must_use]
    pub fn is_alive(&self, r: ResidentId) -> bool {
        self.world.locate(r).is_some()
    }

    /// Returns the position of a resident.
    #[must_use]
    pub fn pos(&self, r: ResidentId) -> Pos {
        self.with_resident(r, |realm, idx| realm.pos(idx))
    }

    /// Returns the verb of the running action of a resident.
    #[must_use]
    pub fn activity(&self, r: ResidentId) -> Option<&'static str> {
        self.with_resident(r, |realm, idx| realm.activity(idx).map(|a| a.verb))
    }

    /// Returns an attribute of a resident, by name.
    #[must_use]
    pub fn attr(&self, r: ResidentId, name: &str) -> i64 {
        let attr = match self.world.blueprint().attr_id(name) {
            Some(attr) => attr,
            None => panic!("no attribute is named `{name}`"),
        };
        self.with_resident(r, |realm, idx| realm.attr(idx, attr))
    }

    /// Sets an attribute of a resident, by name. Returns the clamped value.
    pub fn set_attr(&mut self, r: ResidentId, name: &str, value: i64) -> i64 {
        let attr = match self.world.blueprint().attr_id(name) {
            Some(attr) => attr,
            None => panic!("no attribute is named `{name}`"),
        };
        self.with_resident(r, |realm, idx| realm.attr_set(idx, attr, value))
    }

    /// Returns true when a resident has a condition, by name.
    #[must_use]
    pub fn has_condition(&self, r: ResidentId, name: &str) -> bool {
        let cond = match self.world.blueprint().condition_id(name) {
            Some(cond) => cond,
            None => panic!("no condition is named `{name}`"),
        };
        self.with_resident(r, |realm, idx| realm.has_condition(idx, cond))
    }

    /// Gives a resident a condition, by name. With `ticks`, the condition
    /// ends after that many ticks of the zone.
    pub fn set_condition(&mut self, r: ResidentId, name: &str, ticks: Option<u64>) {
        let cond = match self.world.blueprint().condition_id(name) {
            Some(cond) => cond,
            None => panic!("no condition is named `{name}`"),
        };
        self.with_resident(r, |realm, idx| {
            realm.set_condition(idx, cond, ticks.map(|t| realm.tick() + t));
        });
    }

    /// Returns the current value of a parameter in the global realm, by
    /// name.
    #[must_use]
    pub fn param(&self, name: &str) -> i64 {
        match self.world.blueprint().param_id(name) {
            Some(param) => self.global().param(param),
            None => panic!("no parameter is named `{name}`"),
        }
    }

    /// Changes the national value of a parameter, by name. Zones see the
    /// change in the next frame.
    pub fn set_param(&mut self, name: &str, value: i64) {
        let reply =
            self.world.admin(AdminCommand::SetParam { name: name.to_string(), value, city: None });
        if let AdminReply::Error { message } = reply {
            panic!("cannot set parameter `{name}`: {message}");
        }
    }

    /// Builds the brief observation of a resident with all notices.
    #[must_use]
    pub fn observe(&self, r: ResidentId) -> Observation {
        self.observe_with(r, &ObserveOpts::default())
    }

    /// Builds the observation of a resident with options.
    #[must_use]
    pub fn observe_with(&self, r: ResidentId, opts: &ObserveOpts) -> Observation {
        match self.world.observe(r, opts) {
            Ok(observation) => observation,
            Err(e) => panic!("cannot observe resident {r}: {e}"),
        }
    }

    /// Returns the notices in the inbox of a resident, oldest first.
    #[must_use]
    pub fn notices(&self, r: ResidentId) -> Vec<Notice> {
        self.observe(r).notices
    }

    /// Returns the action reports in the inbox of a resident, oldest first.
    #[must_use]
    pub fn reports(&self, r: ResidentId) -> Vec<ActionReport> {
        let observation = self.observe(r);
        match observation.sections.get("reports") {
            Some(value) => match serde_json::from_value(value.clone()) {
                Ok(reports) => reports,
                Err(e) => panic!("the reports section is not a list of reports: {e}"),
            },
            None => Vec::new(),
        }
    }

    /// Returns the newest action report in the inbox of a resident.
    #[must_use]
    pub fn last_report(&self, r: ResidentId) -> Option<ActionReport> {
        self.reports(r).pop()
    }

    // ---- Ledger ---------------------------------------------------------------

    /// Returns the money of a holder, summed over all realms.
    #[must_use]
    pub fn money(&self, holder: Holder) -> Money {
        self.realms().map(|realm| realm.ledger().money(holder)).sum()
    }

    /// Returns the money of a resident.
    #[must_use]
    pub fn cash(&self, r: ResidentId) -> Money {
        self.money(Holder::Resident(r))
    }

    /// Returns how many units of an item a holder has, summed over all
    /// realms. The item is given by name.
    #[must_use]
    pub fn qty(&self, holder: Holder, item: &str) -> i64 {
        let id = match self.world.blueprint().item_id(item) {
            Some(id) => id,
            None => panic!("no item is named `{item}`"),
        };
        self.realms().map(|realm| realm.ledger().qty(holder, id)).sum()
    }

    /// Mints money for a resident through the faucet `kernel.grant`.
    pub fn give_money(&mut self, r: ResidentId, amount: Money) {
        let faucet = match self.world.blueprint().faucet_id(GRANT_FAUCET) {
            Some(faucet) => faucet,
            None => panic!("the kernel faucet is missing"),
        };
        self.with_resident(r, |realm, _| {
            realm.ledger_mut().mint(faucet, Holder::Resident(r), amount)
        });
    }

    /// Produces units of an item for a resident through the faucet
    /// `kernel.grant`. The item is given by name.
    pub fn give_item(&mut self, r: ResidentId, item: &str, qty: i64) {
        let bp = self.world.blueprint();
        let (Some(faucet), Some(id)) = (bp.faucet_id(GRANT_FAUCET), bp.item_id(item)) else {
            panic!("no item is named `{item}`");
        };
        self.with_resident(r, |realm, _| {
            realm.ledger_mut().produce(faucet, Holder::Resident(r), id, qty)
        });
    }

    fn realms(&self) -> impl Iterator<Item = &Realm> + '_ {
        let zones = self.world.layout().zones().iter().map(|z| RealmId::Zone(z.id));
        std::iter::once(RealmId::Global).chain(zones).filter_map(|id| self.world.realm(id))
    }

    // ---- Invariants -----------------------------------------------------------

    /// Audits the whole world and returns the totals.
    ///
    /// # Panics
    ///
    /// Panics when money or goods were created or lost outside faucets and
    /// sinks.
    pub fn assert_conserved(&self) -> WorldTotals {
        match self.world.audit() {
            Ok(totals) => totals,
            Err(e) => panic!("{e}"),
        }
    }

    /// Returns the state hash as hexadecimal text.
    #[must_use]
    pub fn hash(&self) -> String {
        hex(&self.world.state_hash())
    }

    /// Replaces the world with one that is restored from its own snapshot.
    /// Nothing that a test can see may change.
    ///
    /// # Panics
    ///
    /// Panics when the snapshot cannot be restored or when the restored
    /// world has another state hash.
    pub fn snapshot_restore(&mut self) {
        let before = self.world.state_hash();
        let bytes = self.world.snapshot();
        let restored = match World::restore(&self.plugins, &bytes) {
            Ok(world) => world,
            Err(e) => panic!("{e}"),
        };
        assert_eq!(
            hex(&restored.state_hash()),
            hex(&before),
            "the restored world has another state hash"
        );
        self.world = restored;
    }

    /// Changes the money balance of a holder in one realm without a faucet
    /// or a sink. It exists so that a test can prove that the audit sees a
    /// wrong balance. Nothing else may call it.
    pub fn corrupt_money(&mut self, realm: RealmId, holder: Holder, delta: i64) {
        match self.world.realm(realm) {
            Some(realm) => realm.ledger_mut().corrupt_money(holder, delta),
            None => panic!("the world has no realm {realm}"),
        }
    }

    /// Changes the item balance of a holder in one realm without a faucet
    /// or a sink. See [`TestWorld::corrupt_money`].
    pub fn corrupt_item(&mut self, realm: RealmId, holder: Holder, item: &str, delta: i64) {
        let id = match self.world.blueprint().item_id(item) {
            Some(id) => id,
            None => panic!("no item is named `{item}`"),
        };
        match self.world.realm(realm) {
            Some(realm) => realm.ledger_mut().corrupt_item(holder, id, delta),
            None => panic!("the world has no realm {realm}"),
        }
    }
}

/// Checks that a scenario is deterministic.
///
/// `setup` and `run` are applied to fresh worlds with the same plugins and
/// configuration. The check passes when:
///
/// - two worlds that run `setup` then `run` have the same state hash, and
/// - a world that runs `setup`, is restored from its own snapshot, and then
///   runs `run` has that hash too.
///
/// Returns the hash as hexadecimal text.
///
/// # Panics
///
/// Panics when the hashes differ.
pub fn assert_deterministic<'a>(
    plugins: &[&'a dyn Plugin],
    config: &WorldConfig,
    setup: impl Fn(&mut TestWorld<'a>),
    run: impl Fn(&mut TestWorld<'a>),
) -> String {
    let mut first = TestWorld::with_config(plugins, config.clone());
    setup(&mut first);
    run(&mut first);

    let mut second = TestWorld::with_config(plugins, config.clone());
    setup(&mut second);
    run(&mut second);
    assert_eq!(first.hash(), second.hash(), "two runs of the same scenario differ");

    let mut restored = TestWorld::with_config(plugins, config.clone());
    setup(&mut restored);
    restored.snapshot_restore();
    run(&mut restored);
    assert_eq!(first.hash(), restored.hash(), "a run that was restored from a snapshot differs");
    first.hash()
}
