//! What every kind of work shares: the energy that it costs, the
//! experience that it gives, and what a shift at a site does.

use unsup_kernel::{ActionError, Holder, Idx, ItemId, Realm, Site, SiteId};

use crate::calc;
use crate::ids::Econ;
use crate::rules::{KindRule, Recipe, Skill, Yield};
use crate::state::nodes::Nodes;

/// What a shift or a `gather` or `produce` action does at a site.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Labour<'a> {
    /// Gather a good from the node of the site.
    Gather(Yield),
    /// Run a recipe with the stock of the site.
    Recipe(&'a Recipe),
    /// Only be there: a shift at a shop, an office or a city hall.
    Plain,
}

impl<'a> Labour<'a> {
    /// Finds what a task means at a site kind. Without a task, a site with
    /// a node gives its first good and every other site a plain shift.
    pub(crate) fn resolve(
        econ: &'a Econ,
        realm: &Realm,
        kind: &str,
        task: Option<&str>,
    ) -> Result<Labour<'a>, ActionError> {
        let rule = econ.rules.kind(kind);
        if let Some(first) = rule.gather.first() {
            let Some(name) = task else {
                return Ok(Labour::Gather(*first));
            };
            let item = realm.defs().item_id(name).ok_or_else(|| unknown_good(name))?;
            return rule
                .gather
                .iter()
                .find(|y| y.item == item)
                .map(|y| Labour::Gather(*y))
                .ok_or_else(|| {
                    ActionError::invalid_args(format!("A {kind} does not give `{name}`."))
                });
        }
        let Some(name) = task else {
            return Ok(Labour::Plain);
        };
        let recipe = econ
            .rules
            .recipe(name)
            .ok_or_else(|| ActionError::not_found(format!("No recipe is named `{name}`.")))?;
        if !recipe.works_at(kind) {
            return Err(ActionError::invalid_args(format!(
                "The recipe `{name}` does not work at a {kind}. It needs: {}.",
                recipe.sites.join(", ")
            )));
        }
        Ok(Labour::Recipe(recipe))
    }

    /// Returns the skill that the labour uses and trains.
    pub(crate) fn skill(&self, rule: &'a KindRule) -> &'a Skill {
        match self {
            Labour::Recipe(recipe) => &recipe.skill,
            Labour::Gather(_) | Labour::Plain => &rule.skill,
        }
    }
}

/// The error for a good that is not in the catalogue.
pub(crate) fn unknown_good(name: &str) -> ActionError {
    ActionError::not_found(format!("No good is named `{name}`. See the item catalogue."))
}

/// Finds a good by name.
pub(crate) fn good(realm: &Realm, name: &str) -> Result<ItemId, ActionError> {
    realm.defs().item_id(name).ok_or_else(|| unknown_good(name))
}

/// Returns the level of a resident in a skill.
pub(crate) fn level(realm: &Realm, idx: Idx, skill: &Skill) -> i64 {
    calc::level(realm.attr(idx, skill.attr))
}

/// Returns the share of the normal output that the mood of a resident
/// gives, in basis points.
pub(crate) fn mood_bp(econ: &Econ, realm: &Realm, idx: Idx) -> i64 {
    calc::mood_bp(realm.attr(idx, econ.ids.mood), realm.param(econ.ids.mood_floor_bp))
}

/// Refuses work when the resident is too tired.
pub(crate) fn check_energy(econ: &Econ, realm: &Realm, idx: Idx) -> Result<(), ActionError> {
    let needed = realm.param(econ.ids.labor_min_energy);
    let energy = realm.attr(idx, econ.ids.energy);
    if energy < needed {
        return Err(ActionError::insufficient("You are too tired to work. Rest or sleep first.")
            .with_details(serde_json::json!({ "needed": needed, "available": energy })));
    }
    Ok(())
}

/// Takes the energy that `minutes` of work cost and gives the experience.
/// Returns the experience that the resident gained.
pub(crate) fn finish_work(
    econ: &Econ,
    realm: &Realm,
    idx: Idx,
    skill: &Skill,
    minutes: i64,
) -> i64 {
    let ids = &econ.ids;
    let cost = calc::energy_cost(realm.param(ids.energy_per_hour), minutes);
    realm.attr_add(idx, ids.energy, -cost);
    let xp = calc::xp_gain(realm.param(ids.xp_per_hour), minutes, mood_bp(econ, realm, idx));
    realm.attr_add(idx, skill.attr, xp);
    xp
}

/// Gathers for `minutes` at a site and gives the goods to `to` through the
/// faucet `nature.harvest`. Returns the units, which the stock of the node
/// limits.
pub(crate) fn harvest(
    econ: &Econ,
    realm: &Realm,
    idx: Idx,
    site: &Site,
    what: Yield,
    minutes: i64,
    to: Holder,
) -> i64 {
    let ids = &econ.ids;
    let rule = econ.rules.kind(&site.kind);
    let wanted = calc::harvest(
        what.per_hour,
        minutes,
        level(realm, idx, &rule.skill),
        realm.param(ids.skill_bonus_bp),
        mood_bp(econ, realm, idx),
    );
    let qty = realm.res_mut::<Nodes>().take(site.id, realm.tick().0, rule, wanted);
    realm.ledger_mut().produce(ids.harvest, to, what.item, qty);
    qty
}

/// Returns how many batches of a recipe the stock of a site can feed.
pub(crate) fn batches_in_stock(realm: &Realm, site: SiteId, recipe: &Recipe) -> i64 {
    let ledger = realm.ledger();
    recipe
        .inputs
        .iter()
        .map(|&(item, qty)| ledger.qty(Holder::Site(site), item) / qty)
        .min()
        .unwrap_or(0)
}

/// Runs `batches` of a recipe at a site: the inputs leave the stock through
/// the sink `production.input` and the outputs enter it through the faucet
/// `production.output`. Nothing changes when an input is missing.
pub(crate) fn run_recipe(
    econ: &Econ,
    realm: &Realm,
    site: SiteId,
    recipe: &Recipe,
    batches: i64,
) -> Result<(), ActionError> {
    if batches <= 0 {
        return Ok(());
    }
    if batches_in_stock(realm, site, recipe) < batches {
        return Err(lacks_inputs(realm, site, recipe, batches));
    }
    let stock = Holder::Site(site);
    let mut ledger = realm.ledger_mut();
    for &(item, qty) in &recipe.inputs {
        ledger.consume(stock, econ.ids.input, item, qty.saturating_mul(batches))?;
    }
    for &(item, qty) in &recipe.outputs {
        ledger.produce(econ.ids.output, stock, item, qty.saturating_mul(batches));
    }
    Ok(())
}

/// The error for a site whose stock cannot feed a recipe.
pub(crate) fn lacks_inputs(
    realm: &Realm,
    site: SiteId,
    recipe: &Recipe,
    batches: i64,
) -> ActionError {
    let ledger = realm.ledger();
    let defs = realm.defs();
    let missing: serde_json::Map<String, serde_json::Value> = recipe
        .inputs
        .iter()
        .filter_map(|&(item, qty)| {
            let needed = qty.saturating_mul(batches);
            let available = ledger.qty(Holder::Site(site), item);
            (available < needed).then(|| {
                (
                    defs.item_name(item).to_string(),
                    serde_json::json!({ "needed": needed, "available": available }),
                )
            })
        })
        .collect();
    ActionError::insufficient(format!(
        "The stock of this site lacks inputs of the recipe `{}`. Use `stock` to add them.",
        recipe.name
    ))
    .with_details(serde_json::json!({ "missing": missing }))
}
