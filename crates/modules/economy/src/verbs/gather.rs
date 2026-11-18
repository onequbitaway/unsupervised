//! The verb `gather`: take raw goods from the node of a farm, a mine or a
//! forest.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Begin, Holder, Realm, Site};

use super::ticks;
use crate::events;
use crate::ids::Econ;
use crate::labour::{self, Labour};
use crate::rules::Yield;
use crate::site;
use crate::state::jobs::JobBook;
use crate::state::nodes::Nodes;

/// Arguments of `gather`.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct GatherArgs {
    /// The good to gather, for example `grain`. The first good of the site
    /// when absent. The `business` section shows what a site gives.
    #[serde(default)]
    pub item: Option<String>,
    /// Game minutes to work. The parameter `economy.gather_minutes` when
    /// absent.
    #[serde(default)]
    pub minutes: Option<u32>,
}

/// Checks that the resident can gather at the site and returns the good.
fn check(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    here: &Site,
    item: Option<&str>,
) -> Result<Yield, ActionError> {
    let rule = econ.rules.kind(&here.kind);
    if rule.gather.is_empty() {
        return Err(ActionError::not_here(
            "There is nothing to gather here. Go to a farm, a mine or a forest.",
        ));
    }
    if realm.res::<JobBook>().job_of(here.id, ctx.resident).is_some() {
        return Err(ActionError::not_allowed(
            "You have a job at this site. Use `work`: the site pays you a wage.",
        ));
    }
    if !site::controls(realm, ctx.idx, here) && !site::is_government(here.owner) {
        return Err(ActionError::not_allowed(
            "This land has an owner. Gather on land of the city, take a job here, or buy the site.",
        ));
    }
    match Labour::resolve(econ, realm, &here.kind, item)? {
        Labour::Gather(what) => Ok(what),
        Labour::Recipe(_) | Labour::Plain => {
            Err(ActionError::internal("A site with a node gave no good."))
        }
    }
}

/// Starts to gather.
pub(crate) fn begin(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &GatherArgs,
) -> Result<Begin, ActionError> {
    let ids = &econ.ids;
    let here = site::here(realm, ctx.idx)?;
    check(econ, realm, ctx, &here, args.item.as_deref())?;
    let (least, most) = (realm.param(ids.gather_minutes_min), realm.param(ids.gather_minutes_max));
    let minutes = args.minutes.map_or(realm.param(ids.gather_minutes), i64::from);
    if minutes < least || minutes > most {
        return Err(ActionError::invalid_args(format!(
            "`minutes` must be from {least} to {most}."
        )));
    }
    labour::check_energy(econ, realm, ctx.idx)?;
    let rule = econ.rules.kind(&here.kind);
    if realm.res::<Nodes>().stock(here.id, realm.tick().0, rule) <= 0 {
        return Err(ActionError::insufficient(
            "Nothing is left to gather here now. The site gets goods back with time.",
        ));
    }
    Ok(Begin::Busy { ticks: ticks(minutes) })
}

/// Ends the action: the goods go to the resident.
pub(crate) fn finish(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &GatherArgs,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)
        .map_err(|_| ActionError::not_here("You left the site before you finished."))?;
    let what = check(econ, realm, ctx, &here, args.item.as_deref())?;
    let rule = econ.rules.kind(&here.kind);
    let minutes = i64::try_from(realm.tick().since(ctx.started)).unwrap_or(0);
    let to = Holder::Resident(ctx.resident);
    let qty = labour::harvest(econ, realm, ctx.idx, &here, what, minutes, to);
    let xp = labour::finish_work(econ, realm, ctx.idx, &rule.skill, minutes);
    let name = realm.defs().item_name(what.item);
    if qty > 0 {
        events::goods_produced(realm, ctx.resident, here.id, realm.pos(ctx.idx), name, qty);
    }
    let left = realm.res::<Nodes>().stock(here.id, realm.tick().0, rule);
    Ok(json!({
        "site": here.id,
        "item": name,
        "qty": qty,
        "skill": rule.skill.name,
        "xp": xp,
        "node": left,
    }))
}
