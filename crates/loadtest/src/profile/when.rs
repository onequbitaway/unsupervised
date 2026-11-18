//! Conditions of an action: what must be true before a resident chooses it.
//!
//! In a profile, `when` is a table. Every line of the table must hold.
//!
//! ```toml
//! when = { cash_min = 750, attr_below = { "need.food" = 900 } }
//! when = { carries_tag = "food" }
//! when = { did = "open_account" }
//! ```

use std::collections::BTreeMap;

use serde::Deserialize;
use unsup_protocol::{Catalogue, ItemId};

use crate::scene::Scene;

/// The conditions of one action, as the profile wrote them. An empty table
/// always holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct When {
    /// The resident has at least this much cash, in cents.
    #[serde(default)]
    pub cash_min: Option<i64>,
    /// The resident has at most this much cash, in cents.
    #[serde(default)]
    pub cash_max: Option<i64>,
    /// The resident carries an item with this tag, for example `food`.
    #[serde(default)]
    pub carries_tag: Option<String>,
    /// The resident carries no item with this tag.
    #[serde(default)]
    pub lacks_tag: Option<String>,
    /// Each named attribute of the resident is below the value.
    #[serde(default)]
    pub attr_below: BTreeMap<String, i64>,
    /// Each named attribute of the resident is above the value.
    #[serde(default)]
    pub attr_above: BTreeMap<String, i64>,
    /// The world accepted this verb from the resident before, in this run.
    #[serde(default)]
    pub did: Option<String>,
}

/// Conditions that are matched with the catalogue of a world: each tag is
/// the list of its items.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Checked {
    cash_min: Option<i64>,
    cash_max: Option<i64>,
    carries: Option<Vec<ItemId>>,
    lacks: Option<Vec<ItemId>>,
    attr_below: Vec<(String, i64)>,
    attr_above: Vec<(String, i64)>,
}

fn items_with(catalogue: &Catalogue, tag: &str) -> Vec<ItemId> {
    catalogue
        .items
        .iter()
        .filter(|item| item.tags.iter().any(|t| t == tag))
        .map(|item| item.id)
        .collect()
}

impl When {
    /// Matches the conditions with a catalogue. Returns one message for
    /// each name that the world does not have.
    pub(super) fn check(&self, catalogue: &Catalogue) -> Result<Checked, Vec<String>> {
        let mut problems = Vec::new();
        let mut tag = |name: &Option<String>| {
            let name = name.as_ref()?;
            let items = items_with(catalogue, name);
            if items.is_empty() {
                problems.push(format!("no item of the world has the tag `{name}`"));
            }
            Some(items)
        };
        let carries = tag(&self.carries_tag);
        let lacks = tag(&self.lacks_tag);
        for name in self.attr_below.keys().chain(self.attr_above.keys()) {
            if !catalogue.attrs.iter().any(|attr| &attr.name == name) {
                problems.push(format!("the world has no attribute `{name}`"));
            }
        }
        if !problems.is_empty() {
            return Err(problems);
        }
        let pairs = |map: &BTreeMap<String, i64>| -> Vec<(String, i64)> {
            map.iter().map(|(name, value)| (name.clone(), *value)).collect()
        };
        Ok(Checked {
            cash_min: self.cash_min,
            cash_max: self.cash_max,
            carries,
            lacks,
            attr_below: pairs(&self.attr_below),
            attr_above: pairs(&self.attr_above),
        })
    }
}

impl Checked {
    /// Returns true when every condition holds for the resident of the
    /// scene. A fact that the scene does not know does not hold.
    pub(super) fn holds(&self, scene: &mut dyn Scene) -> bool {
        if self.cash_min.is_some() || self.cash_max.is_some() {
            let Some(cash) = scene.cash() else { return false };
            if self.cash_min.is_some_and(|min| cash < min)
                || self.cash_max.is_some_and(|max| cash > max)
            {
                return false;
            }
        }
        for (name, value) in &self.attr_below {
            if !scene.attr(name).is_some_and(|attr| attr < *value) {
                return false;
            }
        }
        for (name, value) in &self.attr_above {
            if !scene.attr(name).is_some_and(|attr| attr > *value) {
                return false;
            }
        }
        if self.carries.is_some() || self.lacks.is_some() {
            let carried = scene.carried();
            let has = |items: &[ItemId]| carried.iter().any(|c| items.contains(&c.item));
            if self.carries.as_ref().is_some_and(|items| !has(items))
                || self.lacks.as_ref().is_some_and(|items| has(items))
            {
                return false;
            }
        }
        true
    }
}
