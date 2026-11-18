//! A profile matched with the catalogue of a world.

use std::collections::BTreeSet;

use serde_json::Value;
use unsup_kernel::Rng;
use unsup_protocol::{Catalogue, ItemInfo};

use super::{ArgSpec, Profile, ProfileError};
use crate::defaults::defaults;
use crate::scene::Scene;
use crate::think::TickThink;

/// One action that a resident can take: a verb of the world with its
/// weight and its argument specification.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    verb: String,
    weight: u32,
    args: ArgSpec,
}

/// The action that a resident chose.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Name of the verb.
    pub verb: String,
    /// The arguments. `Value::Null` when the action has none.
    pub args: Value,
}

/// A profile that is ready to use: only verbs that the world has, with the
/// item list of the world for the generators.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// Name of the profile.
    pub name: String,
    /// Think time in ticks.
    pub think: TickThink,
    /// Chance that a resident observes after an action.
    pub observe_share: f64,
    /// Chance that an agent waits for the end of an action.
    pub wait_share: f64,
    /// Verbs of the profile that the world does not have, in profile order.
    pub skipped: Vec<String>,
    /// Messages for the operator: skipped verbs and arguments that do not
    /// match the catalogue.
    pub warnings: Vec<String>,
    entries: Vec<Entry>,
    total_weight: u64,
    items: Vec<ItemInfo>,
    attempts: u32,
}

/// Compares the argument names of a profile action with the schema of the
/// verb, and returns a warning for each difference.
fn schema_warnings(profile: &str, verb: &str, args: &ArgSpec, schema: &Value) -> Vec<String> {
    let Some(given) = args.keys() else {
        // A generator makes the whole object. Its keys are not known here.
        return Vec::new();
    };
    let given: BTreeSet<&str> = given.into_iter().collect();
    let names = |key: &str| -> BTreeSet<&str> {
        match schema.get(key) {
            Some(Value::Object(map)) => map.keys().map(String::as_str).collect(),
            Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
            _ => BTreeSet::new(),
        }
    };
    let (known, required) = (names("properties"), names("required"));
    let mut warnings = Vec::new();
    for missing in required.difference(&given) {
        warnings.push(format!(
            "profile `{profile}`: the verb `{verb}` needs the argument `{missing}`, which the profile does not give"
        ));
    }
    // A schema without `properties` says nothing about the names.
    if schema.get("properties").is_some() {
        for extra in given.difference(&known) {
            warnings
                .push(format!("profile `{profile}`: the verb `{verb}` has no argument `{extra}`"));
        }
    }
    warnings
}

impl Profile {
    /// Matches the profile with the catalogue of a world.
    ///
    /// A verb that the catalogue does not have is skipped and named in
    /// [`Resolved::skipped`] and [`Resolved::warnings`]. The weights of the
    /// other actions keep their ratio.
    ///
    /// # Errors
    ///
    /// Fails when the world has none of the verbs that have a weight.
    pub fn resolve(&self, catalogue: &Catalogue) -> Result<Resolved, ProfileError> {
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        let mut warnings = Vec::new();
        for action in &self.actions {
            let Some(info) = catalogue.verbs.iter().find(|info| info.verb == action.verb) else {
                warnings.push(format!(
                    "profile `{}`: the world has no verb `{}`; the action is skipped",
                    self.name, action.verb
                ));
                skipped.push(action.verb.clone());
                continue;
            };
            warnings.extend(schema_warnings(
                &self.name,
                &action.verb,
                &action.args,
                &info.args_schema,
            ));
            if action.weight > 0 {
                entries.push(Entry {
                    verb: action.verb.clone(),
                    weight: action.weight,
                    args: action.args.clone(),
                });
            }
        }
        let total_weight: u64 = entries.iter().map(|entry| u64::from(entry.weight)).sum();
        if total_weight == 0 {
            return Err(ProfileError {
                message: format!(
                    "the world has none of the verbs of profile `{}` (skipped: {})",
                    self.name,
                    skipped.join(", ")
                ),
            });
        }
        Ok(Resolved {
            name: self.name.clone(),
            think: self.think,
            observe_share: self.observe_share,
            wait_share: self.wait_share,
            skipped,
            warnings,
            entries,
            total_weight,
            items: catalogue.items.clone(),
            attempts: defaults().driver.pick_attempts,
        })
    }
}

impl Resolved {
    /// Returns the verbs that residents can choose, in profile order, each
    /// with its weight.
    #[must_use]
    pub fn verbs(&self) -> Vec<(&str, u32)> {
        self.entries.iter().map(|entry| (entry.verb.as_str(), entry.weight)).collect()
    }

    /// Chooses one entry by weight.
    fn weighted(&self, rng: &mut Rng) -> &Entry {
        let mut ticket = rng.below(self.total_weight);
        for entry in &self.entries {
            let weight = u64::from(entry.weight);
            if ticket < weight {
                return entry;
            }
            ticket -= weight;
        }
        // The ticket is below the sum of the weights, so the loop returns.
        // This line only satisfies the compiler; the list is never empty.
        &self.entries[self.entries.len() - 1]
    }

    /// Chooses the next action of a resident: a verb by weight, then its
    /// arguments. When the arguments cannot be made now (for example no
    /// resident is near), it chooses again, a few times. Returns `None`
    /// when no try gave an action.
    pub fn pick(&self, rng: &mut Rng, scene: &mut dyn Scene) -> Option<Choice> {
        for _ in 0..self.attempts.max(1) {
            let entry = self.weighted(rng);
            if let Some(args) = entry.args.generate(rng, scene, &self.items) {
                return Some(Choice { verb: entry.verb.clone(), args });
            }
        }
        None
    }
}
