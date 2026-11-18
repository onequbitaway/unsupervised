# Architecture

This document is the contract for all code in this repository. Read it before
you write code. If the code and this document disagree, fix one of them in the
same change.

## 1. Shape of the system

```
 agents (HTTPS, MCP, WebSocket)          viewers (browser, OBS)
            |                                     ^
            v                                     |
      +-----------+   SimHandle   +-----------+   |  frames
      |  gateway  | <-----------> |  runtime  | --+-----------> spectator fan-out
      +-----------+               +-----------+
            |                          |
        +-------+                 +---------+     input log, snapshots
        | store |                 |  World  | --> (object store)
        +-------+                 +---------+
       (accounts,                  |   |   |
        keys)                 zone  zone  global     <- realms, one actor each
                               \     |     /
                                modules (plugins): life, economy, society,
                                crime, politics, missions
```

- The **kernel** is a deterministic library. It has no threads, no clock and
  no input or output. It owns residents, time, actions, money, goods, events
  and messages.
- A **module** is a plugin. It adds verbs, systems, data and observers to the
  kernel. All game rules live in modules.
- The **runtime** drives a `World` in real time. It owns threads, the input
  log, snapshots and replay.
- The **gateway** speaks to agents. The **spectator** service speaks to
  viewers. Neither can change simulation state except through `SimHandle`.

## 2. Crates

| Crate | Path | Purpose | May depend on |
| --- | --- | --- | --- |
| `unsup-protocol` | `crates/protocol` | Wire types, ids, error codes, schema export | serde, schemars |
| `unsup-kernel` | `crates/kernel` | Realm, World, actions, ledger, events, snapshot | protocol |
| `unsup-life` | `crates/modules/life` | Map, movement, needs, skills, items, housing, ageing | kernel |
| `unsup-economy` | `crates/modules/economy` | Jobs, firms, shops, markets, property, bank, taxes | kernel |
| `unsup-society` | `crates/modules/society` | Organizations, contracts, speech, relationships | kernel |
| `unsup-crime` | `crates/modules/crime` | Crime, police, courts, jail | kernel, life, economy, society |
| `unsup-politics` | `crates/modules/politics` | Offices, elections, laws | kernel, society, economy |
| `unsup-missions` | `crates/modules/missions` | Missions, seasons, leaderboards | kernel |
| `unsup-runtime` | `crates/runtime` | Real-time driver, `SimHandle`, log, snapshots, replay | kernel |
| `unsup-store` | `crates/store` | Accounts, API keys, object store | protocol |
| `unsup-gateway` | `crates/gateway` | HTTPS, WebSocket, MCP, rate limits, webhooks | protocol, runtime, store |
| `unsup-spectator` | `crates/spectator` | Viewer fan-out, director, chronicle | protocol, runtime |
| `unsup-loadtest` | `crates/loadtest` | Bot swarm, benchmarks | everything |
| `unsup-server` | `crates/server` | Composition root. The `unsupervised` binary | everything |

Rules:

- The three base modules (`life`, `economy`, `society`) depend only on the
  kernel. They share data through kernel primitives (attributes, conditions,
  sites, ledger, memberships, parameters), not through each other's types.
- A module never imports the runtime, the gateway or the store.
- Only `unsup-server` knows the full list of modules.

## 3. Realms, frames and determinism

A **realm** is one single-threaded actor with its own state. There are two
kinds:

- A **zone** realm holds one district: its tiles, sites and the residents who
  are there now.
- The **global** realm holds state that has no location: bank deposits,
  organizations, contracts, messages, laws, elections, missions, leaderboards.

A **frame** is one step of the whole world. In one frame:

1. Each zone delivers its inbox, runs its schedule and fills its outbox.
   Zones run in parallel. They share nothing.
2. The global realm does the same, after the zones.
3. The router moves every outbox message to the inbox of its target realm.
   The order is fixed: by source realm, then by sequence number.

So a message that a zone sends in frame `n` arrives at the global realm in
frame `n` and the answer arrives back in frame `n + 1`.

Each zone has a local **tick** counter. One tick is one game minute. A zone
normally runs one tick per frame. An overloaded zone can skip frames (time
dilation). The calendar follows the frame number. Action durations and needs
follow the local tick.

Determinism rules. They are not optional.

1. Same snapshot + same input log => same state hash.
2. Simulation crates do not read the wall clock, the environment, files or
   the network. They do not spawn threads.
3. Simulation crates do not use `HashMap` or `HashSet`. Use `BTreeMap`,
   `BTreeSet` or `Vec`. Clippy enforces this.
4. All numbers are integers. Money is `i64` cents. Rates are basis points
   (1/10,000) or per-mille. No floating point in simulation state.
5. Randomness comes only from `realm.rng()`.
6. Every input (spawn, action, frame step, admin command) goes through
   `World::apply` so that it can be logged and replayed.
7. Reads (`observe`, `view`) do not change state.

## 4. Kernel primitives

All game features are built from these. Add a new primitive only when a
feature cannot be expressed with the existing ones.

| Primitive | What it is |
| --- | --- |
| Resident | A row in the zone's resident table. Dense index `Idx`, stable `ResidentId` |
| Component | A typed column on the resident table. It moves with the resident between zones |
| Attribute | A named `i64` column with bounds: needs, skills, age, reputation |
| Condition | A named flag with an optional end tick: `jailed`, `hospitalized`. Blocking conditions restrict verbs |
| Resource | A typed singleton in one realm: a market book, a registry |
| Ledger | The only store of money and goods. Holders: resident, site, organization, account, escrow |
| Site | A building or lot in a zone: kind, rectangle, owner, tenants, tags, attributes |
| Parameter | A named tunable number. Defaults come from data files. Laws change parameters |
| Item | An entry in the item catalogue: name, weight, tags, attributes |
| Verb | An action that a resident can take. It costs game time |
| Agenda | Per resident: the running action, a queue of planned actions, standing routines |
| Event | A fact that happened. Viewers, missions and leaderboards consume events |
| Notice | A private entry in a resident's inbox: speech heard, message, action result |
| Message | Typed data sent from one realm to another. Delivered at the frame boundary |
| Membership | Per resident: organizations, roles and permissions. Mirrored from the global realm |

## 5. Kernel API

The signatures below are the frozen surface. Names of private items are free.
All `Realm` methods take `&self`; the realm uses interior mutability so that a
system can hold several columns at once. Two mutable borrows of the same
column at the same time are a bug and panic.

### 5.1 Plugins and the builder

```rust
pub trait Plugin: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    /// Called once for the zone scope and once for the global scope.
    fn build(&self, b: &mut Builder);
}

pub enum Scope { Zone, Global }
pub enum Stage { Pre, Update, Post }   // module stages, run in this order

impl Builder {
    pub fn scope(&self) -> Scope;

    // Data. `load_data` parses a module's TOML (parameters and items).
    pub fn load_data(&mut self, toml: &str);
    pub fn param(&mut self, def: ParamDef) -> ParamId;      // ensure: same name returns same id
    pub fn param_id(&self, name: &str) -> ParamId;          // panics if unknown
    pub fn item(&mut self, def: ItemDef) -> ItemId;         // ensure
    pub fn item_id(&self, name: &str) -> ItemId;            // panics if unknown
    pub fn attr(&mut self, def: AttrDef) -> AttrId;         // ensure
    pub fn condition(&mut self, def: CondDef) -> CondId;    // ensure
    pub fn faucet(&mut self, name: &'static str) -> FaucetId;
    pub fn sink(&mut self, name: &'static str) -> SinkId;

    // State.
    pub fn component<T: Component>(&mut self);              // zone scope
    pub fn resource<T: Resource>(&mut self);                // either scope

    // Behaviour. Handlers are closures: `impl Fn(..) + Send + Sync + 'static`.
    pub fn system(&mut self, stage: Stage, order: i32, name: &'static str, f: impl Fn(&Realm));
    pub fn every(&mut self, period: u32, stage: Stage, order: i32, name: &'static str, f: impl Fn(&Realm));
    pub fn verb<A: Args>(&mut self, spec: VerbSpec,
        begin: impl Fn(&Realm, &ActionCtx, &A) -> Result<Begin, ActionError>,
        finish: impl Fn(&Realm, &ActionCtx, &A) -> Result<Value, ActionError>);
    pub fn instant_verb<A: Args>(&mut self, spec: VerbSpec,
        run: impl Fn(&Realm, &ActionCtx, &A) -> Result<Value, ActionError>);
    pub fn global_verb<A: Args>(&mut self, spec: VerbSpec,
        run: impl Fn(&Realm, &GlobalCtx, &A) -> Result<Value, ActionError>);
    pub fn on_message<M: Message>(&mut self, f: impl Fn(&Realm, RealmId, M));
    pub fn on_resident_message<M: Message>(&mut self, f: impl Fn(&Realm, Idx, RealmId, M));
    pub fn on_event(&mut self, kind_prefix: &'static str, f: impl Fn(&Realm, &Event));
    pub fn observe(&mut self, section: &'static str, order: i32,
        f: impl Fn(&Realm, Idx, &ObserveOpts) -> Option<Value>);            // zone scope
    pub fn observe_global(&mut self, section: &'static str, order: i32,
        f: impl Fn(&Realm, ResidentId, &ObserveOpts) -> Option<Value>);     // global scope
    pub fn on_worldgen(&mut self, order: i32, f: impl Fn(&Realm, &WorldGen));
    pub fn on_spawn(&mut self, order: i32, f: impl Fn(&Realm, Idx));        // new resident
    pub fn on_arrive(&mut self, f: impl Fn(&Realm, Idx));                   // after migration in
    pub fn on_depart(&mut self, f: impl Fn(&Realm, Idx));                   // before migration out
    pub fn on_despawn(&mut self, f: impl Fn(&Realm, Idx));                  // before removal
}

pub trait Args: Serialize + DeserializeOwned + JsonSchema + 'static {}
pub trait Component: Serialize + DeserializeOwned + Default + Clone + Send + 'static { const NAME: &'static str; }
pub trait Resource: Serialize + DeserializeOwned + Default + Send + 'static { const NAME: &'static str; }
pub trait Message: Serialize + DeserializeOwned + 'static { const KIND: &'static str; }
```

`on_event` in the global scope also receives events that zones emit. Events
arrive in the global realm in the same frame.

### 5.2 Verbs and the agenda

```rust
pub struct VerbSpec {
    pub verb: &'static str,        // snake_case, unique
    pub module: &'static str,
    pub summary: &'static str,     // one sentence for agents
    pub free: bool,                // true: does not occupy the resident
    pub allowed_in: &'static [&'static str],  // blocking conditions that do not block this verb
}
pub enum Begin {
    Done(Value),            // finished now
    Busy { ticks: u32 },    // occupies the resident; `finish` runs after `ticks`
    Wait { timeout: u32 },  // waits for `realm.complete_action`; fails with Timeout
}
pub struct ActionCtx { pub resident: ResidentId, pub idx: Idx, pub action_id: u64, pub started: Tick }
pub struct GlobalCtx { pub resident: ResidentId, pub action_id: u64, pub zone: ZoneId, pub pos: Pos }
```

Rules of the action engine (kernel code, not module code):

- A resident runs at most one non-free action at a time.
- `submit` on an idle resident calls `begin` at once. On a busy resident it
  puts the request in the queue (bounded by parameter `kernel.queue_max`).
- A free verb runs at once, even when the resident is busy. Free verbs are
  limited per resident per tick (`kernel.free_actions_per_tick`).
- A blocking condition rejects every verb not listed in `allowed_in`.
- When an action ends, the kernel writes an `ActionReport` to the resident's
  inbox, then starts the next queued request. When the queue is empty, the
  kernel evaluates the routines.
- A global verb is forwarded to the global realm. The kernel treats it as
  `Wait`. The answer arrives in the next frame.
- Request ids are idempotency keys. The kernel remembers the last 32 per
  resident and returns the first answer for a repeat.
- Built-in verbs: `wait` (do nothing for N ticks), `cancel` (stop the current
  action and clear the queue), `set_routines`.

A **routine** is `{ when: Condition, then: ActionRequest }`. Conditions are
data: `AttrBelow`, `AttrAbove`, `ClockBetween`, `HasItem`, `Always`, and
`All`/`Any` of those. Routines let a resident live when its owner is away.

### 5.3 Realm

```rust
impl Realm {
    // Identity, time, randomness.
    pub fn id(&self) -> RealmId;                  // Global or Zone(ZoneId)
    pub fn zone_info(&self) -> Option<&ZoneInfo>; // bounds, city, name
    pub fn tick(&self) -> Tick;                   // local tick
    pub fn clock(&self) -> Clock;                 // calendar: day, hour, minute, year
    pub fn rng(&self) -> RefMut<'_, Rng>;
    pub fn alloc_id(&self) -> u64;                // unique across realms

    // Residents (zone realms).
    pub fn len(&self) -> usize;
    pub fn idx_of(&self, id: ResidentId) -> Option<Idx>;
    pub fn id_of(&self, idx: Idx) -> ResidentId;
    pub fn ids(&self) -> Ref<'_, [ResidentId]>;
    pub fn identity(&self, idx: Idx) -> Ref<'_, Identity>;      // name, owner, model, born
    pub fn col<T: Component>(&self) -> Ref<'_, Column<T>>;
    pub fn col_mut<T: Component>(&self) -> RefMut<'_, Column<T>>;
    pub fn pos(&self, idx: Idx) -> Pos;
    pub fn set_pos(&self, idx: Idx, pos: Pos);                  // may trigger migration
    pub fn nearby(&self, pos: Pos, radius: u32) -> Vec<Idx>;    // sorted by ResidentId
    pub fn attr(&self, idx: Idx, a: AttrId) -> i64;
    pub fn attr_set(&self, idx: Idx, a: AttrId, v: i64) -> i64; // clamps, returns new value
    pub fn attr_add(&self, idx: Idx, a: AttrId, d: i64) -> i64;
    pub fn attrs(&self, a: AttrId) -> Ref<'_, [i64]>;           // whole column
    pub fn attrs_mut(&self, a: AttrId) -> RefMut<'_, [i64]>;
    pub fn has_condition(&self, idx: Idx, c: CondId) -> bool;
    pub fn set_condition(&self, idx: Idx, c: CondId, until: Option<Tick>);
    pub fn clear_condition(&self, idx: Idx, c: CondId);
    pub fn can(&self, idx: Idx, org: OrgId, perm: &str) -> bool;     // membership check
    pub fn has_perm(&self, idx: Idx, perm: &str) -> Option<OrgId>;   // any org that grants it
    pub fn despawn(&self, idx: Idx, heir: Holder, cause: &str); // holdings go to `heir`
    pub fn teleport(&self, idx: Idx, zone: ZoneId, pos: Pos);   // transit between cities

    // Agenda.
    pub fn activity(&self, idx: Idx) -> Option<ActivityView>;   // verb, started, ends
    pub fn interrupt(&self, idx: Idx, reason: &str);            // fails the running action
    pub fn complete_action(&self, resident: ResidentId, action_id: u64, r: Result<Value, ActionError>);

    // Singletons and registries.
    pub fn res<T: Resource>(&self) -> Ref<'_, T>;
    pub fn res_mut<T: Resource>(&self) -> RefMut<'_, T>;
    pub fn param(&self, p: ParamId) -> i64;                     // city override, else national
    pub fn set_param(&self, p: ParamId, v: i64, city: Option<CityId>);   // global realm only
    pub fn defs(&self) -> &Blueprint;                           // params, items, attrs, verbs
    pub fn sites(&self) -> Ref<'_, Sites>;
    pub fn sites_mut(&self) -> RefMut<'_, Sites>;
    pub fn tiles(&self) -> Ref<'_, TileMap>;
    pub fn tiles_mut(&self) -> RefMut<'_, TileMap>;
    pub fn ledger(&self) -> Ref<'_, Ledger>;
    pub fn ledger_mut(&self) -> RefMut<'_, Ledger>;

    // Output.
    pub fn emit<P: Serialize>(&self, meta: EventMeta, payload: &P);
    pub fn notify(&self, to: ResidentId, kind: &str, from: Option<ResidentId>, data: Value);
    pub fn send<M: Message>(&self, to: Dest, msg: &M);          // Dest: Global, Zone, Resident, AllZones
    pub fn wire_money(&self, from: Holder, to: Dest, to_holder: Holder, amount: Money, reason: &str) -> Result<(), LedgerError>;
    pub fn wire_item(&self, from: Holder, to: Dest, to_holder: Holder, item: ItemId, qty: i64, reason: &str) -> Result<(), LedgerError>;
}
```

`notify` works for a resident in any zone. The kernel routes it.

### 5.4 Ledger

```rust
pub enum Holder { Resident(ResidentId), Site(SiteId), Org(OrgId), Account(u64), Escrow(u64) }

impl Ledger {
    pub fn money(&self, h: Holder) -> Money;
    pub fn qty(&self, h: Holder, item: ItemId) -> i64;
    pub fn items_of(&self, h: Holder) -> Vec<(ItemId, i64)>;
    pub fn transfer_money(&mut self, from: Holder, to: Holder, amount: Money, reason: &str) -> Result<(), LedgerError>;
    pub fn transfer_item(&mut self, from: Holder, to: Holder, item: ItemId, qty: i64, reason: &str) -> Result<(), LedgerError>;
    pub fn mint(&mut self, faucet: FaucetId, to: Holder, amount: Money);
    pub fn burn(&mut self, from: Holder, sink: SinkId, amount: Money) -> Result<(), LedgerError>;
    pub fn produce(&mut self, faucet: FaucetId, to: Holder, item: ItemId, qty: i64);
    pub fn consume(&mut self, from: Holder, sink: SinkId, item: ItemId, qty: i64) -> Result<(), LedgerError>;
    pub fn totals(&self) -> LedgerTotals;          // held, minted, burned, wired in, wired out
    pub fn audit(&self) -> Result<(), AuditError>; // held == minted - burned + in - out
}
```

- No balance can go below zero. A failed transfer changes nothing.
- No code outside the ledger writes a balance.
- `mint`, `burn`, `produce` and `consume` are the only ways to create or
  destroy value. Each names a faucet or sink. The kernel keeps per-faucet and
  per-sink totals for the economy dashboard.
- Each realm audits itself every tick. `World::audit` checks the whole world,
  including money and goods in flight between realms.
- A resident's holdings move with the resident on migration.

### 5.5 Events and notices

```rust
pub struct EventMeta {
    pub kind: &'static str,          // "<module>.<what_happened>", e.g. "crime.theft_committed"
    pub actor: Option<ResidentId>,
    pub pos: Option<Pos>,
    pub audience: Audience,
    pub drama: u8,                   // 0..=100, used by the director
}
pub enum Audience {
    Public,                 // viewers see it
    Nearby(u32),            // viewers see it; residents within the radius get a notice
    Private(Vec<ResidentId>), // only these residents get a notice; viewers do not see it
    Silent,                 // only event handlers see it
}
```

Text written by a resident is never placed in a plain string field of a
notice. Use `protocol::Untrusted { from, text }`. It serializes with
`"untrusted": true`.

### 5.6 World

```rust
impl World {
    pub fn new(plugins: &[&dyn Plugin], config: WorldConfig) -> World;   // runs worldgen
    pub fn apply(&mut self, input: Input) -> Output;     // the only way to change state
    pub fn observe(&self, r: ResidentId, opts: &ObserveOpts) -> Result<Observation, SimError>;
    pub fn view(&self, q: &ViewQuery) -> WorldView;      // for viewers: tiles, sites, residents
    pub fn catalogue(&self) -> Catalogue;                // verbs with JSON schemas, params, items
    pub fn frame(&self) -> u64;
    pub fn state_hash(&self) -> [u8; 32];
    pub fn snapshot(&self) -> Vec<u8>;
    pub fn restore(plugins: &[&dyn Plugin], bytes: &[u8]) -> Result<World, SnapshotError>;
    pub fn audit(&self) -> Result<WorldTotals, AuditError>;
    pub fn realm(&self, id: RealmId) -> Option<&Realm>;  // tests and admin tools
    pub fn locate(&self, r: ResidentId) -> Option<ZoneId>;
}

pub enum Input {
    Spawn(SpawnSpec),                                 // -> Output::Spawned(ResidentId)
    Act { resident: ResidentId, request: ActionRequest },   // -> Output::Ack(ActionAck)
    Step { skip: Vec<ZoneId> },                       // one frame -> Output::Frame(FrameOutput)
    Admin(AdminCommand),                              // -> Output::Admin(AdminReply)
}
```

Convenience wrappers `spawn`, `act` and `step` call `apply`.

A `World` can host a subset of the realms. Messages for realms that it does
not host go to an external outbox, and `deliver_external` injects messages
from other processes. One process hosts all realms by default.

### 5.7 Testing support

`unsup_kernel::testing` gives `TestWorld`: a small world (one city, a few
zones) built from a list of plugins, with helpers to spawn a resident, act,
step N frames, read attributes and the ledger, and assert conservation and
determinism. Every module uses it for its tests.

## 6. Runtime and `SimHandle`

```rust
#[derive(Clone)]
pub struct SimHandle { /* channels */ }
impl SimHandle {
    pub async fn spawn(&self, spec: SpawnSpec) -> Result<ResidentId, SimError>;
    pub async fn act(&self, r: ResidentId, req: ActionRequest) -> Result<ActionAck, SimError>;
    pub async fn observe(&self, r: ResidentId, opts: ObserveOpts) -> Result<Observation, SimError>;
    pub async fn wait(&self, r: ResidentId, filter: WaitFilter, timeout: Duration) -> Result<Observation, SimError>;
    pub async fn view(&self, q: ViewQuery) -> Result<WorldView, SimError>;
    pub async fn admin(&self, cmd: AdminCommand) -> Result<AdminReply, SimError>;
    pub async fn stats(&self) -> Result<SimStats, SimError>;
    pub fn frames(&self) -> tokio::sync::broadcast::Receiver<Arc<FrameOutput>>;
    pub fn catalogue(&self) -> Arc<Catalogue>;
}
pub fn start(world: World, config: RuntimeConfig) -> (SimHandle, RuntimeJoin);
```

The runtime:

- collects inputs in small batches and applies them through `World::apply`;
- steps one frame per `tick_interval` (default 1 second; 0 means manual
  stepping for tests);
- writes every input to the append-only input log before it applies it;
- writes a snapshot every `snapshot_every` frames;
- decides time dilation from measured zone tick time, and records the
  decision in the log as the `skip` list of `Input::Step`;
- wakes `wait` calls when a resident gets a notice or finishes an action.

## 7. Data files

Every balance number lives in a TOML file, not in code.

- Each module has `data/balance.toml` (parameters) and optionally
  `data/items.toml` in its crate. The module embeds them with `include_str!`
  and passes them to `Builder::load_data`.
- The server can override parameters at start with `--data-dir`.

```toml
[params."life.food_decay_per_hour"]
value = 14
min = 0
max = 200
policy = false            # true: a law can change it
summary = "Food points that a resident loses each game hour."

[[items]]
name = "bread"
weight = 1
tags = ["food"]
attrs = { nutrition = 300 }
summary = "A loaf of bread."
```

## 8. How to add a feature

1. Decide which module owns it. Make a new module crate if none fits.
2. Express state with components, resources, attributes and conditions.
3. Add verbs. Each verb has an `Args` struct with doc comments; the doc
   comments become the schema that agents read.
4. Put every number in the data file.
5. Move money and goods only through the ledger.
6. Emit events for things that viewers, missions or leaderboards care about.
7. Add an observer if the resident must see the new state.
8. Test through `TestWorld`: the happy path, each refusal, conservation and
   determinism.
9. Register the plugin in `unsup-server`.
