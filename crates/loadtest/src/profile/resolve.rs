//! A profile matched with the catalogue of a world.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use unsup_kernel::Rng;
use unsup_protocol::{ActionRequest, Catalogue, ItemInfo, SiteId};

use super::when::Checked;
use super::{ActionSpec, ArgSpec, Profile, ProfileError};
use crate::defaults::defaults;
use crate::map::WorldMap;
use crate::scene::Scene;
use crate::think::TickThink;

/// The verb with which a resident walks to the site of an action that has
/// `at` (`docs/design/game-design.md`, section 5).
pub const WALK_VERB: &str = "move_to";

/// The argument of [`WALK_VERB`] that names a site.
pub const WALK_SITE_ARG: &str = "site";

/// Greatest number of actions of one profile. A resident keeps the set of
/// actions that it can take in one machine word.
const MAX_ACTIONS: usize = 128;

/// Greatest number of verbs that a profile can remember for `once` and
/// `did`.
const MAX_MARKS: usize = 64;

/// One action that a resident can take: a verb of the world with its
/// weight, its place, its conditions and its argument specification.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    verb: String,
    weight: u32,
    args: ArgSpec,
    /// Kinds of the sites where the action works. Empty: every place.
    at: Vec<String>,
    when: Checked,
    /// The mark that the resident must not have (`once`).
    not_after: Option<u8>,
    /// The mark that the resident must have (`did`).
    after: Option<u8>,
    /// The mark that the resident gets when the world accepts the verb.
    mark: Option<u8>,
}

/// What a resident remembers about its own actions: one mark for each verb
/// that a `once` or a `did` of the profile names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Memory(u64);

/// The action that a resident chose.
#[derive(Clone, Debug, PartialEq)]
pub struct Choice {
    /// Name of the verb.
    pub verb: String,
    /// The arguments. `Value::Null` when the action has none.
    pub args: Value,
    /// The site to which the resident must walk first. `None` when the
    /// action works where the resident stands.
    pub walk_to: Option<SiteId>,
    mark: Option<u8>,
}

impl Choice {
    /// Returns the request that makes the resident walk to the site of the
    /// action, when the action needs a walk.
    #[must_use]
    pub fn walk(&self) -> Option<ActionRequest> {
        self.walk_to.map(|site| ActionRequest::new(WALK_VERB, json!({ WALK_SITE_ARG: site })))
    }

    /// Takes the request of the action itself out of the choice. A second
    /// call gives a request without a verb.
    pub fn take_request(&mut self) -> ActionRequest {
        ActionRequest {
            id: None,
            verb: std::mem::take(&mut self.verb),
            args: std::mem::take(&mut self.args),
        }
    }

    /// Writes into `memory` that the world accepted the action.
    pub fn remember(&self, memory: &mut Memory) {
        if let Some(mark) = self.mark {
            memory.0 |= 1 << mark;
        }
    }
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
    /// Messages for the operator: skipped verbs, and site kinds that the
    /// world does not have.
    pub warnings: Vec<String>,
    entries: Vec<Entry>,
    items: Vec<ItemInfo>,
    attempts: u32,
    site_choices: usize,
}

/// Returns the names under `key` of a JSON schema: the keys of an object or
/// the texts of a list.
fn schema_names<'a>(schema: &'a Value, key: &str) -> BTreeSet<&'a str> {
    match schema.get(key) {
        Some(Value::Object(map)) => map.keys().map(String::as_str).collect(),
        Some(Value::Array(list)) => list.iter().filter_map(Value::as_str).collect(),
        _ => BTreeSet::new(),
    }
}

/// Compares the argument names of a profile action with the schema of the
/// verb, and returns a message for each difference. A verb whose schema has
/// no `properties` takes no arguments.
fn schema_problems(verb: &str, args: &ArgSpec, schema: &Value) -> Vec<String> {
    let Some(given) = args.keys() else {
        return vec![format!(
            "the arguments of the verb `{verb}` are not a table of named arguments"
        )];
    };
    let given: BTreeSet<&str> = given.into_iter().collect();
    let (known, required) = (schema_names(schema, "properties"), schema_names(schema, "required"));
    let mut problems = Vec::new();
    for extra in given.difference(&known) {
        problems.push(format!("the verb `{verb}` has no argument `{extra}`"));
    }
    for missing in required.difference(&given) {
        problems.push(format!(
            "the verb `{verb}` needs the argument `{missing}`, which the profile does not give"
        ));
    }
    problems
}

/// Gives each verb that a `once` or a `did` names a mark.
fn marks(actions: &[ActionSpec]) -> BTreeMap<&str, u8> {
    let named = actions.iter().flat_map(|action| {
        let once = action.once.then_some(action.verb.as_str());
        once.into_iter().chain(action.when.did.as_deref())
    });
    let names: BTreeSet<&str> = named.collect();
    names.into_iter().zip(0u8..).collect()
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
    /// Fails when the world has none of the verbs that have a weight. Fails
    /// also when an action does not fit a verb that the world has: an
    /// argument that the verb does not have, a needed argument that the
    /// profile does not give, a condition that names an attribute, a tag
    /// or a verb that does not exist, or `at` on a free verb. Such an
    /// action would be refused each time, or would do a thing that the
    /// profile does not say.
    pub fn resolve(&self, catalogue: &Catalogue) -> Result<Resolved, ProfileError> {
        let error = |message: String| ProfileError { message };
        if self.actions.len() > MAX_ACTIONS {
            return Err(error(format!(
                "profile `{}` has {} actions; the limit is {MAX_ACTIONS}",
                self.name,
                self.actions.len()
            )));
        }
        let marks = marks(&self.actions);
        if marks.len() > MAX_MARKS {
            return Err(error(format!(
                "profile `{}` names {} verbs in `once` and `did`; the limit is {MAX_MARKS}",
                self.name,
                marks.len()
            )));
        }
        let verb_of = |name: &str| catalogue.verbs.iter().find(|info| info.verb == name);
        let mut entries = Vec::new();
        let mut skipped = Vec::new();
        let mut warnings = Vec::new();
        let mut problems = Vec::new();
        for action in &self.actions {
            let Some(info) = verb_of(&action.verb) else {
                warnings.push(format!(
                    "profile `{}`: the world has no verb `{}`; the action is skipped",
                    self.name, action.verb
                ));
                skipped.push(action.verb.clone());
                continue;
            };
            problems.extend(schema_problems(&action.verb, &action.args, &info.args_schema));
            if info.free && !action.at.is_empty() {
                // A free verb runs at once, also while the resident walks:
                // it would run before the resident is at the site.
                problems.push(format!(
                    "the verb `{}` is free and does not wait behind a walk; it cannot have `at`",
                    action.verb
                ));
            }
            if let Some(did) = &action.when.did
                && !self.actions.iter().any(|other| &other.verb == did)
            {
                problems.push(format!(
                    "the verb `{}` has the condition `did = \"{did}\"`, and the profile has no action `{did}`",
                    action.verb
                ));
            }
            let when = match action.when.check(catalogue) {
                Ok(when) => when,
                Err(found) => {
                    problems.extend(
                        found.into_iter().map(|p| format!("the verb `{}`: {p}", action.verb)),
                    );
                    continue;
                }
            };
            if action.weight > 0 {
                let mark = marks.get(action.verb.as_str()).copied();
                entries.push(Entry {
                    verb: action.verb.clone(),
                    weight: action.weight,
                    args: action.args.clone(),
                    at: action.at.clone(),
                    when,
                    not_after: if action.once { mark } else { None },
                    after: action.when.did.as_deref().and_then(|did| marks.get(did).copied()),
                    mark,
                });
            }
        }
        if entries.iter().any(|entry| !entry.at.is_empty()) {
            let walks_to_sites = verb_of(WALK_VERB).is_some_and(|info| {
                schema_names(&info.args_schema, "properties").contains(WALK_SITE_ARG)
            });
            if !walks_to_sites {
                problems.push(format!(
                    "an action has `at`, and the world has no verb `{WALK_VERB}` with the argument `{WALK_SITE_ARG}`"
                ));
            }
        }
        if !problems.is_empty() {
            return Err(error(format!(
                "profile `{}` does not fit the verbs of the world: {}",
                self.name,
                problems.join("; ")
            )));
        }
        if entries.is_empty() {
            return Err(error(format!(
                "the world has none of the verbs of profile `{}` (skipped: {})",
                self.name,
                skipped.join(", ")
            )));
        }
        let d = &defaults().driver;
        Ok(Resolved {
            name: self.name.clone(),
            think: self.think,
            observe_share: self.observe_share,
            wait_share: self.wait_share,
            skipped,
            warnings,
            entries,
            items: catalogue.items.clone(),
            attempts: d.pick_attempts,
            site_choices: d.site_choices.max(1),
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

    /// Returns the items of the world.
    #[must_use]
    pub fn items(&self) -> &[ItemInfo] {
        &self.items
    }

    /// Adds a warning for each site kind of an `at` that no city of the
    /// map has. An action with such a place is never possible.
    pub fn check_map(&mut self, map: &WorldMap) {
        let kinds: BTreeSet<&String> = self.entries.iter().flat_map(|entry| &entry.at).collect();
        for kind in kinds {
            if !map.has_kind(kind) {
                self.warnings.push(format!(
                    "profile `{}`: no city of the world has a site of the kind `{kind}`",
                    self.name
                ));
            }
        }
    }

    /// Returns the set of the actions whose conditions hold for the
    /// resident of the scene, one bit for each action.
    fn open(&self, scene: &mut dyn Scene, memory: Memory) -> u128 {
        let has = |mark: u8| memory.0 & (1 << mark) != 0;
        let mut open = 0u128;
        for (at, entry) in self.entries.iter().enumerate() {
            if entry.not_after.is_some_and(has) || entry.after.is_some_and(|mark| !has(mark)) {
                continue;
            }
            if entry.when.holds(scene) {
                open |= 1 << at;
            }
        }
        open
    }

    /// Chooses one action of the set by weight.
    fn weighted(&self, rng: &mut Rng, open: u128) -> Option<usize> {
        let is_open = |at: usize| open & (1 << at) != 0;
        let weights = || self.entries.iter().enumerate().filter(|(at, _)| is_open(*at));
        let total: u64 = weights().map(|(_, entry)| u64::from(entry.weight)).sum();
        if total == 0 {
            return None;
        }
        let mut ticket = rng.below(total);
        for (at, entry) in weights() {
            let weight = u64::from(entry.weight);
            if ticket < weight {
                return Some(at);
            }
            ticket -= weight;
        }
        None
    }

    /// Finds the site to which the resident must walk for the action.
    /// `Ok(None)`: the action works where the resident stands. `Err`: the
    /// city has no such site, or the scene does not know the sites.
    fn place(
        &self,
        rng: &mut Rng,
        scene: &mut dyn Scene,
        entry: &Entry,
    ) -> Result<Option<SiteId>, ()> {
        if entry.at.is_empty() {
            return Ok(None);
        }
        if scene.site_kind().is_some_and(|kind| entry.at.iter().any(|at| at == kind)) {
            return Ok(None);
        }
        let pos = scene.pos();
        let near = scene.map().ok_or(())?.nearest(&entry.at, pos, self.site_choices);
        rng.pick(&near).map(|site| Some(site.id)).ok_or(())
    }

    /// Chooses the next action of a resident: a verb by weight among the
    /// actions whose conditions hold, then its place and its arguments.
    /// When the place or the arguments cannot be made now (for example no
    /// resident is near), it chooses again among the other actions, a few
    /// times. Returns `None` when no try gave an action.
    pub fn pick(&self, rng: &mut Rng, scene: &mut dyn Scene, memory: Memory) -> Option<Choice> {
        let mut open = self.open(scene, memory);
        for _ in 0..self.attempts.max(1) {
            let at = self.weighted(rng, open)?;
            let entry = &self.entries[at];
            if let Ok(walk_to) = self.place(rng, scene, entry)
                && let Some(args) = entry.args.generate(rng, scene, &self.items)
            {
                return Some(Choice { verb: entry.verb.clone(), args, walk_to, mark: entry.mark });
            }
            open &= !(1 << at);
        }
        None
    }
}
