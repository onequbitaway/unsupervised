//! The verb `work`: one shift at the site where the resident is.
//!
//! With a job at the site, the shift does the task of the job for the
//! owner and the till of the site pays the wage. Without a job, at a city
//! hall or a depot, the shift is public works.

use serde_json::{Value, json};
use unsup_kernel::{ActionCtx, ActionError, Begin, Dest, Holder, Money, Realm};

use super::ticks;
use crate::calc;
use crate::events;
use crate::ids::Econ;
use crate::labour::{self, Labour};
use crate::public_works;
use crate::site;
use crate::state::jobs::{Job, JobBook};

fn job_here(realm: &Realm, ctx: &ActionCtx, site: unsup_kernel::SiteId) -> Option<Job> {
    realm.res::<JobBook>().job_of(site, ctx.resident).cloned()
}

fn till_short(wage: i64, till: i64) -> ActionError {
    ActionError::insufficient("The till of this site cannot pay your wage.")
        .with_details(json!({ "needed": wage, "available": till }))
}

/// Starts a shift.
pub(crate) fn begin(econ: &Econ, realm: &Realm, ctx: &ActionCtx) -> Result<Begin, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let shift = Begin::Busy { ticks: ticks(realm.param(econ.ids.shift_minutes)) };
    if let Some(job) = job_here(realm, ctx, here.id) {
        labour::check_energy(econ, realm, ctx.idx)?;
        let till = realm.ledger().money(Holder::Site(here.id)).0;
        if till < job.wage {
            return Err(till_short(job.wage, till));
        }
        if let Labour::Recipe(recipe) =
            Labour::resolve(econ, realm, &here.kind, job.task.as_deref())?
            && labour::batches_in_stock(realm, here.id, recipe) < 1
        {
            return Err(labour::lacks_inputs(realm, here.id, recipe, 1));
        }
        return Ok(shift);
    }
    if let Some(wage) = public_works::wage_at(econ, realm, &here) {
        labour::check_energy(econ, realm, ctx.idx)?;
        public_works::begin(realm, ctx, &here, wage)?;
        return Ok(shift);
    }
    Err(ActionError::not_allowed(
        "You have no job at this site. Use `apply_job` here, or work at a city hall or a depot.",
    ))
}

/// Ends a shift: does the task, pays the wage and trains the skill.
pub(crate) fn finish(econ: &Econ, realm: &Realm, ctx: &ActionCtx) -> Result<Value, ActionError> {
    if let Some(shift) = public_works::take(realm, ctx.action_id) {
        return public_works::finish(econ, realm, ctx, &shift);
    }
    let ids = &econ.ids;
    let here = site::here(realm, ctx.idx)
        .map_err(|_| ActionError::not_here("You left the site before the shift ended."))?;
    let Some(job) = job_here(realm, ctx, here.id) else {
        return Err(ActionError::conflict(
            "The shift is not paid: your job at this site ended, or you left the site of the shift.",
        ));
    };
    let pos = realm.pos(ctx.idx);
    // The till and the stock of a site are the money and the goods of one
    // holder.
    let place = Holder::Site(here.id);
    let available = realm.ledger().money(place).0;
    if available < job.wage {
        events::shift_unpaid(realm, ctx.resident, here.id, pos, job.wage);
        return Err(till_short(job.wage, available));
    }
    let rule = econ.rules.kind(&here.kind);
    let labour = Labour::resolve(econ, realm, &here.kind, job.task.as_deref())?;
    let skill = labour.skill(rule);
    let minutes = i64::try_from(realm.tick().since(ctx.started)).unwrap_or(0);

    // The wage: the net part in cash, the income tax by wire to the city.
    let tax = calc::share(job.wage, realm.param(ids.income_tax_bp));
    let worker = Holder::Resident(ctx.resident);
    realm.ledger_mut().transfer_money(place, worker, Money(job.wage - tax), "wage")?;
    realm.wire_money(place, Dest::Global, site::city_holder(realm), Money(tax), "income_tax")?;

    // The task. Everything that it makes goes into the stock of the site.
    let mut made: Vec<(unsup_kernel::ItemId, i64)> = Vec::new();
    match labour {
        Labour::Gather(what) => {
            let qty = labour::harvest(econ, realm, ctx.idx, &here, what, minutes, place);
            made.push((what.item, qty));
        }
        Labour::Recipe(recipe) => {
            let extra = labour::level(realm, ctx.idx, skill) - recipe.min_level;
            let speed = realm.param(ids.speed_bonus_bp);
            let batches = calc::shift_batches(minutes, recipe.minutes, extra, speed)
                .min(labour::batches_in_stock(realm, here.id, recipe));
            labour::run_recipe(econ, realm, here.id, recipe, batches)?;
            made.extend(recipe.outputs.iter().map(|&(item, qty)| (item, qty * batches)));
        }
        Labour::Plain => {}
    }
    made.retain(|(_, qty)| *qty > 0);

    let xp = labour::finish_work(econ, realm, ctx.idx, skill, minutes);
    events::shift_completed(realm, ctx.resident, here.id, pos, job.wage, tax, &skill.name);
    let defs = realm.defs();
    let mut produced = Vec::new();
    for &(item, qty) in &made {
        let name = defs.item_name(item);
        events::goods_produced(realm, ctx.resident, here.id, pos, name, qty);
        produced.push(json!({ "item": name, "qty": qty }));
    }
    Ok(json!({
        "site": here.id,
        "job": job.id,
        "wage": job.wage,
        "tax": tax,
        "net": job.wage - tax,
        "skill": skill.name,
        "xp": xp,
        "produced": produced,
    }))
}
