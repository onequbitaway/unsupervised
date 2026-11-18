//! Plugins, the builder and the blueprint.
//!
//! A module is a [`Plugin`]. When a world is made, the kernel calls
//! [`Plugin::build`] twice for every plugin: once for the zone scope and once
//! for the global scope. The plugin registers data, state and behaviour on
//! the [`Builder`]. The result is the [`Blueprint`]: an immutable description
//! that every realm of the world shares.

use std::any::{TypeId, type_name};
use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use unsup_protocol::{
    ActionError, AttrInfo, Catalogue, CondInfo, Event, ItemId, ItemInfo, ObserveOpts, ParamInfo,
    Pos, RealmId, ResidentId, Tick, VerbInfo, VerbScope, ZoneId,
};

use crate::codec::{self, DecodeError};
use crate::column::{AnyColumn, AnyResource, Component, Resource, new_column, new_resource};
use crate::config::WorldConfig;
use crate::defs::{
    AttrDef, AttrShow, CondDef, DataError, DataFile, ItemDef, ParamDef, Registry, check_param,
};
use crate::ids::{AttrId, CondId, FaucetId, Idx, ParamId, SinkId, VerbId};
use crate::layout::Layout;
use crate::realm::Realm;

/// A module: a set of verbs, systems, data and observers on top of the
/// kernel.
pub trait Plugin: Send + Sync + 'static {
    /// Name of the module, for example `life`.
    fn name(&self) -> &'static str;

    /// Registers everything that the module adds. The kernel calls it once
    /// for the zone scope and once for the global scope; see
    /// [`Builder::scope`].
    fn build(&self, b: &mut Builder);
}

/// The two kinds of realm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Scope {
    /// A zone realm: one district with its residents.
    Zone,
    /// The global realm: state without a location.
    Global,
}

/// The stages of a tick in which module systems run, in this order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Stage {
    /// Before the main stage: decay, timers, input from messages.
    Pre,
    /// The main stage.
    Update,
    /// After the main stage: bookkeeping, statistics.
    Post,
}

/// The description of a verb.
#[derive(Clone, Copy, Debug)]
pub struct VerbSpec {
    /// Name of the verb: `snake_case`, unique in the world.
    pub verb: &'static str,
    /// Name of the module that owns the verb.
    pub module: &'static str,
    /// One sentence for agents.
    pub summary: &'static str,
    /// True when the verb does not occupy the resident. A free verb runs at
    /// once, even when the resident is busy, and must finish at once.
    pub free: bool,
    /// Names of the blocking conditions that do not block this verb.
    pub allowed_in: &'static [&'static str],
}

/// What the `begin` handler of a verb decided.
#[derive(Clone, Debug, PartialEq)]
pub enum Begin {
    /// The action finished now, with this result.
    Done(Value),
    /// The action occupies the resident. `finish` runs after `ticks` ticks
    /// (at least one).
    Busy {
        /// Duration in ticks.
        ticks: u32,
    },
    /// The action occupies the resident until someone calls
    /// `Realm::complete_action`. It fails with `timeout` after `timeout`
    /// ticks (at least one).
    Wait {
        /// Greatest waiting time in ticks.
        timeout: u32,
    },
}

/// What a zone verb handler knows about the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActionCtx {
    /// The resident that acts.
    pub resident: ResidentId,
    /// Its dense index, valid during this call.
    pub idx: Idx,
    /// Id of the action.
    pub action_id: u64,
    /// Tick of the zone at which the action started.
    pub started: Tick,
}

/// What a global verb handler knows about the action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GlobalCtx {
    /// The resident that acts.
    pub resident: ResidentId,
    /// Id of the action.
    pub action_id: u64,
    /// The zone in which the resident was when it asked.
    pub zone: ZoneId,
    /// The position of the resident when it asked.
    pub pos: Pos,
}

/// The arguments of a verb. The doc comments of the type become the schema
/// that agents read.
///
/// The trait is implemented for every type that derives `Serialize`,
/// `Deserialize` and `JsonSchema`.
pub trait Args: Serialize + DeserializeOwned + JsonSchema + 'static {}

impl<T: Serialize + DeserializeOwned + JsonSchema + 'static> Args for T {}

/// The arguments of a verb that takes none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NoArgs {}

/// Typed data that one realm sends to another. `KIND` identifies the type on
/// the wire; it must be unique.
pub trait Message: Serialize + DeserializeOwned + 'static {
    /// Unique name of the message: `<module>.<what>`.
    const KIND: &'static str;
}

/// What a world generation hook knows about the world.
#[derive(Clone, Copy, Debug)]
pub struct WorldGen<'a> {
    /// Seed of the world.
    pub seed: u64,
    /// Configuration of the world.
    pub config: &'a WorldConfig,
    /// Cities and zones of the world.
    pub layout: &'a Layout,
}

pub(crate) type SystemFn = Box<dyn Fn(&Realm) + Send + Sync>;
pub(crate) type BeginFn =
    Box<dyn Fn(&Realm, &ActionCtx, &Value) -> Result<Begin, ActionError> + Send + Sync>;
pub(crate) type FinishFn =
    Box<dyn Fn(&Realm, &ActionCtx, &Value) -> Result<Value, ActionError> + Send + Sync>;
pub(crate) type GlobalFn =
    Box<dyn Fn(&Realm, &GlobalCtx, &Value) -> Result<Value, ActionError> + Send + Sync>;
pub(crate) type ValidateFn = Box<dyn Fn(&Value) -> Result<(), ActionError> + Send + Sync>;
/// A message handler. It returns an error when the payload does not decode
/// into the type of the handler; the handler of the module did not run then.
pub(crate) type MessageFn =
    Box<dyn Fn(&Realm, RealmId, &[u8]) -> Result<(), DecodeError> + Send + Sync>;
/// A handler of a message for a resident. See [`MessageFn`].
pub(crate) type ResidentMessageFn =
    Box<dyn Fn(&Realm, Idx, RealmId, &[u8]) -> Result<(), DecodeError> + Send + Sync>;
pub(crate) type EventFn = Box<dyn Fn(&Realm, &Event) + Send + Sync>;
pub(crate) type ObserveFn = Box<dyn Fn(&Realm, Idx, &ObserveOpts) -> Option<Value> + Send + Sync>;
pub(crate) type ObserveGlobalFn =
    Box<dyn Fn(&Realm, ResidentId, &ObserveOpts) -> Option<Value> + Send + Sync>;
pub(crate) type WorldGenFn = Box<dyn Fn(&Realm, &WorldGen<'_>) + Send + Sync>;
pub(crate) type HookFn = Box<dyn Fn(&Realm, Idx) + Send + Sync>;

/// How a verb runs.
pub(crate) enum VerbKind {
    /// In the resident's zone.
    Zone { begin: BeginFn, finish: FinishFn },
    /// In the global realm.
    Global { run: GlobalFn },
    /// The verb was in a snapshot but no plugin registers it any more.
    Retired,
}

/// Everything that the kernel knows about one verb.
pub(crate) struct VerbEntry {
    pub(crate) name: &'static str,
    pub(crate) module: &'static str,
    pub(crate) summary: &'static str,
    pub(crate) free: bool,
    /// True for kernel verbs: no module condition blocks them.
    pub(crate) allow_all: bool,
    allowed_names: &'static [&'static str],
    /// `allowed_names` as ids. Names that no plugin defines are dropped.
    pub(crate) allowed: Vec<CondId>,
    pub(crate) schema: Value,
    pub(crate) validate: ValidateFn,
    pub(crate) kind: VerbKind,
}

impl VerbEntry {
    pub(crate) fn is_global(&self) -> bool {
        matches!(self.kind, VerbKind::Global { .. })
    }
}

pub(crate) struct SystemEntry {
    pub(crate) stage: Stage,
    order: i32,
    seq: u32,
    /// Name of the system, for profiles and panics.
    pub(crate) name: &'static str,
    /// The system runs every `period` ticks. 1 means every tick.
    pub(crate) period: u32,
    pub(crate) run: SystemFn,
}

pub(crate) struct ComponentEntry {
    pub(crate) name: &'static str,
    pub(crate) make: fn() -> Box<dyn AnyColumn>,
}

pub(crate) struct ResourceEntry {
    pub(crate) name: &'static str,
    pub(crate) make: fn() -> Box<dyn AnyResource>,
}

pub(crate) struct Observer<F> {
    pub(crate) section: &'static str,
    order: i32,
    seq: u32,
    pub(crate) run: F,
}

pub(crate) struct Ordered<F> {
    order: i32,
    seq: u32,
    pub(crate) run: F,
}

/// What plugins registered for one scope.
#[derive(Default)]
pub(crate) struct ScopeDef {
    pub(crate) components: Vec<ComponentEntry>,
    comp_index: BTreeMap<TypeId, usize>,
    pub(crate) resources: Vec<ResourceEntry>,
    res_index: BTreeMap<TypeId, usize>,
    pub(crate) systems: Vec<SystemEntry>,
    pub(crate) messages: BTreeMap<&'static str, Vec<MessageFn>>,
    pub(crate) resident_messages: BTreeMap<&'static str, Vec<ResidentMessageFn>>,
    pub(crate) events: Vec<(&'static str, EventFn)>,
    pub(crate) observers: Vec<Observer<ObserveFn>>,
    pub(crate) global_observers: Vec<Observer<ObserveGlobalFn>>,
    pub(crate) worldgen: Vec<Ordered<WorldGenFn>>,
    pub(crate) on_spawn: Vec<Ordered<HookFn>>,
    pub(crate) on_arrive: Vec<HookFn>,
    pub(crate) on_depart: Vec<HookFn>,
    pub(crate) on_despawn: Vec<HookFn>,
}

impl ScopeDef {
    /// Returns true when a handler of this scope listens to the event kind.
    pub(crate) fn wants_event(&self, kind: &str) -> bool {
        self.events.iter().any(|(prefix, _)| kind.starts_with(prefix))
    }

    pub(crate) fn component_slot(&self, type_id: TypeId) -> Option<usize> {
        self.comp_index.get(&type_id).copied()
    }

    pub(crate) fn resource_slot(&self, type_id: TypeId) -> Option<usize> {
        self.res_index.get(&type_id).copied()
    }
}

/// Ids of the parameters and conditions that the kernel itself uses.
#[derive(Clone, Copy, Debug)]
pub(crate) struct KernelIds {
    pub(crate) queue_max: ParamId,
    pub(crate) free_actions_per_tick: ParamId,
    pub(crate) inbox_max: ParamId,
    pub(crate) routines_max: ParamId,
    pub(crate) sight_radius: ParamId,
    pub(crate) nearby_max: ParamId,
    pub(crate) global_timeout: ParamId,
    pub(crate) routine_backoff: ParamId,
    pub(crate) routine_reports: ParamId,
    pub(crate) late_reports: ParamId,
    pub(crate) frozen: CondId,
}

/// The names behind the numeric ids of a world, in id order.
///
/// A snapshot stores them. A restored world gives every name its old id, so
/// state that holds ids stays valid when the set of plugins changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameTables {
    /// Parameter names.
    pub params: Vec<String>,
    /// Item names.
    pub items: Vec<String>,
    /// Attribute names.
    pub attrs: Vec<String>,
    /// Condition names.
    pub conditions: Vec<String>,
    /// Faucet names.
    pub faucets: Vec<String>,
    /// Sink names.
    pub sinks: Vec<String>,
    /// Verb names.
    pub verbs: Vec<String>,
}

/// The immutable description of a world: definitions and handlers.
///
/// All realms share one blueprint. Read it with `Realm::defs`.
pub struct Blueprint {
    params: Registry<ParamDef>,
    items: Registry<ItemDef>,
    attrs: Registry<AttrDef>,
    conds: Registry<CondDef>,
    faucets: Registry<()>,
    sinks: Registry<()>,
    verbs: Registry<VerbEntry>,
    verb_names: Vec<&'static str>,
    pub(crate) zone: ScopeDef,
    pub(crate) global: ScopeDef,
    message_types: BTreeMap<&'static str, (TypeId, &'static str)>,
    sections: BTreeMap<&'static str, &'static str>,
    /// Bounds of every attribute, by id.
    attr_bounds: Vec<(i64, i64)>,
    /// Whether each condition is blocking, by id.
    cond_blocking: Vec<bool>,
    kernel: Option<KernelIds>,
    /// A hash of the component names in column order.
    pub(crate) component_signature: u64,
}

impl std::fmt::Debug for Blueprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blueprint")
            .field("params", &self.params.len())
            .field("items", &self.items.len())
            .field("attrs", &self.attrs.len())
            .field("conditions", &self.conds.len())
            .field("verbs", &self.verbs.len())
            .field("components", &self.zone.components.len())
            .finish_non_exhaustive()
    }
}

/// Leaks a name so that it can be used where a `&'static str` is needed.
/// Only names of retired verbs from a snapshot go through here: a bounded
/// number of small strings per restore.
fn leak(name: &str) -> &'static str {
    Box::leak(name.to_string().into_boxed_str())
}

/// FNV-1a over a list of names.
fn hash_names<'a>(names: impl Iterator<Item = &'a str>) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for name in names {
        for byte in name.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

impl Blueprint {
    fn empty() -> Blueprint {
        Blueprint {
            params: Registry::new("parameter"),
            items: Registry::new("item"),
            attrs: Registry::new("attribute"),
            conds: Registry::new("condition"),
            faucets: Registry::new("faucet"),
            sinks: Registry::new("sink"),
            verbs: Registry::new("verb"),
            verb_names: Vec::new(),
            zone: ScopeDef::default(),
            global: ScopeDef::default(),
            message_types: BTreeMap::new(),
            sections: BTreeMap::new(),
            attr_bounds: Vec::new(),
            cond_blocking: Vec::new(),
            kernel: None,
            component_signature: 0,
        }
    }

    /// Builds the blueprint of a world.
    ///
    /// `kernel` is the kernel's own plugin; it is built first. `reserved`
    /// holds the name tables of a snapshot, when the world is restored.
    /// `overrides` are data files that are applied after all plugins.
    pub(crate) fn build(
        kernel: &dyn Plugin,
        plugins: &[&dyn Plugin],
        reserved: Option<&NameTables>,
        overrides: &[String],
    ) -> Result<Blueprint, DataError> {
        let mut bp = Blueprint::empty();
        if let Some(names) = reserved {
            bp.params.reserve(&names.params);
            bp.items.reserve(&names.items);
            bp.attrs.reserve(&names.attrs);
            bp.conds.reserve(&names.conditions);
            bp.faucets.reserve(&names.faucets);
            bp.sinks.reserve(&names.sinks);
            bp.verbs.reserve(&names.verbs);
        }
        let mut builder = Builder { scope: Scope::Zone, plugin: "kernel", seq: 0, bp };
        for scope in [Scope::Zone, Scope::Global] {
            builder.scope = scope;
            for plugin in std::iter::once(kernel).chain(plugins.iter().copied()) {
                builder.plugin = plugin.name();
                plugin.build(&mut builder);
            }
        }
        let mut bp = builder.bp;
        for text in overrides {
            bp.apply_override(text)?;
        }
        bp.finish();
        Ok(bp)
    }

    /// Applies an override data file: it changes parameters that exist and
    /// changes or adds items.
    fn apply_override(&mut self, text: &str) -> Result<(), DataError> {
        let file = DataFile::parse(text)?;
        for (name, data) in file.params {
            let Some(id) = self.params.id(&name) else {
                return Err(DataError {
                    message: format!("override of unknown parameter `{name}`"),
                });
            };
            let Some(def) = self.params.get(id) else {
                return Err(DataError {
                    message: format!("override of retired parameter `{name}`"),
                });
            };
            let changed = data.apply_to(def)?;
            self.params.replace(id, changed);
        }
        for data in file.items {
            match self.items.id(&data.name).and_then(|id| self.items.get(id).map(|def| (id, def))) {
                Some((id, def)) => {
                    let changed = data.apply_to(def);
                    self.items.replace(id, changed);
                }
                None => {
                    let def = data.into_def()?;
                    let name = def.name.clone();
                    self.items.ensure(&name, def);
                }
            }
        }
        Ok(())
    }

    /// Freezes the blueprint: fills retired names, sorts handlers and builds
    /// the lookup tables.
    fn finish(&mut self) {
        self.params.fill_reserved(|name| ParamDef::new(name, 0).summary("Retired."));
        self.items.fill_reserved(|name| ItemDef::new(name).summary("Retired."));
        self.attrs
            .fill_reserved(|name| AttrDef::new(name).show(AttrShow::Hidden).summary("Retired."));
        self.conds.fill_reserved(|name| CondDef::new(name).summary("Retired."));
        self.faucets.fill_reserved(|_| ());
        self.sinks.fill_reserved(|_| ());
        self.verbs.fill_reserved(|name| VerbEntry {
            name: leak(name),
            module: "retired",
            summary: "This verb no longer exists.",
            free: false,
            allow_all: false,
            allowed_names: &[],
            allowed: Vec::new(),
            schema: Value::Null,
            validate: Box::new(|_| Ok(())),
            kind: VerbKind::Retired,
        });
        self.verb_names = self.verbs.iter().map(|(_, v)| v.name).collect();

        self.attr_bounds = self.attrs.iter().map(|(_, def)| (def.min, def.max)).collect();
        self.cond_blocking = self.conds.iter().map(|(_, def)| def.blocking).collect();

        // Resolve `allowed_in` names. A name that no plugin defines is
        // dropped: the module that owns the condition is not loaded.
        let allowed: Vec<Vec<CondId>> = self
            .verbs
            .iter()
            .map(|(_, verb)| {
                verb.allowed_names
                    .iter()
                    .filter_map(|name| self.conds.id(name).map(CondId))
                    .collect()
            })
            .collect();
        for (id, list) in allowed.into_iter().enumerate() {
            if let Some(verb) = self.verbs.get_mut(id as u16) {
                verb.allowed = list;
            }
        }

        for scope in [&mut self.zone, &mut self.global] {
            scope.systems.sort_by_key(|s| (s.stage, s.order, s.seq));
            scope.observers.sort_by_key(|o| (o.order, o.seq));
            scope.global_observers.sort_by_key(|o| (o.order, o.seq));
            scope.worldgen.sort_by_key(|h| (h.order, h.seq));
            scope.on_spawn.sort_by_key(|h| (h.order, h.seq));
        }
        self.component_signature = hash_names(self.zone.components.iter().map(|c| c.name));

        let param = |name: &str| {
            ParamId(
                self.params
                    .id(name)
                    .unwrap_or_else(|| panic!("kernel parameter `{name}` is missing")),
            )
        };
        self.kernel = Some(KernelIds {
            queue_max: param("kernel.queue_max"),
            free_actions_per_tick: param("kernel.free_actions_per_tick"),
            inbox_max: param("kernel.inbox_max"),
            routines_max: param("kernel.routines_max"),
            sight_radius: param("kernel.sight_radius"),
            nearby_max: param("kernel.nearby_max"),
            global_timeout: param("kernel.global_timeout"),
            routine_backoff: param("kernel.routine_backoff"),
            routine_reports: param("kernel.routine_reports"),
            late_reports: param("kernel.late_reports"),
            frozen: CondId(self.conds.id("frozen").expect("the kernel registers `frozen`")),
        });
    }

    pub(crate) fn kernel_ids(&self) -> &KernelIds {
        self.kernel.as_ref().expect("the blueprint is finished")
    }

    pub(crate) fn scope(&self, scope: Scope) -> &ScopeDef {
        match scope {
            Scope::Zone => &self.zone,
            Scope::Global => &self.global,
        }
    }

    pub(crate) fn verb(&self, id: VerbId) -> &VerbEntry {
        self.verbs.get(id.0).expect("verb ids come from this blueprint")
    }

    pub(crate) fn verb_id(&self, name: &str) -> Option<VerbId> {
        self.verbs.id(name).map(VerbId)
    }

    /// Returns the name of a verb.
    pub(crate) fn verb_name(&self, id: VerbId) -> &'static str {
        self.verb_names.get(id.index()).copied().unwrap_or("unknown")
    }

    pub(crate) fn attr_bounds(&self, id: AttrId) -> (i64, i64) {
        self.attr_bounds[id.index()]
    }

    pub(crate) fn is_blocking(&self, id: CondId) -> bool {
        self.cond_blocking.get(id.index()).copied().unwrap_or(false)
    }

    /// Checks, in debug builds, that a message kind is not used by two
    /// different types.
    pub(crate) fn check_message_type<M: Message>(&self) {
        if let Some((type_id, name)) = self.message_types.get(M::KIND) {
            debug_assert!(
                *type_id == TypeId::of::<M>(),
                "message kind `{}` is used by both {} and {}",
                M::KIND,
                name,
                type_name::<M>()
            );
        }
    }

    /// Returns true when a handler of the scope takes messages of this kind
    /// (`Message::KIND`) that are sent to a realm: `Dest::Global`,
    /// `Dest::Zone` or `Dest::AllZones`. The handlers are those of
    /// `Builder::on_message`.
    ///
    /// A sender outside the simulation asks this before it sends
    /// (`Envelope::module`). A message that no handler takes is dropped.
    #[must_use]
    pub fn handles_message(&self, kind: &str, scope: Scope) -> bool {
        self.scope(scope).messages.contains_key(kind)
    }

    /// Returns true when a handler takes messages of this kind that are
    /// sent to a resident (`Dest::Resident`). The handlers are those of
    /// `Builder::on_resident_message`; they run in the zone of the resident.
    #[must_use]
    pub fn handles_resident_message(&self, kind: &str) -> bool {
        self.zone.resident_messages.contains_key(kind)
    }

    /// Returns the names behind all numeric ids, for a snapshot.
    #[must_use]
    pub fn name_tables(&self) -> NameTables {
        NameTables {
            params: self.params.names(),
            items: self.items.names(),
            attrs: self.attrs.names(),
            conditions: self.conds.names(),
            faucets: self.faucets.names(),
            sinks: self.sinks.names(),
            verbs: self.verbs.names(),
        }
    }

    // ---- Parameters ----------------------------------------------------

    /// Returns the id of a parameter.
    #[must_use]
    pub fn param_id(&self, name: &str) -> Option<ParamId> {
        self.params.id(name).map(ParamId)
    }

    /// Returns the definition of a parameter.
    ///
    /// # Panics
    ///
    /// Panics when the id is not from this blueprint.
    #[must_use]
    pub fn param(&self, id: ParamId) -> &ParamDef {
        self.params.get(id.0).expect("parameter ids come from this blueprint")
    }

    /// Returns every parameter in id order.
    pub fn params(&self) -> impl Iterator<Item = (ParamId, &ParamDef)> + '_ {
        self.params.iter().map(|(id, def)| (ParamId(id), def))
    }

    // ---- Items ---------------------------------------------------------

    /// Returns the id of an item.
    #[must_use]
    pub fn item_id(&self, name: &str) -> Option<ItemId> {
        self.items.id(name).map(ItemId)
    }

    /// Returns the definition of an item, or `None` when the id is unknown.
    #[must_use]
    pub fn item(&self, id: ItemId) -> Option<&ItemDef> {
        self.items.get(id.0)
    }

    /// Returns the name of an item, or `"unknown"`.
    #[must_use]
    pub fn item_name(&self, id: ItemId) -> &str {
        self.items.name(id.0).unwrap_or("unknown")
    }

    /// Returns every item in id order.
    pub fn items(&self) -> impl Iterator<Item = (ItemId, &ItemDef)> + '_ {
        self.items.iter().map(|(id, def)| (ItemId(id), def))
    }

    /// Returns the items with a tag, in id order.
    pub fn items_with_tag<'a>(
        &'a self,
        tag: &'a str,
    ) -> impl Iterator<Item = (ItemId, &'a ItemDef)> + 'a {
        self.items().filter(move |(_, def)| def.has_tag(tag))
    }

    // ---- Attributes ----------------------------------------------------

    /// Returns the id of an attribute.
    #[must_use]
    pub fn attr_id(&self, name: &str) -> Option<AttrId> {
        self.attrs.id(name).map(AttrId)
    }

    /// Returns the definition of an attribute.
    ///
    /// # Panics
    ///
    /// Panics when the id is not from this blueprint.
    #[must_use]
    pub fn attr(&self, id: AttrId) -> &AttrDef {
        self.attrs.get(id.0).expect("attribute ids come from this blueprint")
    }

    /// Returns every attribute in id order.
    pub fn attrs(&self) -> impl Iterator<Item = (AttrId, &AttrDef)> + '_ {
        self.attrs.iter().map(|(id, def)| (AttrId(id), def))
    }

    // ---- Conditions ----------------------------------------------------

    /// Returns the id of a condition.
    #[must_use]
    pub fn condition_id(&self, name: &str) -> Option<CondId> {
        self.conds.id(name).map(CondId)
    }

    /// Returns the definition of a condition.
    ///
    /// # Panics
    ///
    /// Panics when the id is not from this blueprint.
    #[must_use]
    pub fn condition(&self, id: CondId) -> &CondDef {
        self.conds.get(id.0).expect("condition ids come from this blueprint")
    }

    /// Returns every condition in id order.
    pub fn conditions(&self) -> impl Iterator<Item = (CondId, &CondDef)> + '_ {
        self.conds.iter().map(|(id, def)| (CondId(id), def))
    }

    // ---- Faucets and sinks ---------------------------------------------

    /// Returns the id of a faucet.
    #[must_use]
    pub fn faucet_id(&self, name: &str) -> Option<FaucetId> {
        self.faucets.id(name).map(FaucetId)
    }

    /// Returns the name of a faucet, or `"unknown"`.
    #[must_use]
    pub fn faucet_name(&self, id: FaucetId) -> &str {
        self.faucets.name(id.0).unwrap_or("unknown")
    }

    /// Returns every faucet with its name, in id order.
    pub fn faucets(&self) -> impl Iterator<Item = (FaucetId, &str)> + '_ {
        (0..self.faucets.len() as u16)
            .filter_map(|id| self.faucets.name(id).map(|n| (FaucetId(id), n)))
    }

    /// Returns the id of a sink.
    #[must_use]
    pub fn sink_id(&self, name: &str) -> Option<SinkId> {
        self.sinks.id(name).map(SinkId)
    }

    /// Returns the name of a sink, or `"unknown"`.
    #[must_use]
    pub fn sink_name(&self, id: SinkId) -> &str {
        self.sinks.name(id.0).unwrap_or("unknown")
    }

    /// Returns every sink with its name, in id order.
    pub fn sinks(&self) -> impl Iterator<Item = (SinkId, &str)> + '_ {
        (0..self.sinks.len() as u16).filter_map(|id| self.sinks.name(id).map(|n| (SinkId(id), n)))
    }

    // ---- Verbs ---------------------------------------------------------

    /// Returns the public description of a verb, or `None` when no plugin
    /// registers it.
    #[must_use]
    pub fn verb_info(&self, name: &str) -> Option<VerbInfo> {
        let entry = self.verbs.get(self.verbs.id(name)?)?;
        if matches!(entry.kind, VerbKind::Retired) {
            return None;
        }
        Some(VerbInfo {
            verb: entry.name.to_string(),
            module: entry.module.to_string(),
            summary: entry.summary.to_string(),
            free: entry.free,
            scope: if entry.is_global() { VerbScope::Global } else { VerbScope::Zone },
            args_schema: entry.schema.clone(),
        })
    }

    /// Returns the names of all verbs that a resident can use, in id order.
    pub fn verb_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.verbs.iter().filter(|(_, v)| !matches!(v.kind, VerbKind::Retired)).map(|(_, v)| v.name)
    }

    /// Returns the systems of a scope in running order: name, stage and
    /// period in ticks. Profilers and documentation tools use it.
    pub fn systems(&self, scope: Scope) -> impl Iterator<Item = (&'static str, Stage, u32)> + '_ {
        self.scope(scope).systems.iter().map(|s| (s.name, s.stage, s.period))
    }

    /// Returns the names of the registered components, in column order.
    pub fn component_names(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.zone.components.iter().map(|c| c.name)
    }

    /// Builds the public catalogue. `value` gives the current national value
    /// of each parameter.
    pub(crate) fn catalogue(&self, value: impl Fn(ParamId) -> i64) -> Catalogue {
        let bound = |v: i64| (v != i64::MIN && v != i64::MAX).then_some(v);
        let mut verbs: Vec<VerbInfo> =
            self.verb_names().filter_map(|name| self.verb_info(name)).collect();
        verbs.sort_by(|a, b| {
            (a.module.as_str(), a.verb.as_str()).cmp(&(b.module.as_str(), b.verb.as_str()))
        });
        let mut params: Vec<ParamInfo> = self
            .params()
            .map(|(id, def)| ParamInfo {
                name: def.name.clone(),
                value: value(id),
                min: bound(def.min),
                max: bound(def.max),
                policy: def.policy,
                summary: def.summary.clone(),
            })
            .collect();
        params.sort_by(|a, b| a.name.cmp(&b.name));
        let items = self
            .items()
            .map(|(id, def)| ItemInfo {
                id,
                name: def.name.clone(),
                weight: def.weight,
                tags: def.tags.clone(),
                attrs: def.attrs.clone(),
                summary: def.summary.clone(),
            })
            .collect();
        let mut attrs: Vec<AttrInfo> = self
            .attrs()
            .filter(|(_, def)| def.show != AttrShow::Hidden)
            .map(|(_, def)| AttrInfo {
                name: def.name.clone(),
                min: bound(def.min),
                max: bound(def.max),
                default: def.default,
                summary: def.summary.clone(),
            })
            .collect();
        attrs.sort_by(|a, b| a.name.cmp(&b.name));
        let mut conditions: Vec<CondInfo> = self
            .conditions()
            .map(|(_, def)| CondInfo {
                name: def.name.clone(),
                blocking: def.blocking,
                summary: def.summary.clone(),
            })
            .collect();
        conditions.sort_by(|a, b| a.name.cmp(&b.name));
        Catalogue { verbs, params, items, attrs, conditions }
    }
}

/// Reads the arguments of a verb from JSON. An absent value (`null`) reads as
/// an empty object, so a verb without arguments needs none.
fn parse_args<A: Args>(args: &Value) -> Result<A, ActionError> {
    let result = if args.is_null() {
        A::deserialize(&Value::Object(serde_json::Map::new()))
    } else {
        A::deserialize(args)
    };
    result.map_err(|e| ActionError::invalid_args(format!("Invalid arguments: {e}.")))
}

/// Registers what a plugin adds to the kernel.
///
/// Definitions (parameters, items, attributes, conditions, faucets, sinks)
/// are shared by both scopes and use *ensure* semantics: the first
/// definition of a name wins and a later call returns the same id. So a
/// plugin can declare its data in both passes, and two plugins can share an
/// attribute by name.
///
/// State and behaviour belong to one scope. Methods that exist only for
/// zones (`component`, `verb`, `instant_verb`, `on_resident_message`,
/// `observe`, `on_spawn`, `on_arrive`, `on_depart`, `on_despawn`) do nothing
/// in the global pass, and the methods that exist only for the global realm
/// (`global_verb`, `observe_global`) do nothing in the zone pass. Every
/// other method registers for the scope of the current pass; test
/// [`Builder::scope`] to choose.
///
/// A mistake in a plugin (a verb registered twice, a data file that does not
/// parse, an unknown name) is a bug and panics when the world is made.
pub struct Builder {
    scope: Scope,
    plugin: &'static str,
    seq: u32,
    bp: Blueprint,
}

impl Builder {
    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// Returns the scope of the current pass.
    #[must_use]
    pub fn scope(&self) -> Scope {
        self.scope
    }

    // ---- Data ----------------------------------------------------------

    /// Parses a data file (parameters and items, the format of
    /// `data/balance.toml`) and registers what it declares. A name that is
    /// already defined keeps its first definition.
    ///
    /// # Panics
    ///
    /// Panics when the text is not a valid data file.
    pub fn load_data(&mut self, toml: &str) {
        let plugin = self.plugin;
        let file = DataFile::parse(toml).unwrap_or_else(|e| panic!("plugin `{plugin}`: {e}"));
        for (name, data) in file.params {
            if self.bp.params.id(&name).is_some_and(|id| self.bp.params.get(id).is_some()) {
                continue;
            }
            let def = data.into_def(&name).unwrap_or_else(|e| panic!("plugin `{plugin}`: {e}"));
            self.bp.params.ensure(&name, def);
        }
        for data in file.items {
            if self.bp.items.id(&data.name).is_some_and(|id| self.bp.items.get(id).is_some()) {
                continue;
            }
            let def = data.into_def().unwrap_or_else(|e| panic!("plugin `{plugin}`: {e}"));
            let name = def.name.clone();
            self.bp.items.ensure(&name, def);
        }
    }

    /// Registers a parameter. The same name returns the same id.
    ///
    /// # Panics
    ///
    /// Panics when the default value is outside the range.
    pub fn param(&mut self, def: ParamDef) -> ParamId {
        if let Err(e) = check_param(&def) {
            panic!("plugin `{}`: {e}", self.plugin);
        }
        let name = def.name.clone();
        ParamId(self.bp.params.ensure(&name, def))
    }

    /// Returns the id of a parameter that is already registered.
    ///
    /// # Panics
    ///
    /// Panics when no parameter has this name.
    #[must_use]
    pub fn param_id(&self, name: &str) -> ParamId {
        match self.bp.params.id(name) {
            Some(id) => ParamId(id),
            None => panic!("plugin `{}`: unknown parameter `{name}`", self.plugin),
        }
    }

    /// Registers an item. The same name returns the same id.
    pub fn item(&mut self, def: ItemDef) -> ItemId {
        let name = def.name.clone();
        ItemId(self.bp.items.ensure(&name, def))
    }

    /// Returns the id of an item that is already registered.
    ///
    /// # Panics
    ///
    /// Panics when no item has this name.
    #[must_use]
    pub fn item_id(&self, name: &str) -> ItemId {
        match self.bp.items.id(name) {
            Some(id) => ItemId(id),
            None => panic!("plugin `{}`: unknown item `{name}`", self.plugin),
        }
    }

    /// Registers an attribute. The same name returns the same id.
    ///
    /// # Panics
    ///
    /// Panics when the default value is outside the bounds.
    pub fn attr(&mut self, def: AttrDef) -> AttrId {
        assert!(
            def.min <= def.default && def.default <= def.max,
            "plugin `{}`: attribute `{}` has default {} outside {}..={}",
            self.plugin,
            def.name,
            def.default,
            def.min,
            def.max
        );
        let name = def.name.clone();
        AttrId(self.bp.attrs.ensure(&name, def))
    }

    /// Returns the id of an attribute that is already registered.
    ///
    /// # Panics
    ///
    /// Panics when no attribute has this name.
    #[must_use]
    pub fn attr_id(&self, name: &str) -> AttrId {
        match self.bp.attrs.id(name) {
            Some(id) => AttrId(id),
            None => panic!("plugin `{}`: unknown attribute `{name}`", self.plugin),
        }
    }

    /// Registers a condition. The same name returns the same id.
    pub fn condition(&mut self, def: CondDef) -> CondId {
        let name = def.name.clone();
        CondId(self.bp.conds.ensure(&name, def))
    }

    /// Returns the id of a condition that is already registered.
    ///
    /// # Panics
    ///
    /// Panics when no condition has this name.
    #[must_use]
    pub fn condition_id(&self, name: &str) -> CondId {
        match self.bp.conds.id(name) {
            Some(id) => CondId(id),
            None => panic!("plugin `{}`: unknown condition `{name}`", self.plugin),
        }
    }

    /// Registers a faucet: a named source of money or goods. The same name
    /// returns the same id.
    pub fn faucet(&mut self, name: &'static str) -> FaucetId {
        FaucetId(self.bp.faucets.ensure(name, ()))
    }

    /// Registers a sink: a named drain of money or goods. The same name
    /// returns the same id.
    pub fn sink(&mut self, name: &'static str) -> SinkId {
        SinkId(self.bp.sinks.ensure(name, ()))
    }

    // ---- State ---------------------------------------------------------

    /// Registers a component: a typed column of the resident table. Zone
    /// scope only.
    ///
    /// # Panics
    ///
    /// Panics when another type uses the same `NAME`.
    pub fn component<T: Component>(&mut self) {
        if self.scope != Scope::Zone {
            return;
        }
        let type_id = TypeId::of::<T>();
        if self.bp.zone.comp_index.contains_key(&type_id) {
            return;
        }
        assert!(
            self.bp.zone.components.iter().all(|c| c.name != T::NAME),
            "plugin `{}`: component name `{}` is used by two types",
            self.plugin,
            T::NAME
        );
        self.bp.zone.comp_index.insert(type_id, self.bp.zone.components.len());
        self.bp.zone.components.push(ComponentEntry { name: T::NAME, make: new_column::<T> });
    }

    /// Registers a resource: a typed singleton in every realm of the current
    /// scope.
    ///
    /// # Panics
    ///
    /// Panics when another type uses the same `NAME`.
    pub fn resource<T: Resource>(&mut self) {
        let plugin = self.plugin;
        let scope = match self.scope {
            Scope::Zone => &mut self.bp.zone,
            Scope::Global => &mut self.bp.global,
        };
        let type_id = TypeId::of::<T>();
        if scope.res_index.contains_key(&type_id) {
            return;
        }
        assert!(
            scope.resources.iter().all(|r| r.name != T::NAME),
            "plugin `{plugin}`: resource name `{}` is used by two types",
            T::NAME
        );
        scope.res_index.insert(type_id, scope.resources.len());
        scope.resources.push(ResourceEntry { name: T::NAME, make: new_resource::<T> });
    }

    // ---- Behaviour -----------------------------------------------------

    fn scope_mut(&mut self) -> &mut ScopeDef {
        match self.scope {
            Scope::Zone => &mut self.bp.zone,
            Scope::Global => &mut self.bp.global,
        }
    }

    /// Registers a system that runs every tick in the realms of the current
    /// scope. Systems run by stage, then by `order`, then in registration
    /// order.
    pub fn system(
        &mut self,
        stage: Stage,
        order: i32,
        name: &'static str,
        f: impl Fn(&Realm) + Send + Sync + 'static,
    ) {
        self.every(1, stage, order, name, f);
    }

    /// Registers a system that runs every `period` ticks.
    ///
    /// The system runs when `(tick + offset) % period == 0`. The offset is 0
    /// in the global realm and the zone number in a zone, so that the zones
    /// do not all do their periodic work in the same frame.
    ///
    /// # Panics
    ///
    /// Panics when `period` is 0.
    pub fn every(
        &mut self,
        period: u32,
        stage: Stage,
        order: i32,
        name: &'static str,
        f: impl Fn(&Realm) + Send + Sync + 'static,
    ) {
        assert!(period > 0, "plugin `{}`: system `{name}` has period 0", self.plugin);
        let seq = self.next_seq();
        self.scope_mut().systems.push(SystemEntry {
            stage,
            order,
            seq,
            name,
            period,
            run: Box::new(f),
        });
    }

    fn add_verb(&mut self, spec: VerbSpec, schema: Value, validate: ValidateFn, kind: VerbKind) {
        let plugin = self.plugin;
        assert!(
            !spec.verb.is_empty()
                && spec
                    .verb
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "plugin `{plugin}`: verb name `{}` is not snake_case",
            spec.verb
        );
        assert!(
            !self.bp.verbs.id(spec.verb).is_some_and(|id| self.bp.verbs.get(id).is_some()),
            "plugin `{plugin}`: verb `{}` is registered twice",
            spec.verb
        );
        self.bp.verbs.ensure(
            spec.verb,
            VerbEntry {
                name: spec.verb,
                module: spec.module,
                summary: spec.summary,
                free: spec.free,
                allow_all: false,
                allowed_names: spec.allowed_in,
                allowed: Vec::new(),
                schema,
                validate,
                kind,
            },
        );
    }

    /// Registers a verb that runs in the resident's zone. Zone scope only.
    ///
    /// `begin` runs when the action starts. It checks the request and says
    /// what happens next ([`Begin`]). A `begin` that returns an error must
    /// not have changed anything. `finish` runs when a `Begin::Busy` action
    /// ends; its result goes to the action report.
    ///
    /// # Panics
    ///
    /// Panics when the verb name is taken or is not `snake_case`.
    pub fn verb<A: Args>(
        &mut self,
        spec: VerbSpec,
        begin: impl Fn(&Realm, &ActionCtx, &A) -> Result<Begin, ActionError> + Send + Sync + 'static,
        finish: impl Fn(&Realm, &ActionCtx, &A) -> Result<Value, ActionError> + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Zone {
            return;
        }
        let schema = schemars::schema_for!(A).to_value();
        let kind = VerbKind::Zone {
            begin: Box::new(move |realm, ctx, args| begin(realm, ctx, &parse_args::<A>(args)?)),
            finish: Box::new(move |realm, ctx, args| finish(realm, ctx, &parse_args::<A>(args)?)),
        };
        self.add_verb(spec, schema, Box::new(|args| parse_args::<A>(args).map(|_| ())), kind);
    }

    /// Registers a zone verb that always finishes at once. Zone scope only.
    ///
    /// # Panics
    ///
    /// Panics when the verb name is taken or is not `snake_case`.
    pub fn instant_verb<A: Args>(
        &mut self,
        spec: VerbSpec,
        run: impl Fn(&Realm, &ActionCtx, &A) -> Result<Value, ActionError> + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Zone {
            return;
        }
        let schema = schemars::schema_for!(A).to_value();
        let kind = VerbKind::Zone {
            begin: Box::new(move |realm, ctx, args| {
                run(realm, ctx, &parse_args::<A>(args)?).map(Begin::Done)
            }),
            finish: Box::new(|_, _, _| Ok(Value::Null)),
        };
        self.add_verb(spec, schema, Box::new(|args| parse_args::<A>(args).map(|_| ())), kind);
    }

    /// Registers a verb that runs in the global realm. Global scope only.
    ///
    /// The zone checks the arguments and forwards the request. `run` is
    /// called in the global realm in the same frame, and the answer reaches
    /// the resident in the next frame as an action report.
    ///
    /// # Panics
    ///
    /// Panics when the verb name is taken or is not `snake_case`.
    pub fn global_verb<A: Args>(
        &mut self,
        spec: VerbSpec,
        run: impl Fn(&Realm, &GlobalCtx, &A) -> Result<Value, ActionError> + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Global {
            return;
        }
        let schema = schemars::schema_for!(A).to_value();
        let kind = VerbKind::Global {
            run: Box::new(move |realm, ctx, args| run(realm, ctx, &parse_args::<A>(args)?)),
        };
        self.add_verb(spec, schema, Box::new(|args| parse_args::<A>(args).map(|_| ())), kind);
    }

    /// Marks a verb as a kernel verb: no module condition blocks it.
    pub(crate) fn allow_everywhere(&mut self, verb: &str) {
        if let Some(entry) = self.bp.verbs.id(verb).and_then(|id| self.bp.verbs.get_mut(id)) {
            entry.allow_all = true;
        }
    }

    fn note_message_type<M: Message>(&mut self) {
        let known =
            self.bp.message_types.entry(M::KIND).or_insert((TypeId::of::<M>(), type_name::<M>()));
        assert!(
            known.0 == TypeId::of::<M>(),
            "plugin `{}`: message kind `{}` is used by both {} and {}",
            self.plugin,
            M::KIND,
            known.1,
            type_name::<M>()
        );
    }

    /// Registers a handler for a message type in the realms of the current
    /// scope. The handler gets the realm that sent the message.
    ///
    /// A payload that does not decode into `M` is a bug of the module that
    /// sent it, and the delivery panics. The exception is a message that
    /// came through `Input::Deliver`: the kernel drops it and counts it in
    /// `World::dropped_messages`, and the handler does not run.
    ///
    /// # Panics
    ///
    /// Panics when another type uses the same `KIND`.
    pub fn on_message<M: Message>(
        &mut self,
        f: impl Fn(&Realm, RealmId, M) + Send + Sync + 'static,
    ) {
        self.note_message_type::<M>();
        let handler: MessageFn = Box::new(move |realm, from, bytes| {
            f(realm, from, codec::decode::<M>(M::KIND, bytes)?);
            Ok(())
        });
        self.scope_mut().messages.entry(M::KIND).or_default().push(handler);
    }

    /// Registers a handler for a message that was sent to a resident
    /// (`Dest::Resident`). It runs in the zone where the resident is. Zone
    /// scope only.
    ///
    /// # Panics
    ///
    /// Panics when another type uses the same `KIND`.
    pub fn on_resident_message<M: Message>(
        &mut self,
        f: impl Fn(&Realm, Idx, RealmId, M) + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Zone {
            return;
        }
        self.note_message_type::<M>();
        let handler: ResidentMessageFn = Box::new(move |realm, idx, from, bytes| {
            f(realm, idx, from, codec::decode::<M>(M::KIND, bytes)?);
            Ok(())
        });
        self.bp.zone.resident_messages.entry(M::KIND).or_default().push(handler);
    }

    /// Registers a handler for events whose kind starts with `kind_prefix`,
    /// in the realms of the current scope.
    ///
    /// A zone handler gets the events of its own zone, later in the same
    /// tick. A global handler gets the events of the global realm and of
    /// every zone, in the same frame.
    pub fn on_event(
        &mut self,
        kind_prefix: &'static str,
        f: impl Fn(&Realm, &Event) + Send + Sync + 'static,
    ) {
        self.scope_mut().events.push((kind_prefix, Box::new(f)));
    }

    fn claim_section(&mut self, section: &'static str) {
        assert!(
            !matches!(section, "self" | "nearby" | "reports"),
            "plugin `{}`: observation section `{section}` belongs to the kernel",
            self.plugin
        );
        if let Some(owner) = self.bp.sections.insert(section, self.plugin) {
            panic!(
                "plugin `{}`: observation section `{section}` is already registered by `{owner}`",
                self.plugin
            );
        }
    }

    /// Registers an observer that writes one section of the observation of a
    /// resident from the state of its zone. Zone scope only. Return `None`
    /// to leave the section out. An observer must not change state.
    ///
    /// # Panics
    ///
    /// Panics when the section name is taken.
    pub fn observe(
        &mut self,
        section: &'static str,
        order: i32,
        f: impl Fn(&Realm, Idx, &ObserveOpts) -> Option<Value> + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Zone {
            return;
        }
        self.claim_section(section);
        let seq = self.next_seq();
        self.bp.zone.observers.push(Observer { section, order, seq, run: Box::new(f) });
    }

    /// Registers an observer that writes one section of the observation of a
    /// resident from the state of the global realm. Global scope only.
    ///
    /// # Panics
    ///
    /// Panics when the section name is taken.
    pub fn observe_global(
        &mut self,
        section: &'static str,
        order: i32,
        f: impl Fn(&Realm, ResidentId, &ObserveOpts) -> Option<Value> + Send + Sync + 'static,
    ) {
        if self.scope != Scope::Global {
            return;
        }
        self.claim_section(section);
        let seq = self.next_seq();
        self.bp.global.global_observers.push(Observer { section, order, seq, run: Box::new(f) });
    }

    /// Registers a hook that runs once in every realm of the current scope
    /// when the world is made. Hooks run by `order`, then in registration
    /// order.
    pub fn on_worldgen(
        &mut self,
        order: i32,
        f: impl Fn(&Realm, &WorldGen<'_>) + Send + Sync + 'static,
    ) {
        let seq = self.next_seq();
        self.scope_mut().worldgen.push(Ordered { order, seq, run: Box::new(f) });
    }

    /// Registers a hook that runs when a new resident appears. Zone scope
    /// only. Hooks run by `order`, then in registration order.
    pub fn on_spawn(&mut self, order: i32, f: impl Fn(&Realm, Idx) + Send + Sync + 'static) {
        if self.scope != Scope::Zone {
            return;
        }
        let seq = self.next_seq();
        self.bp.zone.on_spawn.push(Ordered { order, seq, run: Box::new(f) });
    }

    /// Registers a hook that runs after a resident arrived from another zone.
    /// Zone scope only.
    pub fn on_arrive(&mut self, f: impl Fn(&Realm, Idx) + Send + Sync + 'static) {
        if self.scope == Scope::Zone {
            self.bp.zone.on_arrive.push(Box::new(f));
        }
    }

    /// Registers a hook that runs before a resident leaves for another zone.
    /// Zone scope only.
    pub fn on_depart(&mut self, f: impl Fn(&Realm, Idx) + Send + Sync + 'static) {
        if self.scope == Scope::Zone {
            self.bp.zone.on_depart.push(Box::new(f));
        }
    }

    /// Registers a hook that runs before a resident is removed from the
    /// world. Zone scope only.
    pub fn on_despawn(&mut self, f: impl Fn(&Realm, Idx) + Send + Sync + 'static) {
        if self.scope == Scope::Zone {
            self.bp.zone.on_despawn.push(Box::new(f));
        }
    }
}
