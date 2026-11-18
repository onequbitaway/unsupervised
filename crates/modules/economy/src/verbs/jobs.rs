//! The verbs of employment: `post_job`, `close_job`, `apply_job`,
//! `quit_job` and `fire`.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::{ActionCtx, ActionError, Dest, Holder, Message, Realm, ResidentId};

use super::Phase;
use crate::board;
use crate::events;
use crate::hiring;
use crate::ids::Econ;
use crate::labour::{self, Labour};
use crate::site;
use crate::state::jobs::{Job, JobBook};
use crate::state::ties::{JobTie, TieChange, Ties};

/// Notice to the owner of a site when a resident takes a job there.
pub const NOTICE_WORKER_HIRED: &str = "economy.worker_hired";

/// Arguments of `post_job`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PostJobArgs {
    /// Wage in cents for one shift, before income tax. The till of the
    /// site pays it when a shift ends.
    pub wage: i64,
    /// How many workers the job takes.
    pub slots: u32,
    /// Least level that a worker needs in the skill of the job. 0 when
    /// absent.
    #[serde(default)]
    pub min_level: u32,
    /// What a shift does. At a farm, a mine or a forest: the name of the
    /// good to gather. At a workshop, a factory or a restaurant: the name
    /// of a recipe. Absent: the first good of the site, or a shift that
    /// makes nothing.
    #[serde(default)]
    pub task: Option<String>,
}

/// Arguments of `close_job` and `apply_job`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct JobArgs {
    /// Id of the job. `list_jobs` and the `business` section show it.
    pub job: u64,
}

/// Arguments of `fire`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FireArgs {
    /// The worker to fire.
    pub resident: ResidentId,
}

/// A worker in another zone quits. The zone of the job takes it out of the
/// job.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct JobLeave {
    job: u64,
    resident: ResidentId,
}

impl Message for JobLeave {
    const KIND: &'static str = "economy.job_leave";
}

/// Handles a [`JobLeave`] in the zone of the job.
pub(crate) fn on_job_leave(realm: &Realm, leave: &JobLeave) {
    hiring::remove_worker(realm, leave.job, leave.resident, "quit");
}

fn not_hirer() -> ActionError {
    ActionError::not_allowed(
        "Only the owner of this site, or a member of its owner with `site.manage` or `job.hire`, can do this.",
    )
}

/// The verb `post_job`: offers a job at the site where the resident is.
pub(crate) fn post_job(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &PostJobArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let ids = &econ.ids;
    let here = site::here(realm, ctx.idx)?;
    if !site::may_hire(realm, ctx.idx, &here) {
        return Err(not_hirer());
    }
    let rule = econ.rules.kind(&here.kind);
    if rule.business && !site::is_open(&here) {
        return Err(ActionError::not_allowed(
            "This business has no permit. Use `open_business` first.",
        ));
    }
    let wage_max = realm.param(ids.wage_max);
    if args.wage < 1 || args.wage > wage_max {
        return Err(ActionError::invalid_args(format!(
            "The wage must be from 1 to {wage_max} cents."
        )));
    }
    let min_wage = realm.param(ids.min_wage);
    if args.wage < min_wage {
        return Err(ActionError::not_allowed(format!(
            "The wage is below the minimum wage of {min_wage} cents for one shift."
        ))
        .with_details(json!({ "min_wage": min_wage })));
    }
    let slots_max = realm.param(ids.job_slots_max);
    if args.slots < 1 || i64::from(args.slots) > slots_max {
        return Err(ActionError::invalid_args(format!(
            "A job takes from 1 to {slots_max} workers."
        )));
    }
    let labour = Labour::resolve(econ, realm, &here.kind, args.task.as_deref())?;
    let mut min_level = i64::from(args.min_level);
    if let Labour::Recipe(recipe) = labour {
        let shift = realm.param(ids.shift_minutes);
        if recipe.minutes > shift {
            return Err(ActionError::invalid_args(format!(
                "One batch of `{}` takes {} minutes. A shift has {shift} minutes.",
                recipe.name, recipe.minutes
            )));
        }
        // A worker needs at least the level of the recipe.
        min_level = min_level.max(recipe.min_level);
    }
    let jobs_max = realm.param(ids.site_jobs_max);
    let count = realm.res::<JobBook>().at_site(here.id).count();
    if i64::try_from(count).unwrap_or(i64::MAX) >= jobs_max {
        return Err(ActionError::conflict(format!(
            "This site has {jobs_max} jobs already. Close one first."
        )));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let id = realm.alloc_id();
    let job = Job {
        id,
        site: here.id,
        wage: args.wage,
        slots: args.slots,
        min_level,
        task: args.task.clone(),
        workers: Vec::new(),
    };
    realm.res_mut::<JobBook>().jobs.insert(id, job);
    board::touch(realm);
    events::job_posted(realm, ctx.resident, realm.pos(ctx.idx), id, here.id, args.wage, args.slots);
    Ok(json!({
        "job": id,
        "site": here.id,
        "skill": labour.skill(rule).name,
        "min_level": min_level,
    }))
}

/// The verb `close_job`: ends a job at the site where the resident is.
pub(crate) fn close_job(
    realm: &Realm,
    ctx: &ActionCtx,
    args: &JobArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let job_site = realm
        .res::<JobBook>()
        .jobs
        .get(&args.job)
        .map(|job| job.site)
        .ok_or_else(|| no_job(args.job))?;
    if job_site != here.id {
        return Err(ActionError::not_here("That job is at another site. Go there first."));
    }
    if !site::may_hire(realm, ctx.idx, &here) {
        return Err(not_hirer());
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    let released = hiring::close_job(realm, args.job, "closed").unwrap_or(0);
    Ok(json!({ "job": args.job, "released": released }))
}

fn no_job(job: u64) -> ActionError {
    ActionError::not_found(format!("This district has no job {job}. Use `list_jobs`."))
}

/// The verb `apply_job`: takes a job at the site where the resident is.
pub(crate) fn apply_job(
    econ: &Econ,
    realm: &Realm,
    ctx: &ActionCtx,
    args: &JobArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    let job =
        realm.res::<JobBook>().jobs.get(&args.job).cloned().ok_or_else(|| no_job(args.job))?;
    if job.site != here.id {
        return Err(ActionError::not_here("That job is at another site. Go there first."));
    }
    if realm.col::<Ties>()[ctx.idx].job().is_some() {
        return Err(ActionError::conflict("You have a job already. Use `quit_job` first."));
    }
    if job.free_slots() == 0 {
        return Err(ActionError::conflict("The job takes no more workers."));
    }
    let rule = econ.rules.kind(&here.kind);
    let labour = Labour::resolve(econ, realm, &here.kind, job.task.as_deref())?;
    let skill = labour.skill(rule);
    let level = labour::level(realm, ctx.idx, skill);
    if level < job.min_level {
        return Err(ActionError::not_allowed(format!(
            "The job needs level {} in {}. You have level {level}.",
            job.min_level, skill.name
        ))
        .with_details(json!({ "skill": skill.name, "needed": job.min_level, "level": level })));
    }
    if phase.is_check() {
        return Ok(Value::Null);
    }
    if let Some(entry) = realm.res_mut::<JobBook>().jobs.get_mut(&args.job) {
        entry.workers.push(ctx.resident);
    }
    let zone = realm.zone_info().map(|zone| zone.id).unwrap_or_default();
    let tie = JobTie { job: job.id, site: here.id, zone, kind: here.kind.clone(), wage: job.wage };
    realm.col_mut::<Ties>()[ctx.idx].apply(TieChange::Hired(tie));
    board::touch(realm);
    events::worker_hired(realm, ctx.resident, realm.pos(ctx.idx), here.id, job.id, job.wage);
    if let Some(Holder::Resident(owner)) = here.owner
        && owner != ctx.resident
    {
        realm.notify(
            owner,
            NOTICE_WORKER_HIRED,
            Some(ctx.resident),
            json!({ "job": job.id, "site": here.id }),
        );
    }
    Ok(json!({ "job": job.id, "site": here.id, "wage": job.wage, "skill": skill.name }))
}

/// The verb `quit_job`: leaves the job. It works from every place.
pub(crate) fn quit_job(realm: &Realm, ctx: &ActionCtx, phase: Phase) -> Result<Value, ActionError> {
    let Some(tie) = realm.col::<Ties>()[ctx.idx].job().cloned() else {
        return Err(ActionError::not_found("You have no job."));
    };
    if phase.is_check() {
        return Ok(Value::Null);
    }
    realm.col_mut::<Ties>()[ctx.idx].apply(TieChange::JobEnded { job: tie.job });
    if realm.zone_info().is_some_and(|zone| zone.id == tie.zone) {
        hiring::remove_worker(realm, tie.job, ctx.resident, "quit");
    } else {
        realm.send(Dest::Zone(tie.zone), &JobLeave { job: tie.job, resident: ctx.resident });
    }
    Ok(json!({ "job": tie.job, "site": tie.site }))
}

/// The verb `fire`: ends the job of a worker at the site where the
/// resident is.
pub(crate) fn fire(
    realm: &Realm,
    ctx: &ActionCtx,
    args: &FireArgs,
    phase: Phase,
) -> Result<Value, ActionError> {
    let here = site::here(realm, ctx.idx)?;
    if !site::may_hire(realm, ctx.idx, &here) {
        return Err(not_hirer());
    }
    let job = realm
        .res::<JobBook>()
        .job_of(here.id, args.resident)
        .map(|job| job.id)
        .ok_or_else(|| ActionError::not_found("That resident has no job at this site."))?;
    if phase.is_check() {
        return Ok(Value::Null);
    }
    hiring::remove_worker(realm, job, args.resident, "fired");
    Ok(json!({ "job": job, "resident": args.resident }))
}
