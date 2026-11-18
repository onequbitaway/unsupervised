//! Wire types of Unsupervised.
//!
//! Everything that crosses a process boundary is defined here: ids, actions,
//! observations, events, frames, the catalogue and the control messages. The
//! types are the single source of truth for `docs/design/protocol.md`; the
//! JSON schema in `schema/` and the SDK types are generated from them with
//! [`schema::wire_schema`].
//!
//! Conventions:
//!
//! - JSON field names are `snake_case`.
//! - Ids are newtypes that serialize as plain numbers.
//! - Enums that carry data use an internal tag named `state`, `type` or
//!   `kind`. Enums without data serialize as a `snake_case` string.
//! - Optional fields are left out when they are empty, and may be left out by
//!   the sender.
//! - Two codecs carry the same types: JSON and MessagePack. See [`codec`].
//!
//! Text written by a resident never travels in a plain string field. It is
//! always wrapped in [`Untrusted`], which serializes with `"untrusted": true`.

mod action;
mod catalogue;
pub mod codec;
mod control;
mod event;
mod ids;
mod observe;
mod routine;
pub mod schema;

pub use action::{ActionAck, ActionError, ActionReport, ActionRequest, ActionStatus, ErrorCode};
pub use catalogue::{AttrInfo, Catalogue, CondInfo, ItemInfo, ParamInfo, VerbInfo, VerbScope};
pub use control::{AdminCommand, AdminReply, SimError, SpawnSpec, WaitFilter};
pub use event::{
    CityView, Event, FrameOutput, ResidentView, SiteView, ViewQuery, WorldView, ZoneDelta, ZoneView,
};
pub use ids::{
    CityId, Clock, ItemId, Money, OrgId, OwnerId, Pos, RealmId, Rect, ResidentId, SiteId, Tick,
    ZoneId,
};
pub use observe::{AlwaysTrue, Detail, Notice, Observation, ObserveOpts, Untrusted};
pub use routine::{Condition, Routine};

/// The JSON value type used for open payloads (action arguments, results,
/// notice data, observation sections).
pub use serde_json::Value;
