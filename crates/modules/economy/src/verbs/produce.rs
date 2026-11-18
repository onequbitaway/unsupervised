//! The verbs `produce` and `list_recipes`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Begin, ItemId, Realm, Site};

use super::ticks;
use crate::calc;
use crate::events;
use crate::ids::Econ;
use crate::labour;
use crate::rules::Recipe;
use crate::site;
use crate::state::jobs::JobBook;

/// Arguments of `produce`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProduceArgs {
    /// Name of the recipe. `list_recipes` shows every recipe.
    pub recipe: String,
    /// How many batches to make. 1 when absent.
    #[serde(default)]
    pub batches: Option<u32>,
}

/// Arguments of `list_recipes`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct ListRecipesArgs {
    /// Only recipes that work at this site kind, for example `workshop`.
    #[serde(default)]
    pub site: Option<String>,
    /// Only recipes that make this good, for example `bread`.
    #[serde(default)]
    pub output: Option<String>,
}

/// A request that passed every check.
struct Plan<'a> {
    here: Site,
    recipe: &'a Recipe,
    batches: i64,
    /// Levels of the resident above the least level of the recipe.
    extra_levels: i64,
}

fn check<'a>(
    econ: &'a Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &ProduceArgs,
) -> Result<Plan<'a>, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let recipe = econ.rules.recipe(&args.recipe).ok_or_else(|| {
        ActionError::not_found(format!("No recipe is named `{}`. Use `list_recipes`.", args.recipe))
    })?;
    if !recipe.works_at(&here.kind) {
        return Err(ActionError::not_here(format!(
            "The recipe `{}` does not work at a {}. It needs: {}.",
            recipe.name,
            here.kind,
            recipe.sites.join(", ")
        )));
    }
    if econ.rules.kind(&here.kind).business && !site::is_open(&here) {
        return Err(ActionError::not_allowed(
            "This business has no permit. Its owner must use `open_business` first.",
        ));
    }
    let employed = realm.res::<JobBook>().job_of(here.id, ctx.resident).is_some();
    if !employed && !site::controls(realm, ctx.idx, &here) {
        return Err(ActionError::not_allowed(
            "Only the owner of this site, a member of its owner with `site.manage`, or a worker of the site can produce here.",
        ));
    }
    let most = realm.param(econ.ids.produce_batches_max);
    let batches = args.batches.map_or(1, i64::from);
    if batches < 1 || batches > most {
        return Err(ActionError::invalid_args(format!("`batches` must be from 1 to {most}.")));
    }
    let level = labour::level(realm, ctx.idx, &recipe.skill);
    if level < recipe.min_level {
        return Err(ActionError::not_allowed(format!(
            "The recipe `{}` needs level {} in {}. You have level {level}.",
            recipe.name, recipe.min_level, recipe.skill.name
        ))
        .with_details(json!({
            "skill": recipe.skill.name,
            "needed": recipe.min_level,
            "level": level,
        })));
    }
    if labour::batches_in_stock(realm, here.id, recipe) < batches {
        return Err(labour::lacks_inputs(realm, here.id, recipe, batches));
    }
    Ok(Plan { here, recipe, batches, extra_levels: level - recipe.min_level })
}

/// Starts to produce. The time grows with the batches and falls with the
/// skill.
pub(crate) fn begin(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &ProduceArgs,
) -> Result<Begin, ActionError> {
    let plan = check(econ, realm, ctx, args)?;
    labour::check_energy(econ, realm, ctx.idx)?;
    let minutes = calc::produce_ticks(
        plan.recipe.minutes,
        plan.batches,
        plan.extra_levels,
        realm.param(econ.ids.speed_bonus_bp),
    );
    Ok(Begin::Busy { ticks: ticks(minutes) })
}

/// Ends the action: the inputs leave the stock of the site and the outputs
/// enter it.
pub(crate) fn finish(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &ProduceArgs,
) -> Result<Value, ActionError> {
    let plan = check(econ, realm, ctx, args)?;
    let Plan { here, recipe, batches, .. } = plan;
    labour::run_recipe(econ, realm, here.id, recipe, batches)?;
    let minutes = i64::try_from(realm.tick().since(ctx.started)).unwrap_or(0);
    let xp = labour::finish_work(econ, realm, ctx.idx, &recipe.skill, minutes);
    let pos = realm.pos(ctx.idx);
    let defs = realm.defs();
    let mut outputs = Vec::new();
    for &(item, qty) in &recipe.outputs {
        let (name, qty) = (defs.item_name(item), qty * batches);
        events::goods_produced(realm, ctx.resident, here.id, pos, name, qty);
        outputs.push(json!({ "item": name, "qty": qty }));
    }
    Ok(json!({
        "site": here.id,
        "recipe": recipe.name,
        "batches": batches,
        "outputs": outputs,
        "skill": recipe.skill.name,
        "xp": xp,
    }))
}

fn goods(realm: &Realm, list: &[(ItemId, i64)]) -> Value {
    let defs = realm.defs();
    let map: Map<String, Value> =
        list.iter().map(|&(item, qty)| (defs.item_name(item).to_string(), json!(qty))).collect();
    Value::Object(map)
}

/// The verb `list_recipes`: every recipe, or those that match.
pub(crate) fn list_recipes(
    econ: &Econ,
    realm: &Realm,
    args: &ListRecipesArgs,
) -> Result<Value, ActionError> {
    let output = match &args.output {
        Some(name) => Some(labour::good(realm, name)?),
        None => None,
    };
    let recipes: Vec<Value> = econ
        .rules
        .recipes
        .iter()
        .filter(|r| args.site.as_ref().is_none_or(|kind| r.works_at(kind)))
        .filter(|r| output.is_none_or(|item| r.outputs.iter().any(|(out, _)| *out == item)))
        .map(|r| {
            json!({
                "recipe": r.name,
                "sites": r.sites,
                "skill": r.skill.name,
                "min_level": r.min_level,
                "minutes": r.minutes,
                "inputs": goods(realm, &r.inputs),
                "outputs": goods(realm, &r.outputs),
            })
        })
        .collect();
    Ok(json!({ "recipes": recipes }))
}
