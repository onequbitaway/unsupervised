//! JSON schema export for the wire types.

use schemars::{JsonSchema, SchemaGenerator};
use serde_json::{Map, Value, json};

use crate::{
    ActionAck, ActionError, ActionReport, ActionRequest, ActionStatus, AdminCommand, AdminReply,
    AttrInfo, Catalogue, CityId, CityView, Clock, CondInfo, Condition, Detail, ErrorCode, Event,
    FrameOutput, ItemId, ItemInfo, Money, Notice, Observation, ObserveOpts, OrgId, OwnerId,
    ParamInfo, Pos, RealmId, Rect, ResidentId, ResidentView, Routine, SimError, SiteId, SiteView,
    SpawnSpec, Tick, Untrusted, VerbInfo, VerbScope, ViewQuery, WaitFilter, WorldView, ZoneDelta,
    ZoneId, ZoneView,
};

/// Adds the schema of `T` to the generator and returns the name of `T`.
fn add<T: JsonSchema>(generator: &mut SchemaGenerator, names: &mut Vec<String>) {
    // The returned `$ref` is not needed: the definition stays in the generator.
    let _ = generator.subschema_for::<T>();
    names.push(T::schema_name().into_owned());
}

/// Returns one JSON Schema document (draft 2020-12) that defines every wire
/// type under `$defs`. The `x-wire-types` list names the top-level types in
/// the order of the protocol document.
#[must_use]
pub fn wire_schema() -> Value {
    let mut generator = SchemaGenerator::default();
    let mut names = Vec::new();
    macro_rules! add_all {
        ($($ty:ty),* $(,)?) => { $( add::<$ty>(&mut generator, &mut names); )* };
    }
    add_all!(
        ResidentId,
        OwnerId,
        OrgId,
        SiteId,
        ZoneId,
        CityId,
        ItemId,
        Tick,
        Money,
        Pos,
        Rect,
        RealmId,
        Clock,
        ActionRequest,
        ActionAck,
        ActionStatus,
        ActionReport,
        ActionError,
        ErrorCode,
        Routine,
        Condition,
        ObserveOpts,
        Detail,
        Observation,
        Notice,
        Untrusted,
        Event,
        FrameOutput,
        ZoneDelta,
        ResidentView,
        ViewQuery,
        WorldView,
        CityView,
        ZoneView,
        SiteView,
        Catalogue,
        VerbInfo,
        VerbScope,
        ParamInfo,
        ItemInfo,
        AttrInfo,
        CondInfo,
        SpawnSpec,
        AdminCommand,
        AdminReply,
        WaitFilter,
        SimError,
    );
    let definitions: Map<String, Value> = generator.take_definitions(true);
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Unsupervised wire types",
        "x-wire-types": names,
        "$defs": definitions,
    })
}

/// Returns the JSON schema of one type as a self-contained document.
#[must_use]
pub fn schema_of<T: JsonSchema>() -> Value {
    SchemaGenerator::default().into_root_schema_for::<T>().to_value()
}
