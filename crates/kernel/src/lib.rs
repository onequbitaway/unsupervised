//! The deterministic simulation kernel of Unsupervised.
//!
//! The kernel is a library. It has no threads of its own, no clock and no
//! input or output. It owns residents, time, actions, money, goods, events
//! and messages. All game rules live in modules: plugins that add verbs,
//! systems, data and observers on top of the kernel.
//!
//! The contract is `docs/design/architecture.md`. This crate implements
//! sections 3 to 5 and 7.
//!
//! # Map of the crate
//!
//! - [`World`] holds all realms and is the only door for inputs
//!   ([`World::apply`]). It also reads ([`World::observe`], [`World::view`]),
//!   hashes, snapshots and audits.
//! - [`Realm`] is one single-threaded actor: a zone or the global realm.
//!   Module code reaches all state through it.
//! - [`Plugin`], [`Builder`] and [`Blueprint`] are how a module registers
//!   what it adds.
//! - [`Ledger`] is the only store of money and goods.
//! - [`testing::TestWorld`] is a small world for module tests.
//!
//! # A module in a few lines
//!
//! ```
//! use serde::{Deserialize, Serialize};
//! use unsup_kernel::schemars::JsonSchema;
//! use unsup_kernel::testing::TestWorld;
//! use unsup_kernel::{AttrDef, Begin, Builder, Plugin, Scope, Stage, VerbSpec};
//!
//! /// Arguments of `rest`.
//! #[derive(Serialize, Deserialize, JsonSchema)]
//! # #[schemars(crate = "unsup_kernel::schemars")]
//! struct RestArgs {
//!     /// How many ticks to rest.
//!     ticks: u32,
//! }
//!
//! struct Life;
//!
//! impl Plugin for Life {
//!     fn name(&self) -> &'static str {
//!         "life"
//!     }
//!
//!     fn build(&self, b: &mut Builder) {
//!         let energy = b.attr(AttrDef::new("energy").range(0, 100).default(50));
//!         if b.scope() == Scope::Zone {
//!             // Every resident loses one point of energy per tick.
//!             b.system(Stage::Update, 0, "life.tire", move |realm| {
//!                 for value in realm.attrs_mut(energy).iter_mut() {
//!                     *value = (*value - 1).max(0);
//!                 }
//!             });
//!         }
//!         b.verb::<RestArgs>(
//!             VerbSpec {
//!                 verb: "rest",
//!                 module: "life",
//!                 summary: "Rest to get energy back.",
//!                 free: false,
//!                 allowed_in: &[],
//!             },
//!             |_realm, _ctx, args| Ok(Begin::Busy { ticks: args.ticks }),
//!             move |realm, ctx, args| {
//!                 let now = realm.attr_add(ctx.idx, energy, 3 * i64::from(args.ticks));
//!                 Ok(serde_json::json!({ "energy": now }))
//!             },
//!         );
//!     }
//! }
//!
//! let mut world = TestWorld::new(&[&Life]);
//! let ada = world.spawn("Ada");
//! world.act_ok(ada, "rest", serde_json::json!({ "ticks": 10 }));
//! world.step(10);
//! // Ten ticks of tiring, then thirty points back.
//! assert_eq!(world.attr(ada, "energy"), 50 - 10 + 30);
//! ```

mod agenda;
mod codec;
mod column;
mod config;
mod defs;
mod directory;
mod event;
mod grid;
mod ids;
mod kernel_plugin;
mod kin;
mod layout;
mod ledger;
mod message;
mod plugin;
mod realm;
mod rng;
mod sites;
mod spawn;
pub mod testing;
mod tiles;
mod timers;
mod world;

pub use agenda::{ActivityView, REPORT_KIND};
pub use codec::DecodeError;
pub use column::{Column, Component, Resource};
pub use config::{CityConfig, ConfigError, MAX_ZONES, TICKS_PER_DAY, WorldConfig};
pub use defs::{AttrDef, AttrShow, CondDef, DataError, ItemDef, ParamDef};
pub use directory::{DirEntry, Directory};
pub use event::{Audience, EventMeta};
pub use ids::{AttrId, CondId, FaucetId, Idx, ParamId, SinkId, VerbId};
pub use kernel_plugin::{GRANT_FAUCET, SetRoutinesArgs, WaitArgs};
pub use kin::Kin;
pub use layout::{CityInfo, Layout, ZoneInfo};
pub use ledger::{
    AuditError, Flow, Holder, ItemTotals, Ledger, LedgerError, LedgerTotals, MoneyTotals, UNCLAIMED,
};
pub use message::{Dest, Envelope};
pub use plugin::{
    ActionCtx, Args, Begin, Blueprint, Builder, GlobalCtx, Message, NameTables, NoArgs, Plugin,
    Scope, Stage, VerbSpec, WorldGen,
};
pub use realm::{Identity, Membership, Outsider, PosError, RESERVED_IDS, Realm};
pub use rng::Rng;
pub use sites::{Site, Sites};
pub use tiles::TileMap;
pub use world::{
    Input, MemoryEstimate, Output, SnapshotError, TickClock, World, WorldError, WorldItem,
    WorldMoney, WorldTotals, ZoneTiming,
};

/// The wire types. Modules use them through the kernel so that every crate
/// has the same version.
pub use unsup_protocol as protocol;
pub use unsup_protocol::{
    ActionError, ActionRequest, CityId, Clock, ErrorCode, Event, ItemId, Money, ObserveOpts, OrgId,
    OwnerId, Pos, RealmId, Rect, ResidentId, SiteId, Tick, Untrusted, ZoneId,
};

/// Re-export of `schemars`, for `#[derive(JsonSchema)]` on verb arguments.
pub use schemars;
/// Re-export of `serde_json`, for values in results, notices and events.
pub use serde_json;
pub use serde_json::Value;
