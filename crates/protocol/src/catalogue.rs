//! The public catalogue: verbs, parameters, items, attributes, conditions.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ids::ItemId;

/// Everything that an agent can look up before it acts.
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
pub struct Catalogue {
    /// Every verb, ordered by module and name.
    pub verbs: Vec<VerbInfo>,
    /// Every parameter, ordered by name.
    pub params: Vec<ParamInfo>,
    /// Every item, ordered by id.
    pub items: Vec<ItemInfo>,
    /// Every visible attribute, ordered by name.
    pub attrs: Vec<AttrInfo>,
    /// Every condition, ordered by name.
    pub conditions: Vec<CondInfo>,
}

/// Where a verb runs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VerbScope {
    /// The verb runs in the resident's zone.
    Zone,
    /// The verb runs in the global realm. Its answer comes in a report.
    Global,
}

/// One verb of the catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VerbInfo {
    /// Name of the verb.
    pub verb: String,
    /// The module that owns the verb.
    pub module: String,
    /// One sentence for agents.
    pub summary: String,
    /// True when the verb does not occupy the resident.
    pub free: bool,
    /// Where the verb runs.
    pub scope: VerbScope,
    /// JSON schema of the arguments.
    pub args_schema: Value,
}

/// One tunable number of the world.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ParamInfo {
    /// Name: `<module>.<what>`.
    pub name: String,
    /// The national value now.
    pub value: i64,
    /// Least allowed value, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Greatest allowed value, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// True when a law can change the parameter.
    pub policy: bool,
    /// One sentence that says what the number means.
    pub summary: String,
}

/// One entry of the item catalogue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ItemInfo {
    /// Id of the item.
    pub id: ItemId,
    /// Name of the item.
    pub name: String,
    /// Weight of one unit.
    pub weight: i64,
    /// Tags, for example `food`.
    pub tags: Vec<String>,
    /// Named numbers, for example `nutrition`.
    pub attrs: BTreeMap<String, i64>,
    /// One sentence that describes the item.
    pub summary: String,
}

/// One attribute of residents.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttrInfo {
    /// Name of the attribute.
    pub name: String,
    /// Least value, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Greatest value, if there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// Value of a new resident.
    pub default: i64,
    /// One sentence that says what the attribute means.
    pub summary: String,
}

/// One condition that a resident can have.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CondInfo {
    /// Name of the condition.
    pub name: String,
    /// True when the condition forbids most verbs.
    pub blocking: bool,
    /// One sentence that says what the condition means.
    pub summary: String,
}
