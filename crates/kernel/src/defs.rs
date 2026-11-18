//! Definitions: parameters, items, attributes, conditions, and the data
//! files that declare them.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

/// A named tunable number. Defaults come from data files; the global realm
/// can change the value at run time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParamDef {
    /// Name: `<module>.<what>`.
    pub name: String,
    /// Default national value.
    pub value: i64,
    /// Least allowed value.
    pub min: i64,
    /// Greatest allowed value.
    pub max: i64,
    /// True when a law can change the parameter.
    pub policy: bool,
    /// One sentence that says what the number means.
    pub summary: String,
}

impl ParamDef {
    /// Makes a parameter without bounds.
    #[must_use]
    pub fn new(name: impl Into<String>, value: i64) -> ParamDef {
        ParamDef {
            name: name.into(),
            value,
            min: i64::MIN,
            max: i64::MAX,
            policy: false,
            summary: String::new(),
        }
    }

    /// Sets the allowed range.
    #[must_use]
    pub fn range(mut self, min: i64, max: i64) -> ParamDef {
        self.min = min;
        self.max = max;
        self
    }

    /// Says whether a law can change the parameter.
    #[must_use]
    pub fn policy(mut self, policy: bool) -> ParamDef {
        self.policy = policy;
        self
    }

    /// Sets the summary.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> ParamDef {
        self.summary = summary.into();
        self
    }
}

/// An entry of the item catalogue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemDef {
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

impl ItemDef {
    /// Makes an item with weight 0, no tags and no attributes.
    #[must_use]
    pub fn new(name: impl Into<String>) -> ItemDef {
        ItemDef {
            name: name.into(),
            weight: 0,
            tags: Vec::new(),
            attrs: BTreeMap::new(),
            summary: String::new(),
        }
    }

    /// Sets the weight of one unit.
    #[must_use]
    pub fn weight(mut self, weight: i64) -> ItemDef {
        self.weight = weight;
        self
    }

    /// Adds a tag.
    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> ItemDef {
        self.tags.push(tag.into());
        self
    }

    /// Sets a named number.
    #[must_use]
    pub fn attr(mut self, name: impl Into<String>, value: i64) -> ItemDef {
        self.attrs.insert(name.into(), value);
        self
    }

    /// Sets the summary.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> ItemDef {
        self.summary = summary.into();
        self
    }

    /// Returns true when the item has the tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Returns a named number, or 0 when the item does not have it.
    #[must_use]
    pub fn attr_value(&self, name: &str) -> i64 {
        self.attrs.get(name).copied().unwrap_or(0)
    }
}

/// When the resident itself sees an attribute in its observation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AttrShow {
    /// In every observation.
    #[default]
    Brief,
    /// Only in an observation with full detail.
    Full,
    /// Never. The attribute is internal and is not in the catalogue.
    Hidden,
}

/// A named `i64` column of the resident table, with bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttrDef {
    /// Name of the attribute.
    pub name: String,
    /// Least value.
    pub min: i64,
    /// Greatest value.
    pub max: i64,
    /// Value of a new resident.
    pub default: i64,
    /// When the resident sees the attribute.
    pub show: AttrShow,
    /// One sentence that says what the attribute means.
    pub summary: String,
}

impl AttrDef {
    /// Makes an attribute without bounds, with default 0, shown always.
    #[must_use]
    pub fn new(name: impl Into<String>) -> AttrDef {
        AttrDef {
            name: name.into(),
            min: i64::MIN,
            max: i64::MAX,
            default: 0,
            show: AttrShow::Brief,
            summary: String::new(),
        }
    }

    /// Sets the bounds.
    #[must_use]
    pub fn range(mut self, min: i64, max: i64) -> AttrDef {
        self.min = min;
        self.max = max;
        self
    }

    /// Sets the value of a new resident.
    #[must_use]
    pub fn default(mut self, value: i64) -> AttrDef {
        self.default = value;
        self
    }

    /// Says when the resident sees the attribute.
    #[must_use]
    pub fn show(mut self, show: AttrShow) -> AttrDef {
        self.show = show;
        self
    }

    /// Sets the summary.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> AttrDef {
        self.summary = summary.into();
        self
    }
}

/// A named flag that a resident can have, with an optional end tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CondDef {
    /// Name of the condition.
    pub name: String,
    /// True when the condition rejects every verb that does not list it in
    /// `VerbSpec::allowed_in`.
    pub blocking: bool,
    /// One sentence that says what the condition means.
    pub summary: String,
}

impl CondDef {
    /// Makes a condition that does not block verbs.
    #[must_use]
    pub fn new(name: impl Into<String>) -> CondDef {
        CondDef { name: name.into(), blocking: false, summary: String::new() }
    }

    /// Makes the condition blocking.
    #[must_use]
    pub fn blocking(mut self) -> CondDef {
        self.blocking = true;
        self
    }

    /// Sets the summary.
    #[must_use]
    pub fn summary(mut self, summary: impl Into<String>) -> CondDef {
        self.summary = summary.into();
        self
    }
}

/// A list of definitions with stable numeric ids and a name index.
///
/// Ids are positions in the list. `ensure` returns the existing id when the
/// name is known. A registry can start with *reserved* names (from a
/// snapshot), so that ids stay the same when the set of plugins changes.
#[derive(Clone, Debug)]
pub(crate) struct Registry<T> {
    kind: &'static str,
    entries: Vec<Entry<T>>,
    index: BTreeMap<String, u16>,
}

#[derive(Clone, Debug)]
struct Entry<T> {
    name: String,
    /// `None` while the name is only reserved.
    def: Option<T>,
}

impl<T> Registry<T> {
    pub(crate) fn new(kind: &'static str) -> Registry<T> {
        Registry { kind, entries: Vec::new(), index: BTreeMap::new() }
    }

    /// Reserves names at the ids `0..names.len()`. Call it before any
    /// `ensure`.
    pub(crate) fn reserve(&mut self, names: &[String]) {
        debug_assert!(self.entries.is_empty(), "reserve must come first");
        for name in names {
            let id = self.entries.len() as u16;
            self.index.insert(name.clone(), id);
            self.entries.push(Entry { name: name.clone(), def: None });
        }
    }

    /// Returns the id of `name`. The first definition of a name wins; a later
    /// one is ignored.
    pub(crate) fn ensure(&mut self, name: &str, def: T) -> u16 {
        if let Some(&id) = self.index.get(name) {
            let entry = &mut self.entries[usize::from(id)];
            if entry.def.is_none() {
                entry.def = Some(def);
            }
            return id;
        }
        assert!(!name.is_empty(), "a {} needs a name", self.kind);
        assert!(
            self.entries.len() < usize::from(u16::MAX),
            "too many definitions of kind {}",
            self.kind
        );
        let id = self.entries.len() as u16;
        self.index.insert(name.to_string(), id);
        self.entries.push(Entry { name: name.to_string(), def: Some(def) });
        id
    }

    /// Replaces the definition of a known name.
    pub(crate) fn replace(&mut self, id: u16, def: T) {
        self.entries[usize::from(id)].def = Some(def);
    }

    pub(crate) fn id(&self, name: &str) -> Option<u16> {
        self.index.get(name).copied()
    }

    /// Returns the definition, or `None` when the id is unknown or the name
    /// is only reserved.
    pub(crate) fn get(&self, id: u16) -> Option<&T> {
        self.entries.get(usize::from(id)).and_then(|e| e.def.as_ref())
    }

    pub(crate) fn get_mut(&mut self, id: u16) -> Option<&mut T> {
        self.entries.get_mut(usize::from(id)).and_then(|e| e.def.as_mut())
    }

    pub(crate) fn name(&self, id: u16) -> Option<&str> {
        self.entries.get(usize::from(id)).map(|e| e.name.as_str())
    }

    /// Number of ids, including reserved names.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// Every name in id order, including reserved names.
    pub(crate) fn names(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.name.clone()).collect()
    }

    /// Every defined entry in id order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u16, &T)> {
        self.entries.iter().enumerate().filter_map(|(i, e)| e.def.as_ref().map(|d| (i as u16, d)))
    }

    /// Gives every reserved name that no plugin defined a definition made by
    /// `retired`. The name keeps its id, so old state stays readable.
    pub(crate) fn fill_reserved(&mut self, retired: impl Fn(&str) -> T) {
        for entry in &mut self.entries {
            if entry.def.is_none() {
                entry.def = Some(retired(&entry.name));
            }
        }
    }
}

/// A data file is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataError {
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid data file: {}", self.message)
    }
}

impl std::error::Error for DataError {}

/// The parsed form of a data file. Every field of an entry is optional so
/// that an override file can change one number.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DataFile {
    #[serde(default)]
    pub(crate) params: BTreeMap<String, ParamData>,
    #[serde(default)]
    pub(crate) items: Vec<ItemData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ParamData {
    pub(crate) value: Option<i64>,
    pub(crate) min: Option<i64>,
    pub(crate) max: Option<i64>,
    pub(crate) policy: Option<bool>,
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ItemData {
    pub(crate) name: String,
    pub(crate) weight: Option<i64>,
    pub(crate) tags: Option<Vec<String>>,
    pub(crate) attrs: Option<BTreeMap<String, i64>>,
    pub(crate) summary: Option<String>,
}

impl DataFile {
    pub(crate) fn parse(text: &str) -> Result<DataFile, DataError> {
        toml::from_str(text).map_err(|e| DataError { message: e.to_string() })
    }
}

impl ParamData {
    /// Makes the definition of a new parameter.
    pub(crate) fn into_def(self, name: &str) -> Result<ParamDef, DataError> {
        let Some(value) = self.value else {
            return Err(DataError { message: format!("parameter `{name}` has no `value`") });
        };
        let def = ParamDef {
            name: name.to_string(),
            value,
            min: self.min.unwrap_or(i64::MIN),
            max: self.max.unwrap_or(i64::MAX),
            policy: self.policy.unwrap_or(false),
            summary: self.summary.unwrap_or_default(),
        };
        check_param(&def)?;
        Ok(def)
    }

    /// Changes the given fields of an existing definition.
    pub(crate) fn apply_to(self, def: &ParamDef) -> Result<ParamDef, DataError> {
        let def = ParamDef {
            name: def.name.clone(),
            value: self.value.unwrap_or(def.value),
            min: self.min.unwrap_or(def.min),
            max: self.max.unwrap_or(def.max),
            policy: self.policy.unwrap_or(def.policy),
            summary: self.summary.unwrap_or_else(|| def.summary.clone()),
        };
        check_param(&def)?;
        Ok(def)
    }
}

/// Checks that the default of a parameter is inside its range.
pub(crate) fn check_param(def: &ParamDef) -> Result<(), DataError> {
    if def.min > def.max {
        return Err(DataError {
            message: format!("parameter `{}` has min {} above max {}", def.name, def.min, def.max),
        });
    }
    if def.value < def.min || def.value > def.max {
        return Err(DataError {
            message: format!(
                "parameter `{}` has value {} outside {}..={}",
                def.name, def.value, def.min, def.max
            ),
        });
    }
    Ok(())
}

impl ItemData {
    /// Makes the definition of a new item.
    pub(crate) fn into_def(self) -> Result<ItemDef, DataError> {
        if self.name.trim().is_empty() {
            return Err(DataError { message: "an item has an empty name".into() });
        }
        Ok(ItemDef {
            name: self.name,
            weight: self.weight.unwrap_or(0),
            tags: self.tags.unwrap_or_default(),
            attrs: self.attrs.unwrap_or_default(),
            summary: self.summary.unwrap_or_default(),
        })
    }

    /// Changes the given fields of an existing definition.
    pub(crate) fn apply_to(self, def: &ItemDef) -> ItemDef {
        ItemDef {
            name: def.name.clone(),
            weight: self.weight.unwrap_or(def.weight),
            tags: self.tags.unwrap_or_else(|| def.tags.clone()),
            attrs: self.attrs.unwrap_or_else(|| def.attrs.clone()),
            summary: self.summary.unwrap_or_else(|| def.summary.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
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
"#;

    #[test]
    fn parses_the_documented_format() {
        let mut file = DataFile::parse(SAMPLE).unwrap();
        let param = file.params.remove("life.food_decay_per_hour").unwrap();
        let def = param.into_def("life.food_decay_per_hour").unwrap();
        assert_eq!(
            def,
            ParamDef::new("life.food_decay_per_hour", 14)
                .range(0, 200)
                .summary("Food points that a resident loses each game hour.")
        );
        let item = file.items.pop().unwrap().into_def().unwrap();
        assert_eq!(
            item,
            ItemDef::new("bread")
                .weight(1)
                .tag("food")
                .attr("nutrition", 300)
                .summary("A loaf of bread.")
        );
        assert!(item.has_tag("food") && !item.has_tag("tool"));
        assert_eq!((item.attr_value("nutrition"), item.attr_value("none")), (300, 0));
    }

    #[test]
    fn rejects_bad_files() {
        assert!(DataFile::parse("[params.x]\nvalue = \"high\"").is_err());
        assert!(DataFile::parse("[params.x]\nvalue = 1\ncolour = 2").is_err());
        assert!(DataFile::parse("[other]\na = 1").is_err());
        assert!(DataFile::parse("[[items]]\nweight = 1").is_err());

        let no_value = DataFile::parse("[params.x]\nmin = 1").unwrap().params.remove("x").unwrap();
        assert!(no_value.into_def("x").unwrap_err().message.contains("no `value`"));
        let outside =
            DataFile::parse("[params.x]\nvalue = 9\nmax = 5").unwrap().params.remove("x").unwrap();
        assert!(outside.into_def("x").unwrap_err().message.contains("outside"));
        let inverted = DataFile::parse("[params.x]\nvalue = 3\nmin = 5\nmax = 1")
            .unwrap()
            .params
            .remove("x")
            .unwrap();
        assert!(inverted.into_def("x").unwrap_err().message.contains("above max"));
        let unnamed = DataFile::parse("[[items]]\nname = \" \"").unwrap().items.pop().unwrap();
        assert!(unnamed.into_def().is_err());
    }

    #[test]
    fn an_override_changes_only_the_given_fields() {
        let base = ParamDef::new("x", 14).range(0, 200).policy(true).summary("s");
        let change = DataFile::parse("[params.x]\nvalue = 20").unwrap().params.remove("x").unwrap();
        assert_eq!(change.apply_to(&base).unwrap(), ParamDef { value: 20, ..base.clone() });
        let too_high =
            DataFile::parse("[params.x]\nvalue = 201").unwrap().params.remove("x").unwrap();
        assert!(too_high.apply_to(&base).is_err());

        let bread = ItemDef::new("bread").weight(1).tag("food").attr("nutrition", 300);
        let change = DataFile::parse("[[items]]\nname = \"bread\"\nweight = 2")
            .unwrap()
            .items
            .pop()
            .unwrap();
        assert_eq!(change.apply_to(&bread), ItemDef { weight: 2, ..bread });
    }

    #[test]
    fn registry_keeps_ids_stable() {
        let mut registry: Registry<u32> = Registry::new("thing");
        registry.reserve(&["old".to_string(), "kept".to_string()]);
        assert_eq!(registry.ensure("new", 1), 2);
        assert_eq!(registry.ensure("kept", 2), 1);
        // The first definition wins.
        assert_eq!(registry.ensure("kept", 3), 1);
        assert_eq!(registry.get(1), Some(&2));
        assert_eq!(registry.get(0), None);
        assert_eq!(registry.id("old"), Some(0));
        assert_eq!(registry.id("nope"), None);
        assert_eq!(registry.len(), 3);
        assert_eq!(registry.iter().map(|(i, _)| i).collect::<Vec<_>>(), [1, 2]);
        registry.fill_reserved(|name| name.len() as u32);
        assert_eq!(registry.get(0), Some(&3));
        assert_eq!(registry.names(), ["old", "kept", "new"]);
        assert_eq!(registry.name(2), Some("new"));
        registry.replace(2, 9);
        assert_eq!(registry.get(2), Some(&9));
    }
}
