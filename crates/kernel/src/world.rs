//! The world: all realms, the router between them, and the only door for
//! inputs.
//!
//! A frame is one step of the whole world:
//!
//! 1. Every zone runs one tick. Zones run in parallel; they share nothing,
//!    so the result does not depend on the thread schedule.
//! 2. Messages from zones to the global realm are delivered, ordered by zone
//!    and sequence number, and the global realm runs one tick.
//! 3. Residents that left a zone arrive in their new zone. Then the router
//!    moves every other message to the inbox of its target realm: first the
//!    messages of the global realm, then those of the zones in zone order.
//!
//! So a message from a zone reaches the global realm in the same frame, and
//! every other message is delivered at the start of the next tick of its
//! target.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use unsup_protocol::{
    ActionAck, ActionRequest, AdminCommand, AdminReply, Catalogue, CityId, CityView, FrameOutput,
    ItemId, Observation, ObserveOpts, Pos, RealmId, ResidentId, SimError, SpawnSpec, ViewQuery,
    WorldView, ZoneId,
};

use crate::codec::{self, DecodeError};
use crate::config::{ConfigError, WorldConfig};
use crate::defs::DataError;
use crate::kernel_plugin::KernelPlugin;
use crate::layout::Layout;
use crate::ledger::{AuditError, UNCLAIMED};
use crate::message::{Body, Dest, Envelope, KernelMsg};
use crate::plugin::{Blueprint, NameTables, Plugin, WorldGen};
use crate::realm::{Identity, Realm, RealmState, Transfer};
use crate::spawn::SpawnSites;

/// Greatest length of a resident name and of a model name, in bytes.
const NAME_MAX: usize = 64;

/// Marks the start of a snapshot.
const SNAPSHOT_MAGIC: &str = "unsupervised-snapshot";

/// Version of the snapshot format.
const SNAPSHOT_VERSION: u32 = 1;

/// Something that changes the world. Every change goes through
/// [`World::apply`], so a log of inputs replays the world exactly.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    /// Create a resident. The answer is [`Output::Spawned`].
    Spawn(SpawnSpec),
    /// One action request of a resident. The answer is [`Output::Ack`].
    Act {
        /// The resident that acts.
        resident: ResidentId,
        /// What it asks for.
        request: ActionRequest,
    },
    /// Run one frame. The answer is [`Output::Frame`].
    Step {
        /// Zones that do not tick in this frame (time dilation).
        skip: Vec<ZoneId>,
    },
    /// A command of an administrator. The answer is [`Output::Admin`].
    Admin(AdminCommand),
    /// Messages from realms that another process hosts. The answer is
    /// [`Output::Delivered`].
    Deliver {
        /// The messages, in the order in which the other process sent them.
        messages: Vec<Envelope>,
    },
}

/// The answer to an [`Input`].
#[derive(Clone, Debug)]
pub enum Output {
    /// The id of the new resident.
    Spawned(ResidentId),
    /// The acknowledgement of an action request.
    Ack(ActionAck),
    /// What changed in the frame.
    Frame(FrameOutput),
    /// The answer to an administrator.
    Admin(AdminReply),
    /// The messages were taken.
    Delivered,
    /// The input was refused. Nothing changed.
    Error(SimError),
}

/// A world could not be made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldError {
    /// The configuration is not valid.
    Config(ConfigError),
    /// A data override of the configuration is not valid.
    Data(DataError),
}

impl fmt::Display for WorldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorldError::Config(e) => e.fmt(f),
            WorldError::Data(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for WorldError {}

/// A snapshot could not be restored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// The bytes are not a snapshot, or a part of it cannot be read.
    Decode(DecodeError),
    /// The snapshot has a format version that this build does not read.
    Version {
        /// The version in the snapshot.
        found: u32,
    },
    /// The configuration in the snapshot is not valid.
    Config(ConfigError),
    /// A data override in the snapshot is not valid for these plugins.
    Data(DataError),
    /// The realms in the snapshot do not match its configuration.
    Realms(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SnapshotError::Decode(e) => write!(f, "cannot restore the snapshot: {e}"),
            SnapshotError::Version { found } => write!(
                f,
                "cannot restore the snapshot: it has format version {found}, this build reads version {SNAPSHOT_VERSION}"
            ),
            SnapshotError::Config(e) => write!(f, "cannot restore the snapshot: {e}"),
            SnapshotError::Data(e) => write!(f, "cannot restore the snapshot: {e}"),
            SnapshotError::Realms(message) => write!(f, "cannot restore the snapshot: {message}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Totals of money in the whole world, in cents.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WorldMoney {
    /// Sum of all balances in all hosted realms.
    pub held: i64,
    /// Created through faucets.
    pub minted: i64,
    /// Destroyed through sinks.
    pub burned: i64,
    /// In messages between hosted realms that are not delivered yet.
    pub in_flight: i64,
    /// Sent to realms in other processes, minus received from them.
    pub external: i64,
}

/// Totals of one item in the whole world.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct WorldItem {
    /// Sum of all balances in all hosted realms.
    pub held: i64,
    /// Created through faucets.
    pub produced: i64,
    /// Destroyed through sinks.
    pub consumed: i64,
    /// In messages between hosted realms that are not delivered yet.
    pub in_flight: i64,
    /// Sent to realms in other processes, minus received from them.
    pub external: i64,
}

/// Totals of money and goods in the whole world. For each of them:
/// `held + in_flight + external == created - destroyed`.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct WorldTotals {
    /// Money.
    pub money: WorldMoney,
    /// Every item that the world has ever seen.
    pub items: BTreeMap<ItemId, WorldItem>,
}

/// How long one zone took to tick in the last frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ZoneTiming {
    /// The zone.
    pub zone: ZoneId,
    /// Time of the tick in the unit of the clock that the host gave
    /// (`World::set_clock`), normally nanoseconds. 0 for a skipped zone.
    pub elapsed: u64,
}

/// A clock that the host gives to the world to measure zone ticks. The
/// kernel never reads the wall clock itself, and the measurements never
/// change simulation state.
pub type TickClock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// An estimate of the memory that residents use.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct MemoryEstimate {
    /// Number of residents in hosted zones.
    pub residents: usize,
    /// Bytes of all resident tables: columns, indexes, agendas, inboxes and
    /// ledger entries. Heap data inside module components is not counted.
    pub bytes: usize,
}

impl MemoryEstimate {
    /// Returns the bytes per resident, or 0 when there is no resident.
    #[must_use]
    pub fn per_resident(&self) -> usize {
        self.bytes.checked_div(self.residents).unwrap_or(0)
    }
}

/// The world-level part of a snapshot, as it is read.
#[derive(Deserialize)]
struct Snapshot {
    magic: String,
    version: u32,
    config: WorldConfig,
    names: NameTables,
    frame: u64,
    next_resident: u64,
    away: BTreeSet<ResidentId>,
    external: Vec<Envelope>,
    external_money: i64,
    #[serde(with = "crate::codec::pairs")]
    external_items: BTreeMap<ItemId, i64>,
    realms: Vec<RealmBlob>,
}

/// The world-level part of a snapshot, as it is written.
#[derive(Serialize)]
struct SnapshotRef<'a> {
    magic: &'a str,
    version: u32,
    config: &'a WorldConfig,
    names: NameTables,
    frame: u64,
    next_resident: u64,
    away: &'a BTreeSet<ResidentId>,
    external: &'a [Envelope],
    external_money: i64,
    #[serde(with = "crate::codec::pairs")]
    external_items: &'a BTreeMap<ItemId, i64>,
    realms: Vec<RealmBlob>,
}

/// The state of one realm inside a snapshot.
#[derive(Serialize, Deserialize)]
struct RealmBlob {
    id: RealmId,
    #[serde(with = "crate::codec::bytes")]
    data: Vec<u8>,
}

/// A persistent simulated world: the zone realms, the global realm and the
/// router between them.
///
/// A `World` has no threads of its own, no clock and no input or output.
/// The same snapshot and the same inputs always give the same state hash.
pub struct World {
    bp: Arc<Blueprint>,
    layout: Arc<Layout>,
    config: WorldConfig,
    frame: u64,
    next_resident: u64,
    /// One slot per zone of the layout. `None` when another process hosts
    /// the zone.
    zones: Vec<Option<Realm>>,
    global: Option<Realm>,
    /// Where every resident of a hosted zone is. Derived from the zone
    /// tables.
    locations: BTreeMap<ResidentId, ZoneId>,
    /// Residents that left for a zone in another process.
    away: BTreeSet<ResidentId>,
    /// Messages for realms in other processes.
    external: Vec<Envelope>,
    /// Money sent to other processes minus money received from them.
    external_money: i64,
    /// Goods sent to other processes minus goods received from them.
    external_items: BTreeMap<ItemId, i64>,
    // Not simulation state.
    staged: Vec<Envelope>,
    routed: Vec<Envelope>,
    clock: Option<TickClock>,
    timings: Vec<ZoneTiming>,
    dropped: u64,
    /// Where new residents of each city appear. Derived from the sites.
    spawn_sites: SpawnSites,
}

impl fmt::Debug for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("World")
            .field("frame", &self.frame)
            .field("zones", &self.zones.len())
            .field("residents", &self.locations.len())
            .finish_non_exhaustive()
    }
}

/// Returns the money and the goods that a message carries.
fn value_of(envelope: &Envelope) -> (i64, Vec<(ItemId, i64)>) {
    match &envelope.body {
        Body::Kernel(msg) => match &**msg {
            KernelMsg::WireMoney { amount, .. } => (*amount, Vec::new()),
            KernelMsg::WireItem { item, qty, .. } => (0, vec![(*item, *qty)]),
            KernelMsg::Arrive(transfer) => (transfer.money, transfer.items.clone()),
            _ => (0, Vec::new()),
        },
        Body::Module { .. } => (0, Vec::new()),
    }
}

impl World {
    // ---- Construction ---------------------------------------------------------

    /// Makes a world from plugins and a configuration, and runs world
    /// generation.
    ///
    /// # Panics
    ///
    /// Panics when the configuration or one of its data overrides is not
    /// valid, and when a plugin makes a mistake (see [`Builder`]). Use
    /// [`World::try_new`] to get the configuration errors as a value.
    ///
    /// [`Builder`]: crate::Builder
    #[must_use]
    pub fn new(plugins: &[&dyn Plugin], config: WorldConfig) -> World {
        match World::try_new(plugins, config) {
            Ok(world) => world,
            Err(e) => panic!("{e}"),
        }
    }

    /// Makes a world from plugins and a configuration, and runs world
    /// generation. Returns an error when the configuration or one of its
    /// data overrides is not valid.
    pub fn try_new(plugins: &[&dyn Plugin], config: WorldConfig) -> Result<World, WorldError> {
        config.validate().map_err(WorldError::Config)?;
        let bp = Blueprint::build(&KernelPlugin, plugins, None, &config.data)
            .map_err(WorldError::Data)?;
        let mut world = World::empty(bp, config);
        let context =
            WorldGen { seed: world.config.seed, config: &world.config, layout: &world.layout };
        for realm in world.zones.iter().flatten() {
            realm.worldgen(&context);
        }
        if let Some(global) = &world.global {
            global.worldgen(&context);
        }
        // Residents that world generation moved go to their zone, and the
        // zones get what the global realm sent, before the first frame.
        world.move_departures();
        world.route_outboxes();
        Ok(world)
    }

    /// Makes the realms of a world, all empty.
    fn empty(bp: Blueprint, config: WorldConfig) -> World {
        let bp = Arc::new(bp);
        let layout = Arc::new(Layout::new(&config));
        let hosts = |id: RealmId| config.hosted.as_ref().is_none_or(|list| list.contains(&id));
        let make = |id: RealmId| {
            Realm::new(id, Arc::clone(&bp), Arc::clone(&layout), config.seed, config.audit_every)
        };
        let zones = layout
            .zones()
            .iter()
            .map(|zone| hosts(RealmId::Zone(zone.id)).then(|| make(RealmId::Zone(zone.id))))
            .collect();
        let global = hosts(RealmId::Global).then(|| make(RealmId::Global));
        World {
            zones,
            global,
            bp,
            layout,
            config,
            frame: 0,
            next_resident: 1,
            locations: BTreeMap::new(),
            away: BTreeSet::new(),
            external: Vec::new(),
            external_money: 0,
            external_items: BTreeMap::new(),
            staged: Vec::new(),
            routed: Vec::new(),
            clock: None,
            timings: Vec::new(),
            dropped: 0,
            spawn_sites: SpawnSites::default(),
        }
    }

    // ---- Inputs ---------------------------------------------------------------

    /// Applies one input. It is the only way to change the state of the
    /// world; log the inputs to replay it.
    pub fn apply(&mut self, input: Input) -> Output {
        match input {
            Input::Spawn(spec) => match self.do_spawn(spec) {
                Ok(id) => Output::Spawned(id),
                Err(e) => Output::Error(e),
            },
            Input::Act { resident, request } => match self.do_act(resident, request) {
                Ok(ack) => Output::Ack(ack),
                Err(e) => Output::Error(e),
            },
            Input::Step { skip } => Output::Frame(self.do_step(&skip)),
            Input::Admin(command) => Output::Admin(self.do_admin(command)),
            Input::Deliver { messages } => {
                self.do_deliver(messages);
                Output::Delivered
            }
        }
    }

    /// Creates a resident. It wraps [`World::apply`].
    pub fn spawn(&mut self, spec: SpawnSpec) -> Result<ResidentId, SimError> {
        match self.apply(Input::Spawn(spec)) {
            Output::Spawned(id) => Ok(id),
            Output::Error(e) => Err(e),
            _ => Err(SimError::Invalid),
        }
    }

    /// Handles one action request of a resident. It wraps [`World::apply`].
    pub fn act(
        &mut self,
        resident: ResidentId,
        request: ActionRequest,
    ) -> Result<ActionAck, SimError> {
        match self.apply(Input::Act { resident, request }) {
            Output::Ack(ack) => Ok(ack),
            Output::Error(e) => Err(e),
            _ => Err(SimError::Invalid),
        }
    }

    /// Runs one frame in which every zone ticks. It wraps [`World::apply`].
    pub fn step(&mut self) -> FrameOutput {
        match self.apply(Input::Step { skip: Vec::new() }) {
            Output::Frame(frame) => frame,
            _ => FrameOutput::default(),
        }
    }

    /// Runs a command of an administrator. It wraps [`World::apply`].
    pub fn admin(&mut self, command: AdminCommand) -> AdminReply {
        match self.apply(Input::Admin(command)) {
            Output::Admin(reply) => reply,
            _ => AdminReply::Error { message: "The command gave no answer.".into() },
        }
    }

    /// Takes messages that realms in other processes sent to realms of this
    /// world. It wraps [`World::apply`].
    pub fn deliver_external(&mut self, messages: Vec<Envelope>) {
        self.apply(Input::Deliver { messages });
    }

    /// Takes the messages for realms that this world does not host. The
    /// host sends them to the process that hosts the target. It is empty
    /// when the world hosts every realm.
    pub fn take_external(&mut self) -> Vec<Envelope> {
        std::mem::take(&mut self.external)
    }

    fn zone(&self, zone: ZoneId) -> Option<&Realm> {
        self.zones.get(zone.0 as usize).and_then(Option::as_ref)
    }

    fn hosts_everything(&self) -> bool {
        self.global.is_some() && self.zones.iter().all(Option::is_some)
    }

    fn do_spawn(&mut self, spec: SpawnSpec) -> Result<ResidentId, SimError> {
        let name = spec.name.trim();
        if name.is_empty() || name.len() > NAME_MAX {
            return Err(SimError::Invalid);
        }
        if spec.model.as_ref().is_some_and(|m| m.len() > NAME_MAX) {
            return Err(SimError::Invalid);
        }
        let city = match spec.city {
            Some(city) => self.layout.city(city).map(|c| c.id).ok_or(SimError::Invalid)?,
            None => self.emptiest_city().ok_or(SimError::NotHosted)?,
        };
        let id = ResidentId(self.next_resident);
        let (zone, pos) = self.spawn_point(city, id)?;
        let realm = self.zone(zone).ok_or(SimError::NotHosted)?;
        let identity = Identity {
            name: name.to_string(),
            owner: spec.owner,
            model: spec.model,
            born: self.frame,
        };
        realm.spawn_resident(id, identity, pos);
        self.next_resident += 1;
        self.locations.insert(id, zone);
        self.after_input(zone);
        Ok(id)
    }

    /// Returns the city with the fewest residents among the cities that
    /// have a hosted zone. The lowest id wins a tie.
    fn emptiest_city(&self) -> Option<CityId> {
        self.layout
            .cities()
            .iter()
            .filter_map(|city| {
                let mut hosted = false;
                let mut residents = 0usize;
                for &zone in &city.zones {
                    if let Some(realm) = self.zone(zone) {
                        hosted = true;
                        residents += realm.len();
                    }
                }
                hosted.then_some((residents, city.id))
            })
            .min()
            .map(|(_, city)| city)
    }

    /// Returns where a new resident of a city appears: the centre of a site
    /// with the tag `spawn` when the city has one (the residents are spread
    /// over all such sites by id), else the centre of the first zone of the
    /// city.
    ///
    /// The list of spawn sites comes from a cache (see [`SpawnSites`]), so
    /// a spawn does not scan the sites of the city.
    fn spawn_point(&mut self, city: CityId, id: ResidentId) -> Result<(ZoneId, Pos), SimError> {
        let city = self.layout.city(city).ok_or(SimError::Invalid)?;
        let points = self.spawn_sites.of_city(city, &self.zones);
        if !points.is_empty() {
            return Ok(points[(id.0 % points.len() as u64) as usize]);
        }
        let first = city.zones.first().copied().ok_or(SimError::Invalid)?;
        let info = self.layout.zone(first).ok_or(SimError::Invalid)?;
        if self.zones.get(first.0 as usize).is_none_or(Option::is_none) {
            return Err(SimError::NotHosted);
        }
        Ok((first, info.bounds.centre()))
    }

    /// Finds the zone and the realm of a resident.
    fn find(&self, resident: ResidentId) -> Result<(ZoneId, &Realm), SimError> {
        match self.locations.get(&resident) {
            Some(&zone) => self.zone(zone).map(|realm| (zone, realm)).ok_or(SimError::NotHosted),
            None if self.away.contains(&resident) => Err(SimError::NotHosted),
            None => Err(SimError::UnknownResident),
        }
    }

    fn do_act(
        &mut self,
        resident: ResidentId,
        request: ActionRequest,
    ) -> Result<ActionAck, SimError> {
        let (zone, realm) = self.find(resident)?;
        let idx = realm.idx_of(resident).ok_or(SimError::UnknownResident)?;
        let ack = realm.submit(idx, request);
        self.after_input(zone);
        Ok(ack)
    }

    fn do_admin(&mut self, command: AdminCommand) -> AdminReply {
        let refuse = |message: &str| AdminReply::Error { message: message.to_string() };
        let sim_error = |e: SimError| AdminReply::Error {
            message: format!("The resident cannot be reached: {e}."),
        };
        match command {
            AdminCommand::Freeze { resident }
            | AdminCommand::Unfreeze { resident }
            | AdminCommand::Despawn { resident } => {
                let (zone, realm) = match self.find(resident) {
                    Ok(found) => found,
                    Err(e) => return sim_error(e),
                };
                let Some(idx) = realm.idx_of(resident) else {
                    return sim_error(SimError::UnknownResident);
                };
                match command {
                    AdminCommand::Freeze { .. } => realm.freeze(idx),
                    AdminCommand::Unfreeze { .. } => realm.unfreeze(idx),
                    _ => realm.despawn(idx, UNCLAIMED, "admin"),
                }
                self.after_input(zone);
                AdminReply::Ok
            }
            AdminCommand::SetParam { name, value, city } => {
                let Some(global) = &self.global else {
                    return refuse("This process does not host the global realm.");
                };
                let Some(param) = self.bp.param_id(&name) else {
                    return AdminReply::Error {
                        message: format!("No parameter is named `{name}`."),
                    };
                };
                if let Some(city) = city
                    && self.layout.city(city).is_none()
                {
                    return AdminReply::Error { message: format!("No city has the id {city}.") };
                }
                global.set_param(param, value, city);
                // Hand the change to the zones now, so that they apply it
                // in the next frame.
                self.route_outboxes();
                AdminReply::Ok
            }
            AdminCommand::RollbackZone { .. } => refuse(
                "The kernel cannot roll back a zone. A rollback needs the snapshot history of the runtime.",
            ),
        }
    }

    fn do_deliver(&mut self, messages: Vec<Envelope>) {
        for mut envelope in messages {
            // The world did not make this envelope. A module message in it
            // that no handler can take is dropped, not a reason to stop.
            envelope.foreign = true;
            let (money, items) = value_of(&envelope);
            self.external_money = self.external_money.wrapping_sub(money);
            for (item, qty) in items {
                let net = self.external_items.entry(item).or_insert(0);
                *net = net.wrapping_sub(qty);
            }
            self.route(envelope);
        }
    }

    /// Brings a zone to rest after an input between frames. Residents that
    /// left it go to their new zone at once, so between inputs every
    /// resident is in exactly one zone. The messages that the input made go
    /// to the inbox of their target at once, so the target sees them in its
    /// next tick.
    fn after_input(&mut self, zone: ZoneId) {
        let mut staged = std::mem::take(&mut self.staged);
        let (departures, despawned) = match self.zone(zone) {
            Some(realm) => {
                realm.finish_step();
                realm.take_outbox(&mut staged);
                (realm.take_departures(), realm.take_despawned())
            }
            None => (Vec::new(), Vec::new()),
        };
        for id in despawned {
            self.locations.remove(&id);
        }
        for (target, transfer) in departures {
            self.arrive(RealmId::Zone(zone), target, transfer);
        }
        for envelope in staged.drain(..) {
            self.route(envelope);
        }
        self.staged = staged;
    }

    // ---- The frame --------------------------------------------------------------

    fn do_step(&mut self, skip: &[ZoneId]) -> FrameOutput {
        self.frame += 1;
        let frame = self.frame;

        // 1. Zones, in parallel. Each closure owns one realm; nothing is
        //    shared but the immutable blueprint and layout.
        let clock = self.clock.clone();
        let tick_zone = |realm: &mut Realm| -> u64 {
            realm.begin_frame(frame);
            if realm.zone_id().is_some_and(|zone| skip.contains(&zone)) {
                return 0;
            }
            match &clock {
                Some(clock) => {
                    let start = clock();
                    realm.run_tick();
                    clock().saturating_sub(start)
                }
                None => {
                    realm.run_tick();
                    0
                }
            }
        };
        let elapsed: Vec<u64> = if self.zones.len() > 1 {
            self.zones.par_iter_mut().map(|slot| slot.as_mut().map_or(0, &tick_zone)).collect()
        } else {
            self.zones.iter_mut().map(|slot| slot.as_mut().map_or(0, &tick_zone)).collect()
        };
        if self.clock.is_some() {
            self.timings.clear();
            self.timings.extend(
                elapsed
                    .iter()
                    .enumerate()
                    .map(|(i, &elapsed)| ZoneTiming { zone: ZoneId(i as u32), elapsed }),
            );
        }

        // 2. Zone messages for the global realm, then the global tick.
        let mut staged = std::mem::take(&mut self.staged);
        for realm in self.zones.iter().flatten() {
            realm.take_outbox(&mut staged);
        }
        let mut routed = std::mem::take(&mut self.routed);
        match &self.global {
            Some(global) => {
                for envelope in staged.drain(..) {
                    if envelope.to == Dest::Global {
                        global.enqueue(envelope);
                    } else {
                        routed.push(envelope);
                    }
                }
                global.begin_frame(frame);
                global.run_tick();
                // The messages of the global realm go first.
                global.take_outbox(&mut staged);
            }
            None => routed.append(&mut staged),
        }

        // 3. Arrivals, then every other message.
        self.move_departures();
        for envelope in staged.drain(..).chain(routed.drain(..)) {
            self.route(envelope);
        }
        self.staged = staged;
        self.routed = routed;

        // Output.
        let mut out =
            FrameOutput { frame, clock: self.layout.clock_at(frame), ..FrameOutput::default() };
        if let Some(global) = &self.global {
            global.take_events(&mut out.events);
            global.take_woke(&mut out.woke);
        }
        for realm in self.zones.iter().flatten() {
            realm.take_events(&mut out.events);
            realm.take_woke(&mut out.woke);
            out.zones.extend(realm.take_delta());
        }
        out.woke.sort_unstable();
        out.woke.dedup();
        out
    }

    /// Moves every resident that left a zone to its new zone, ordered by the
    /// zone that it left, and forgets the residents that left the world.
    fn move_departures(&mut self) {
        for i in 0..self.zones.len() {
            let (departures, despawned) = match &self.zones[i] {
                Some(realm) => (realm.take_departures(), realm.take_despawned()),
                None => continue,
            };
            for id in despawned {
                self.locations.remove(&id);
            }
            for (target, transfer) in departures {
                self.arrive(RealmId::Zone(ZoneId(i as u32)), target, transfer);
            }
        }
    }

    /// Puts a migrating resident into its new zone, or hands it to the
    /// process that hosts that zone.
    fn arrive(&mut self, from: RealmId, target: ZoneId, transfer: Transfer) {
        let id = transfer.id;
        match self.zone(target) {
            Some(realm) => {
                realm.arrive(transfer);
                realm.rest();
                self.locations.insert(id, target);
                self.away.remove(&id);
            }
            None => {
                self.locations.remove(&id);
                self.away.insert(id);
                self.send_external(Envelope::new(
                    from,
                    0,
                    Dest::Zone(target),
                    Body::Kernel(Box::new(KernelMsg::Arrive(Box::new(transfer)))),
                ));
            }
        }
    }

    fn send_external(&mut self, envelope: Envelope) {
        let (money, items) = value_of(&envelope);
        self.external_money = self.external_money.wrapping_add(money);
        for (item, qty) in items {
            let net = self.external_items.entry(item).or_insert(0);
            *net = net.wrapping_add(qty);
        }
        self.external.push(envelope);
    }

    /// Moves the messages in the outboxes of all realms to their targets
    /// without a tick: first those of the global realm, then those of the
    /// zones in zone order. World generation and administrator commands use
    /// it, so that every realm sees in its next tick what was sent.
    fn route_outboxes(&mut self) {
        let mut staged = std::mem::take(&mut self.staged);
        if let Some(global) = &self.global {
            global.take_outbox(&mut staged);
        }
        for realm in self.zones.iter().flatten() {
            realm.take_outbox(&mut staged);
        }
        for envelope in staged.drain(..) {
            self.route(envelope);
        }
        self.staged = staged;
    }

    /// Delivers one message to the inbox of its target realm.
    fn route(&mut self, mut envelope: Envelope) {
        match envelope.to {
            Dest::Global => match &self.global {
                Some(global) => global.enqueue(envelope),
                None => self.send_external(envelope),
            },
            Dest::Zone(zone) => {
                if self.layout.zone(zone).is_none() {
                    // `send` to a zone that does not exist. Wires are
                    // checked when they are sent, so no value is lost here.
                    self.dropped += 1;
                    return;
                }
                let Envelope { from, seq, to, body, foreign } = envelope;
                match body {
                    Body::Kernel(msg) if matches!(*msg, KernelMsg::Arrive(_)) => {
                        if let KernelMsg::Arrive(transfer) = *msg {
                            self.arrive(from, zone, *transfer);
                        }
                    }
                    body => {
                        let envelope = Envelope { from, seq, to, body, foreign };
                        match self.zone(zone) {
                            Some(realm) => realm.enqueue(envelope),
                            None => self.send_external(envelope),
                        }
                    }
                }
            }
            Dest::Resident(resident) => match self.locations.get(&resident) {
                Some(&zone) => match self.zone(zone) {
                    Some(realm) => realm.enqueue(envelope),
                    None => self.send_external(envelope),
                },
                None if self.away.contains(&resident) => self.send_external(envelope),
                None => {
                    // The resident no longer exists. Value goes to the
                    // unclaimed account of the global realm; everything else
                    // is dropped.
                    let redirected = match &mut envelope.body {
                        Body::Kernel(msg) => match &mut **msg {
                            KernelMsg::WireMoney { to, .. } | KernelMsg::WireItem { to, .. } => {
                                *to = UNCLAIMED;
                                true
                            }
                            _ => false,
                        },
                        Body::Module { .. } => false,
                    };
                    if redirected {
                        envelope.to = Dest::Global;
                        match &self.global {
                            Some(global) => global.enqueue(envelope),
                            None => self.send_external(envelope),
                        }
                    } else {
                        self.dropped += 1;
                    }
                }
            },
            Dest::AllZones => {
                for realm in self.zones.iter().flatten() {
                    realm.enqueue(envelope.clone());
                }
                if self.zones.iter().any(Option::is_none) {
                    self.send_external(envelope);
                }
            }
        }
    }

    // ---- Reads ------------------------------------------------------------------

    /// Builds what a resident can see now, and what it received since
    /// `opts.since_notice`. It does not change state.
    pub fn observe(&self, r: ResidentId, opts: &ObserveOpts) -> Result<Observation, SimError> {
        let (_, realm) = self.find(r)?;
        let mut observation = realm.observe(r, opts).ok_or(SimError::UnknownResident)?;
        if let Some(global) = &self.global {
            global.observe_global(r, opts, &mut observation.sections);
        }
        Ok(observation)
    }

    /// Returns the sequence number that the next notice of a resident will
    /// get. A caller that waits for news compares it with later values.
    pub fn next_notice(&self, r: ResidentId) -> Result<u64, SimError> {
        let (_, realm) = self.find(r)?;
        realm.next_notice(r).ok_or(SimError::UnknownResident)
    }

    /// Answers a question of a viewer: the layout of the world, or
    /// everything in one zone. It does not change state.
    pub fn view(&self, q: &ViewQuery) -> Result<WorldView, SimError> {
        match q {
            ViewQuery::Layout => Ok(WorldView::Layout {
                frame: self.frame,
                clock: self.layout.clock_at(self.frame),
                cities: self
                    .layout
                    .cities()
                    .iter()
                    .map(|city| CityView {
                        id: city.id,
                        name: city.name.clone(),
                        bounds: city.bounds,
                        zones: city.zones.clone(),
                    })
                    .collect(),
                zones: self
                    .layout
                    .zones()
                    .iter()
                    .map(|zone| unsup_protocol::ZoneView {
                        id: zone.id,
                        city: zone.city,
                        name: zone.name.clone(),
                        bounds: zone.bounds,
                        residents: self.zone(zone.id).map_or(0, |realm| realm.len() as u32),
                    })
                    .collect(),
            }),
            ViewQuery::Zone { zone } => {
                if self.layout.zone(*zone).is_none() {
                    return Err(SimError::Invalid);
                }
                self.zone(*zone).map(Realm::world_view).ok_or(SimError::NotHosted)
            }
        }
    }

    /// Returns the public catalogue: verbs with the JSON schemas of their
    /// arguments, parameters with their national values, items, attributes
    /// and conditions.
    #[must_use]
    pub fn catalogue(&self) -> Catalogue {
        self.bp.catalogue(|param| match &self.global {
            Some(global) => global.param(param),
            None => self.bp.param(param).value,
        })
    }

    /// Returns the number of frames that the world has run.
    #[must_use]
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Returns one realm, for tests and administration tools.
    ///
    /// All `Realm` methods take `&self`, so a caller can change state
    /// through this reference. Such a change is not an input: it is not in
    /// the input log and a replay will not have it. After a change that
    /// makes a resident leave its zone, call [`World::settle`].
    #[must_use]
    pub fn realm(&self, id: RealmId) -> Option<&Realm> {
        match id {
            RealmId::Global => self.global.as_ref(),
            RealmId::Zone(zone) => self.zone(zone),
        }
    }

    /// Returns the zone in which a resident is, or `None` when it is not in
    /// a zone that this world hosts.
    #[must_use]
    pub fn locate(&self, r: ResidentId) -> Option<ZoneId> {
        self.locations.get(&r).copied()
    }

    /// Returns the configuration of the world.
    #[must_use]
    pub fn config(&self) -> &WorldConfig {
        &self.config
    }

    /// Returns the cities and zones of the world.
    #[must_use]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Returns the definitions of the world.
    #[must_use]
    pub fn blueprint(&self) -> &Blueprint {
        &self.bp
    }

    /// Returns the number of residents in the zones that this world hosts.
    #[must_use]
    pub fn residents(&self) -> usize {
        self.locations.len()
    }

    /// Returns the number of messages that were dropped. It is a statistic,
    /// not simulation state.
    ///
    /// A message is dropped when it has no target: it is for a resident
    /// that no longer exists, or for a zone that does not exist. A module
    /// message that came through `Input::Deliver` is also dropped when no
    /// handler of its target takes the kind, or when its payload does not
    /// decode into the type of the handler.
    #[must_use]
    pub fn dropped_messages(&self) -> u64 {
        let in_realms: u64 =
            self.global.iter().chain(self.zones.iter().flatten()).map(Realm::dropped).sum();
        self.dropped + in_realms
    }

    /// Brings every zone to rest and moves residents that left their zone,
    /// without a frame. Tests and administration tools call it after they
    /// changed state through [`World::realm`].
    pub fn settle(&mut self) {
        for i in 0..self.zones.len() {
            self.after_input(ZoneId(i as u32));
        }
        if let Some(global) = &self.global {
            global.rest();
        }
        self.route_outboxes();
    }

    /// Gives the world a clock to measure how long each zone ticks, or
    /// takes it away. The runtime uses the measurements to decide time
    /// dilation. They never change simulation state.
    pub fn set_clock(&mut self, clock: Option<TickClock>) {
        self.clock = clock;
        self.timings.clear();
    }

    /// Returns how long each zone ticked in the last frame. It is empty
    /// without a clock.
    #[must_use]
    pub fn timings(&self) -> &[ZoneTiming] {
        &self.timings
    }

    /// Returns an estimate of the memory that residents use.
    #[must_use]
    pub fn memory_estimate(&self) -> MemoryEstimate {
        MemoryEstimate {
            residents: self.locations.len(),
            bytes: self.zones.iter().flatten().map(Realm::memory_estimate).sum::<usize>()
                + self.locations.len() * 32,
        }
    }

    // ---- Audit ------------------------------------------------------------------

    /// Audits the ledgers of all hosted realms and the whole world: nothing
    /// is created or lost outside faucets and sinks, also not in messages
    /// between realms. Returns the totals.
    pub fn audit(&self) -> Result<WorldTotals, AuditError> {
        let mut totals = WorldTotals::default();
        let mut wired = 0i64;
        let mut wired_items: BTreeMap<ItemId, i64> = BTreeMap::new();
        let realms = self.global.iter().chain(self.zones.iter().flatten());
        for realm in realms.clone() {
            let ledger = realm.ledger();
            if let Err(mut error) = ledger.audit() {
                error.realm = Some(realm.id());
                return Err(error);
            }
            let t = ledger.totals();
            totals.money.held = totals.money.held.wrapping_add(t.money.held);
            totals.money.minted = totals.money.minted.wrapping_add(t.money.minted);
            totals.money.burned = totals.money.burned.wrapping_add(t.money.burned);
            wired = wired.wrapping_add(t.money.wired_out).wrapping_sub(t.money.wired_in);
            for (item, it) in t.items {
                let entry = totals.items.entry(item).or_default();
                entry.held = entry.held.wrapping_add(it.held);
                entry.produced = entry.produced.wrapping_add(it.produced);
                entry.consumed = entry.consumed.wrapping_add(it.consumed);
                let net = wired_items.entry(item).or_insert(0);
                *net = net.wrapping_add(it.wired_out).wrapping_sub(it.wired_in);
            }
        }
        // Value in messages that wait in an inbox or an outbox.
        for realm in realms {
            realm.for_each_pending(|envelope| {
                let (money, items) = value_of(envelope);
                totals.money.in_flight = totals.money.in_flight.wrapping_add(money);
                for (item, qty) in items {
                    let entry = totals.items.entry(item).or_default();
                    entry.in_flight = entry.in_flight.wrapping_add(qty);
                }
            });
        }
        totals.money.external = self.external_money;
        for (&item, &net) in &self.external_items {
            totals.items.entry(item).or_default().external = net;
        }
        // What left the ledgers must be on the way or outside.
        let on_the_way = totals.money.in_flight.wrapping_add(totals.money.external);
        if wired != on_the_way {
            return Err(AuditError {
                realm: None,
                item: None,
                held: totals.money.held.wrapping_add(on_the_way),
                expected: totals.money.minted.wrapping_sub(totals.money.burned),
            });
        }
        for (&item, t) in &totals.items {
            let on_the_way = t.in_flight.wrapping_add(t.external);
            if wired_items.get(&item).copied().unwrap_or(0) != on_the_way {
                return Err(AuditError {
                    realm: None,
                    item: Some(item),
                    held: t.held.wrapping_add(on_the_way),
                    expected: t.produced.wrapping_sub(t.consumed),
                });
            }
        }
        Ok(totals)
    }

    // ---- Snapshot -----------------------------------------------------------------

    fn snapshot_parts(&self) -> SnapshotRef<'_> {
        SnapshotRef {
            magic: SNAPSHOT_MAGIC,
            version: SNAPSHOT_VERSION,
            config: &self.config,
            names: self.bp.name_tables(),
            frame: self.frame,
            next_resident: self.next_resident,
            away: &self.away,
            external: &self.external,
            external_money: self.external_money,
            external_items: &self.external_items,
            realms: Vec::new(),
        }
    }

    /// Returns the hash of the whole simulation state: blake3 over the
    /// canonical serialized state. Two worlds with the same snapshot and the
    /// same inputs have the same hash.
    #[must_use]
    pub fn state_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(&codec::encode("snapshot header", &self.snapshot_parts()));
        for realm in self.global.iter().chain(self.zones.iter().flatten()) {
            let bytes = realm.save();
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        hasher.finalize().into()
    }

    /// Writes the whole simulation state as MessagePack. The snapshot holds
    /// the configuration and the names behind all numeric ids, so
    /// [`World::restore`] needs only the plugins.
    ///
    /// Output for viewers that waits for the next frame (public events,
    /// zone deltas and the list of residents with news, made by inputs
    /// since the last frame) is not simulation state. It is not in the
    /// snapshot and not in the state hash.
    #[must_use]
    pub fn snapshot(&self) -> Vec<u8> {
        let mut parts = self.snapshot_parts();
        parts.realms = self
            .global
            .iter()
            .chain(self.zones.iter().flatten())
            .map(|realm| RealmBlob { id: realm.id(), data: realm.save() })
            .collect();
        codec::encode("snapshot", &parts)
    }

    /// Returns the snapshot and the state hash, and serializes the state of
    /// every realm only once. The bytes are those of [`World::snapshot`] and
    /// the hash is that of [`World::state_hash`]; a caller that needs both
    /// (a snapshot writer that names the file after the hash) saves half of
    /// the work.
    #[must_use]
    pub fn snapshot_with_hash(&self) -> (Vec<u8>, [u8; 32]) {
        let mut parts = self.snapshot_parts();
        let mut hasher = blake3::Hasher::new();
        hasher.update(&codec::encode("snapshot header", &parts));
        for realm in self.global.iter().chain(self.zones.iter().flatten()) {
            let data = realm.save();
            hasher.update(&(data.len() as u64).to_le_bytes());
            hasher.update(&data);
            parts.realms.push(RealmBlob { id: realm.id(), data });
        }
        (codec::encode("snapshot", &parts), hasher.finalize().into())
    }

    /// Makes a world from a snapshot. World generation does not run again.
    ///
    /// The plugins may differ from those that made the snapshot. Every
    /// parameter, item, attribute, condition, faucet, sink and verb keeps
    /// its numeric id; new ones get new ids, and ones that no plugin
    /// registers any more stay as retired entries. Components and resources
    /// are matched by name: a new one starts with its default value, and one
    /// that no plugin registers any more is dropped.
    pub fn restore(plugins: &[&dyn Plugin], bytes: &[u8]) -> Result<World, SnapshotError> {
        let snapshot: Snapshot = codec::decode("snapshot", bytes).map_err(SnapshotError::Decode)?;
        if snapshot.magic != SNAPSHOT_MAGIC {
            return Err(SnapshotError::Decode(DecodeError {
                what: "snapshot".into(),
                message: "the data is not a snapshot".into(),
            }));
        }
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::Version { found: snapshot.version });
        }
        snapshot.config.validate().map_err(SnapshotError::Config)?;
        let bp =
            Blueprint::build(&KernelPlugin, plugins, Some(&snapshot.names), &snapshot.config.data)
                .map_err(SnapshotError::Data)?;
        let mut world = World::empty(bp, snapshot.config);
        world.frame = snapshot.frame;
        world.next_resident = snapshot.next_resident;
        world.away = snapshot.away;
        world.external = snapshot.external;
        world.external_money = snapshot.external_money;
        world.external_items = snapshot.external_items;

        let mut seen = BTreeSet::new();
        for blob in snapshot.realms {
            let state: RealmState = codec::decode(&format!("the state of {}", blob.id), &blob.data)
                .map_err(SnapshotError::Decode)?;
            if state.id != blob.id || !seen.insert(blob.id) {
                return Err(SnapshotError::Realms(format!(
                    "the state of {} is there twice or mislabelled",
                    blob.id
                )));
            }
            let Some(realm) = world.realm(blob.id) else {
                return Err(SnapshotError::Realms(format!(
                    "the snapshot has the state of {}, which its configuration does not host",
                    blob.id
                )));
            };
            realm.load(state, world.frame).map_err(SnapshotError::Decode)?;
        }
        for realm in world.global.iter().chain(world.zones.iter().flatten()) {
            if !seen.contains(&realm.id()) {
                return Err(SnapshotError::Realms(format!(
                    "the snapshot lacks the state of {}",
                    realm.id()
                )));
            }
        }
        let mut locations = BTreeMap::new();
        for realm in world.zones.iter().flatten() {
            if let Some(zone) = realm.zone_id() {
                for &id in realm.ids().iter() {
                    if locations.insert(id, zone).is_some() {
                        return Err(SnapshotError::Realms(format!(
                            "resident {id} is in two zones"
                        )));
                    }
                }
            }
        }
        world.locations = locations;
        Ok(world)
    }

    /// Returns true when this world hosts every realm of its layout.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.hosts_everything()
    }
}
