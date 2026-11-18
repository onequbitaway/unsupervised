//! Appointments: the verbs `appoint` and `dismiss` of the executive.
//!
//! The mayor fills the police and the court of its city. The president
//! fills the central bank. Each post is a role with its permissions, so an
//! appointment gives the resident the powers of the post in the next
//! frame, and a dismissal takes them away.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Builder, CityId, EventMeta, GlobalCtx, OrgId, Realm, ResidentId, Value,
};
use unsup_society::permissions::GOV_APPOINT;

use crate::model::{Gov, Post};
use crate::offices;
use crate::params::{self, Params};
use crate::remote::spec;
use crate::state::Politics;

/// Arguments of `appoint` and `dismiss`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct PostArgs {
    /// The post: `police_chief`, `police_officer` or `judge` of a city, or
    /// `central_banker` of the nation.
    pub(crate) post: Post,
    /// The resident that gets or loses the post.
    pub(crate) resident: ResidentId,
    /// The city of a city post. Leave it out when you are the mayor of one
    /// city: that city is used.
    #[serde(default)]
    pub(crate) city: Option<CityId>,
}

/// One post of one government, and the role behind it.
struct Target {
    /// The government whose executive fills the post.
    gov: Gov,
    /// The organization that has the role.
    org: OrgId,
    /// Name of the role.
    role: &'static str,
}

/// Finds the post that the arguments name and checks that the resident is
/// the executive that fills it.
fn target(realm: &Realm, ctx: &GlobalCtx, args: &PostArgs) -> Result<Target, ActionError> {
    let layout = realm.layout();
    let city = match args.city {
        Some(city) if layout.city(city).is_none() => {
            return Err(ActionError::not_found(format!("No city has the id {city}.")));
        }
        Some(city) => city,
        // A national post has no city; the number is not used then.
        None if args.post.is_national() => CityId(0),
        None => layout
            .cities()
            .iter()
            .map(|info| info.id)
            .find(|&id| offices::can(realm, OrgId::city(id), ctx.resident, GOV_APPOINT))
            .ok_or_else(|| {
                ActionError::not_allowed(
                    "You need the permission `gov.appoint` in the government of a city.",
                )
            })?,
    };
    let gov = offices::post_gov(args.post, city);
    offices::require(realm, gov, ctx.resident, GOV_APPOINT)?;
    Ok(Target { gov, org: offices::post_org(args.post, city), role: offices::post_role(args.post) })
}

/// Returns the fields of an appointment in an event, a notice or a result.
fn facts(target: &Target, ctx: &GlobalCtx, args: &PostArgs) -> Value {
    target.gov.describe_with(json!({
        "post": args.post.name(),
        "resident": args.resident,
        "by": ctx.resident,
        "org": target.org,
    }))
}

fn appoint(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &PostArgs,
) -> Result<Value, ActionError> {
    let target = target(realm, ctx, args)?;
    if args.resident == ctx.resident {
        return Err(ActionError::invalid_args("You cannot appoint yourself."));
    }
    if !realm.directory().is_alive(args.resident) {
        return Err(ActionError::not_found(format!("No resident has the id {}.", args.resident)));
    }
    // A resident has one role in an organization. An appointment must not
    // take an office away without a word.
    match offices::role_of(realm, target.org, args.resident) {
        Some(role) if role == target.role => {
            return Err(ActionError::conflict("That resident holds this post already."));
        }
        Some(role) => {
            return Err(ActionError::conflict(format!(
                "That resident holds the role `{role}` in this organization. A resident has one role in an organization."
            )));
        }
        None => {}
    }
    let mut holders = offices::holders(realm, target.org, target.role);
    let seats = offices::post_seats(realm, p, target.gov, args.post);
    if holders.len() as u64 >= seats {
        return Err(ActionError::conflict(format!(
            "Every seat of this post is taken. The post has {seats}. Dismiss a holder first."
        ))
        .with_details(json!({ "seats": seats, "holders": holders })));
    }
    holders.push(args.resident);
    let now = offices::install(realm, target.org, target.role, &holders);
    if !now.contains(&args.resident) {
        return Err(ActionError::internal("The registry refused the appointment."));
    }
    let facts = facts(&target, ctx, args);
    realm.notify(args.resident, "politics.appointed", Some(ctx.resident), facts.clone());
    realm.emit(
        EventMeta::public("politics.official_appointed", params::drama(realm, p.drama_appointed))
            .actor(ctx.resident)
            .at(ctx.pos),
        &facts,
    );
    realm.res_mut::<Politics>().touch_government(target.gov);
    Ok(facts)
}

fn dismiss(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &PostArgs,
) -> Result<Value, ActionError> {
    let target = target(realm, ctx, args)?;
    let mut holders = offices::holders(realm, target.org, target.role);
    if !holders.contains(&args.resident) {
        return Err(ActionError::not_found("That resident does not hold this post."));
    }
    holders.retain(|&holder| holder != args.resident);
    offices::install(realm, target.org, target.role, &holders);
    let facts = facts(&target, ctx, args);
    realm.notify(args.resident, "politics.dismissed", Some(ctx.resident), facts.clone());
    realm.emit(
        EventMeta::public("politics.official_dismissed", params::drama(realm, p.drama_dismissed))
            .actor(ctx.resident)
            .at(ctx.pos),
        &facts,
    );
    realm.res_mut::<Politics>().touch_government(target.gov);
    Ok(facts)
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<PostArgs>(
        spec(
            "appoint",
            "As mayor, appoint a police chief, a police officer or a judge of your city. As president, appoint the central banker. You need `gov.appoint`.",
        ),
        move |realm, ctx, args| appoint(realm, &p, ctx, args),
    );
    b.global_verb::<PostArgs>(
        spec(
            "dismiss",
            "Take a post away from a resident that you or an earlier executive appointed. You need `gov.appoint`.",
        ),
        move |realm, ctx, args| dismiss(realm, &p, ctx, args),
    );
}
