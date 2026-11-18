//! The rules that come from the data files: site kinds, recipes and depot
//! prices.
//!
//! `data/items.toml` and `data/balance.toml` have the format of the kernel
//! and go through `Builder::load_data`. The other files have their own
//! tables. A mistake in a data file is a bug and panics when the world is
//! made, like every other mistake in a plugin.

use std::collections::BTreeMap;

use serde::Deserialize;
use unsup_kernel::{AttrDef, AttrId, AttrShow, Builder, ItemId, ParamDef, ParamId};

/// The skills of the game (`docs/design/game-design.md`, section 3). The
/// life module owns them; this module declares them with the same
/// definition so that it can read and train them.
pub(crate) const SKILLS: [&str; 13] = [
    "labor",
    "farming",
    "mining",
    "crafting",
    "cooking",
    "trading",
    "management",
    "medicine",
    "law",
    "investigation",
    "stealth",
    "combat",
    "rhetoric",
];

/// The skill of a site kind that is not in `sites.toml`.
const FALLBACK_SKILL: &str = "labor";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SitesFile {
    kinds: Vec<KindData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KindData {
    kind: String,
    skill: String,
    #[serde(default)]
    business: bool,
    #[serde(default)]
    retail: bool,
    #[serde(default)]
    lettable: bool,
    #[serde(default)]
    units: u32,
    #[serde(default)]
    public_works: bool,
    #[serde(default)]
    depot: bool,
    #[serde(default)]
    gather: Vec<YieldData>,
    #[serde(default)]
    node_max: i64,
    #[serde(default)]
    node_regen_per_hour: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct YieldData {
    item: String,
    per_hour: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipesFile {
    recipes: Vec<RecipeData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeData {
    name: String,
    sites: Vec<String>,
    skill: String,
    min_level: i64,
    minutes: i64,
    inputs: BTreeMap<String, i64>,
    outputs: BTreeMap<String, i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DepotFile {
    depot: Vec<DepotData>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DepotData {
    item: String,
    floor: i64,
    ceiling: i64,
}

/// A skill: the attribute and its short name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Skill {
    /// The attribute `skill.<name>`.
    pub(crate) attr: AttrId,
    /// The short name, for example `farming`.
    pub(crate) name: String,
}

/// A good that `gather` gives at a site kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Yield {
    /// The good.
    pub(crate) item: ItemId,
    /// Units for one hour of work at level 0 and full mood.
    pub(crate) per_hour: i64,
}

/// What one site kind can do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KindRule {
    /// The skill that a shift at the site trains.
    pub(crate) skill: Skill,
    /// True when the site needs a permit to trade, to produce and to hire.
    pub(crate) business: bool,
    /// True when the owner sets prices and residents buy and sell there.
    pub(crate) retail: bool,
    /// True when the owner can let the site to tenants.
    pub(crate) lettable: bool,
    /// Tenants that a lettable site takes without the attribute `capacity`.
    pub(crate) units: u32,
    /// True when a resident without a job can work a starter shift there.
    pub(crate) public_works: bool,
    /// True when the site is the buyer and seller of last resort.
    pub(crate) depot: bool,
    /// Goods that `gather` gives. Empty when the site has no node.
    pub(crate) gather: Vec<Yield>,
    /// Greatest stock of the node.
    pub(crate) node_max: i64,
    /// Units that the node gets back each game hour.
    pub(crate) node_regen_per_hour: i64,
}

/// A recipe: inputs in the site stock become outputs in the site stock.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recipe {
    /// Name of the recipe.
    pub(crate) name: String,
    /// Site kinds where the recipe works.
    pub(crate) sites: Vec<String>,
    /// The skill that the recipe uses and trains.
    pub(crate) skill: Skill,
    /// Least level in that skill.
    pub(crate) min_level: i64,
    /// Game minutes for one batch at the least level.
    pub(crate) minutes: i64,
    /// Goods that one batch uses.
    pub(crate) inputs: Vec<(ItemId, i64)>,
    /// Goods that one batch makes.
    pub(crate) outputs: Vec<(ItemId, i64)>,
}

/// The two price parameters of one good at the depot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DepotPrice {
    /// Cents that the depot pays for one unit.
    pub(crate) floor: ParamId,
    /// Cents that one unit costs at the depot.
    pub(crate) ceiling: ParamId,
}

/// Everything that the data files say, with names resolved to ids.
#[derive(Clone, Debug)]
pub(crate) struct Rules {
    kinds: BTreeMap<String, KindRule>,
    fallback: KindRule,
    /// Every recipe, in the order of the data file.
    pub(crate) recipes: Vec<Recipe>,
    /// The goods that the depot trades.
    pub(crate) depot: BTreeMap<ItemId, DepotPrice>,
}

fn parse<T: for<'de> Deserialize<'de>>(file: &str, text: &str) -> T {
    match toml::from_str(text) {
        Ok(data) => data,
        Err(e) => panic!("plugin `economy`: invalid data file `{file}`: {e}"),
    }
}

/// Declares one skill attribute and returns it.
fn skill(b: &mut Builder, name: &str) -> Skill {
    assert!(SKILLS.contains(&name), "plugin `economy`: `{name}` is not a skill of the game");
    let attr = b.attr(
        AttrDef::new(format!("skill.{name}"))
            .range(0, i64::MAX)
            .show(AttrShow::Full)
            .summary(format!("Experience points in {name}. Level = floor(sqrt(xp / 100)).")),
    );
    Skill { attr, name: name.to_string() }
}

fn goods(b: &Builder, list: BTreeMap<String, i64>, recipe: &str) -> Vec<(ItemId, i64)> {
    list.into_iter()
        .map(|(name, qty)| {
            assert!(
                qty > 0,
                "plugin `economy`: recipe `{recipe}` has `{name}` with quantity {qty}"
            );
            (b.item_id(&name), qty)
        })
        .collect()
}

impl Rules {
    /// Reads the data files and registers what they declare: the skill
    /// attributes and the price parameters of the depot. The items must be
    /// loaded first.
    pub(crate) fn load(b: &mut Builder) -> Rules {
        for name in SKILLS {
            skill(b, name);
        }
        let sites: SitesFile = parse("sites.toml", include_str!("../data/sites.toml"));
        let mut kinds = BTreeMap::new();
        for data in sites.kinds {
            let gather: Vec<Yield> = data
                .gather
                .iter()
                .map(|y| {
                    assert!(
                        y.per_hour > 0,
                        "plugin `economy`: site kind `{}` gives `{}` at {} per hour",
                        data.kind,
                        y.item,
                        y.per_hour
                    );
                    Yield { item: b.item_id(&y.item), per_hour: y.per_hour }
                })
                .collect();
            assert!(
                gather.is_empty() || data.node_max > 0,
                "plugin `economy`: site kind `{}` has goods to gather but no node",
                data.kind
            );
            assert!(
                data.node_regen_per_hour >= 0,
                "plugin `economy`: site kind `{}` has a negative node regeneration",
                data.kind
            );
            let rule = KindRule {
                skill: skill(b, &data.skill),
                business: data.business,
                retail: data.retail,
                lettable: data.lettable,
                units: data.units.max(1),
                public_works: data.public_works,
                depot: data.depot,
                gather,
                node_max: data.node_max,
                node_regen_per_hour: data.node_regen_per_hour,
            };
            let known = kinds.insert(data.kind.clone(), rule);
            assert!(
                known.is_none(),
                "plugin `economy`: site kind `{}` is defined twice",
                data.kind
            );
        }
        let fallback = KindRule {
            skill: skill(b, FALLBACK_SKILL),
            business: false,
            retail: false,
            lettable: false,
            units: 1,
            public_works: false,
            depot: false,
            gather: Vec::new(),
            node_max: 0,
            node_regen_per_hour: 0,
        };

        let file: RecipesFile = parse("recipes.toml", include_str!("../data/recipes.toml"));
        let mut recipes: Vec<Recipe> = Vec::new();
        for data in file.recipes {
            assert!(
                recipes.iter().all(|r| r.name != data.name),
                "plugin `economy`: recipe `{}` is defined twice",
                data.name
            );
            assert!(
                data.minutes > 0 && data.min_level >= 0,
                "plugin `economy`: recipe `{}` has bad minutes or level",
                data.name
            );
            assert!(
                !data.sites.is_empty() && !data.inputs.is_empty() && !data.outputs.is_empty(),
                "plugin `economy`: recipe `{}` needs sites, inputs and outputs",
                data.name
            );
            recipes.push(Recipe {
                skill: skill(b, &data.skill),
                min_level: data.min_level,
                minutes: data.minutes,
                inputs: goods(b, data.inputs, &data.name),
                outputs: goods(b, data.outputs, &data.name),
                sites: data.sites,
                name: data.name,
            });
        }

        let file: DepotFile = parse("depot.toml", include_str!("../data/depot.toml"));
        let mut depot = BTreeMap::new();
        for data in file.depot {
            assert!(
                0 <= data.floor && data.floor <= data.ceiling,
                "plugin `economy`: the depot prices of `{}` are not 0 <= floor <= ceiling",
                data.item
            );
            let item = b.item_id(&data.item);
            let floor = b.param(
                ParamDef::new(format!("economy.depot_floor.{}", data.item), data.floor)
                    .range(0, i64::MAX)
                    .policy(true)
                    .summary(format!("Cents that the depot pays for one unit of {}.", data.item)),
            );
            let ceiling = b.param(
                ParamDef::new(format!("economy.depot_ceiling.{}", data.item), data.ceiling)
                    .range(0, i64::MAX)
                    .policy(true)
                    .summary(format!("Cents that one unit of {} costs at the depot.", data.item)),
            );
            let known = depot.insert(item, DepotPrice { floor, ceiling });
            assert!(
                known.is_none(),
                "plugin `economy`: depot item `{}` is defined twice",
                data.item
            );
        }
        Rules { kinds, fallback, recipes, depot }
    }

    /// Returns the rule of a site kind.
    pub(crate) fn kind(&self, kind: &str) -> &KindRule {
        self.kinds.get(kind).unwrap_or(&self.fallback)
    }

    /// Returns true when `open_business` can make a site of this kind.
    pub(crate) fn is_business(&self, kind: &str) -> bool {
        self.kinds.get(kind).is_some_and(|rule| rule.business)
    }

    /// Returns the names of the business kinds, in alphabetical order.
    pub(crate) fn business_kinds(&self) -> Vec<&str> {
        self.kinds.iter().filter(|(_, rule)| rule.business).map(|(name, _)| name.as_str()).collect()
    }

    /// Returns the names of the site kinds with starter shifts, in
    /// alphabetical order.
    pub(crate) fn public_works_kinds(&self) -> Vec<&str> {
        self.kinds
            .iter()
            .filter(|(_, rule)| rule.public_works)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Returns a recipe by name.
    pub(crate) fn recipe(&self, name: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.name == name)
    }
}

impl Recipe {
    /// Returns true when the recipe works at a site of this kind.
    pub(crate) fn works_at(&self, kind: &str) -> bool {
        self.sites.iter().any(|s| s == kind)
    }
}
