//! The verbs that change members, roles and grants. They run in the global
//! realm, where the registry is.

use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, GlobalCtx, OrgId, Realm, ResidentId, Value};

use super::args::{OrgArgs, OrgDefineRoleArgs, OrgGrantArgs, OrgResidentArgs, OrgSetRoleArgs};
use super::lifecycle::after_departure;
use super::mirror::{describe_with, mirror, mirror_permission, mirror_role};
use super::state::{Org, Orgs};
use crate::params::{self, Params};
use crate::{perms, spec, text};

/// The error for an organization id that the registry does not have.
pub(crate) fn no_org(id: OrgId) -> ActionError {
    ActionError::not_found(format!("No organization has the id {id}."))
}

/// Checks that a resident has a permission in an organization.
pub(crate) fn require(org: &Org, who: ResidentId, perm: &str) -> Result<(), ActionError> {
    if org.can(who, perm) {
        Ok(())
    } else {
        Err(ActionError::not_allowed(format!(
            "You need the permission `{perm}` in organization {}.",
            org.id
        )))
    }
}

/// The error for a limit that is reached.
fn full(what: &str, max: usize) -> ActionError {
    ActionError::conflict(format!("{what} The limit is {max}."))
}

fn invite(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &OrgResidentArgs,
) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    require(org, ctx.resident, perms::ORG_INVITE)?;
    if !realm.directory().is_alive(args.resident) {
        return Err(ActionError::not_found(format!("No resident has the id {}.", args.resident)));
    }
    if org.role_of(args.resident).is_some() {
        return Err(ActionError::conflict("That resident is a member already."));
    }
    let frame = realm.frame();
    if org.invited(args.resident, frame) {
        return Err(ActionError::conflict("That resident has an invitation already."));
    }
    let max = params::count(realm, p.org_invites_max);
    if org.open_invites(frame) >= max {
        return Err(full("The organization has too many open invitations.", max));
    }
    let expires = frame + realm.param(p.org_invite_ttl).max(1).unsigned_abs();
    realm.notify(
        args.resident,
        "society.org_invited",
        Some(ctx.resident),
        describe_with(org, json!({ "expires": expires })),
    );
    orgs.add_invite(args.org, args.resident, expires);
    Ok(json!({ "org": args.org, "invited": args.resident, "expires": expires }))
}

fn join(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &OrgArgs) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    if org.role_of(ctx.resident).is_some() {
        return Err(ActionError::conflict("You are a member already."));
    }
    if !org.open && !org.invited(ctx.resident, realm.frame()) {
        return Err(ActionError::not_allowed(
            "You need an open invitation to join this organization.",
        ));
    }
    let max = params::count(realm, p.memberships_max);
    if orgs.membership_count(ctx.resident) >= max {
        return Err(full("You are in too many organizations.", max));
    }
    let max = params::count(realm, p.org_members_max);
    if !org.is_government() && org.member_count() >= max {
        return Err(full("The organization has too many members.", max));
    }
    orgs.set_member(args.org, ctx.resident, perms::MEMBER_ROLE);
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    mirror(realm, org, ctx.resident);
    realm.emit(
        EventMeta::public("society.org_joined", params::drama(realm, p.drama_org_joined))
            .actor(ctx.resident)
            .at(ctx.pos),
        &describe_with(org, json!({ "resident": ctx.resident })),
    );
    Ok(json!({ "org": args.org, "role": perms::MEMBER_ROLE, "members": org.member_count() }))
}

fn leave(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &OrgArgs) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    if org.role_of(ctx.resident).is_none() {
        return Err(ActionError::conflict("You are not a member of this organization."));
    }
    let last_manager = org.can(ctx.resident, perms::ORG_MANAGE) && org.managers() == 1;
    if !org.is_government() && last_manager && org.member_count() > 1 {
        return Err(ActionError::conflict(
            "You are the only member that can manage this organization. Give another member a role with `org.manage` first.",
        ));
    }
    let role = orgs.remove_member(args.org, ctx.resident).unwrap_or_default();
    realm.revoke(ctx.resident, args.org);
    if let Some(org) = orgs.get(args.org) {
        realm.emit(
            EventMeta::public("society.org_left", params::drama(realm, p.drama_org_left))
                .actor(ctx.resident)
                .at(ctx.pos),
            &describe_with(org, json!({ "resident": ctx.resident, "cause": "left" })),
        );
    }
    after_departure(realm, p, &mut orgs, args.org, &role, Some(ctx.resident));
    Ok(json!({ "org": args.org, "dissolved": orgs.get(args.org).is_none() }))
}

fn kick(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &OrgResidentArgs,
) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    require(org, ctx.resident, perms::ORG_MANAGE)?;
    if args.resident == ctx.resident {
        return Err(ActionError::invalid_args("You cannot remove yourself. Use `org_leave`."));
    }
    if org.role_of(args.resident).is_none() {
        return Err(ActionError::not_found("That resident is not a member of this organization."));
    }
    let role = orgs.remove_member(args.org, args.resident).unwrap_or_default();
    realm.revoke(args.resident, args.org);
    if let Some(org) = orgs.get(args.org) {
        realm.notify(
            args.resident,
            "society.org_kicked",
            Some(ctx.resident),
            describe_with(org, json!({})),
        );
        realm.emit(
            EventMeta::public("society.org_left", params::drama(realm, p.drama_org_left))
                .actor(ctx.resident)
                .at(ctx.pos),
            &describe_with(
                org,
                json!({ "resident": args.resident, "cause": "kicked", "by": ctx.resident }),
            ),
        );
    }
    after_departure(realm, p, &mut orgs, args.org, &role, None);
    Ok(json!({ "org": args.org, "removed": args.resident }))
}

fn define_role(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &OrgDefineRoleArgs,
) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    require(org, ctx.resident, perms::ORG_MANAGE)?;
    let name_max = params::count(realm, p.role_name_max);
    if !text::is_ident(&args.role, name_max) {
        return Err(ActionError::invalid_args(format!(
            "A role name has lower-case letters, digits and `_`, starts with a letter and has at most {name_max} bytes."
        )));
    }
    if let Some(unknown) = args.permissions.iter().find(|perm| !perms::is_known(perm)) {
        return Err(ActionError::invalid_args(format!("No permission is named `{unknown}`."))
            .with_details(json!({ "permissions": perms::PERMISSIONS })));
    }
    let roles_max = params::count(realm, p.org_roles_max);
    if !org.roles.contains_key(&args.role) && org.roles.len() >= roles_max {
        return Err(full("The organization has too many roles.", roles_max));
    }
    let mut permissions = args.permissions.clone();
    permissions.sort();
    permissions.dedup();
    // The change must leave a member that can manage the organization.
    let keeps_manager = org.is_government()
        || permissions.iter().any(|perm| perm == perms::ORG_MANAGE)
        || org
            .members()
            .any(|(member, role)| role != args.role && org.can(member, perms::ORG_MANAGE));
    if !keeps_manager {
        return Err(ActionError::conflict(
            "After this change no member could manage the organization.",
        ));
    }
    let inactive: Vec<&String> = permissions.iter().filter(|perm| !org.holds(perm)).collect();
    let result = json!({
        "org": args.org,
        "role": args.role,
        "permissions": permissions,
        "inactive": inactive,
    });
    if let Some(org) = orgs.get_mut(args.org) {
        org.roles.insert(args.role.clone(), permissions);
    }
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    mirror_role(realm, org, &args.role);
    Ok(result)
}

fn set_role(realm: &Realm, ctx: &GlobalCtx, args: &OrgSetRoleArgs) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    require(org, ctx.resident, perms::ORG_MANAGE)?;
    if org.role_of(args.resident).is_none() {
        return Err(ActionError::not_found("That resident is not a member of this organization."));
    }
    if !org.roles.contains_key(&args.role) {
        return Err(ActionError::not_found(format!(
            "The organization has no role `{}`. Use `org_define_role` first.",
            args.role
        )));
    }
    let demotes_last_manager = org.can(args.resident, perms::ORG_MANAGE)
        && org.managers() == 1
        && !org.role_permissions(&args.role).iter().any(|perm| perm == perms::ORG_MANAGE);
    if !org.is_government() && demotes_last_manager {
        return Err(ActionError::conflict(
            "That member is the only one that can manage this organization.",
        ));
    }
    orgs.set_member(args.org, args.resident, &args.role);
    let org = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    mirror(realm, org, args.resident);
    if args.resident != ctx.resident {
        realm.notify(
            args.resident,
            "society.org_role_changed",
            Some(ctx.resident),
            describe_with(org, json!({ "role": args.role })),
        );
    }
    Ok(json!({ "org": args.org, "resident": args.resident, "role": args.role }))
}

fn grant(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &OrgGrantArgs,
) -> Result<Value, ActionError> {
    let mut orgs = realm.res_mut::<Orgs>();
    let giver = orgs.get(args.org).ok_or_else(|| no_org(args.org))?;
    if !giver.is_government() {
        return Err(ActionError::not_allowed(
            "Only a government organization can grant a permission.",
        ));
    }
    require(giver, ctx.resident, perms::GOV_APPOINT)?;
    if !perms::is_known(&args.permission) || !perms::is_sovereign(&args.permission) {
        return Err(ActionError::invalid_args(
            "Only a `law.*`, `gov.*` or `bank.*` permission can be granted.",
        ));
    }
    if !perms::holds_by_itself(args.org, &args.permission) {
        return Err(ActionError::not_allowed(format!(
            "Organization {} does not hold `{}` by itself.",
            args.org, args.permission
        )));
    }
    if args.to == args.org {
        return Err(ActionError::invalid_args("An organization cannot grant to itself."));
    }
    let taker = orgs.get(args.to).ok_or_else(|| no_org(args.to))?;
    let has = taker.grants.get(&args.permission).is_some_and(|set| set.contains(&args.org));
    if has != args.revoke {
        return Err(ActionError::conflict(if args.revoke {
            "That organization does not have this grant."
        } else {
            "That organization has this grant already."
        }));
    }
    if let Some(taker) = orgs.get_mut(args.to) {
        if args.revoke {
            if let Some(set) = taker.grants.get_mut(&args.permission) {
                set.remove(&args.org);
                if set.is_empty() {
                    taker.grants.remove(&args.permission);
                }
            }
        } else {
            taker.grants.entry(args.permission.clone()).or_default().insert(args.org);
        }
    }
    let taker = orgs.get(args.to).ok_or_else(|| no_org(args.to))?;
    let facts = json!({ "by": args.org, "permission": args.permission, "granted": !args.revoke });
    for member in mirror_permission(realm, taker, &args.permission) {
        realm.notify(
            member,
            "society.org_grant_changed",
            Some(ctx.resident),
            describe_with(taker, facts.clone()),
        );
    }
    let kind = if args.revoke { "society.org_grant_revoked" } else { "society.org_granted" };
    realm.emit(
        EventMeta::public(kind, params::drama(realm, p.drama_org_granted))
            .actor(ctx.resident)
            .at(ctx.pos),
        &describe_with(taker, facts),
    );
    Ok(json!({
        "org": args.org,
        "to": args.to,
        "permission": args.permission,
        "granted": !args.revoke,
    }))
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<OrgResidentArgs>(
        spec("org_invite", "Invite a resident to an organization. You need `org.invite` there."),
        move |realm, ctx, args| invite(realm, &p, ctx, args),
    );
    b.global_verb::<OrgArgs>(
        spec("org_join", "Join an organization that invited you or that is open."),
        move |realm, ctx, args| join(realm, &p, ctx, args),
    );
    b.global_verb::<OrgArgs>(
        spec(
            "org_leave",
            "Leave an organization. The last member gets the treasury and the organization ends.",
        ),
        move |realm, ctx, args| leave(realm, &p, ctx, args),
    );
    b.global_verb::<OrgResidentArgs>(
        spec("org_kick", "Remove a member from an organization. You need `org.manage` there."),
        move |realm, ctx, args| kick(realm, &p, ctx, args),
    );
    b.global_verb::<OrgDefineRoleArgs>(
        spec(
            "org_define_role",
            "Make or replace a role: a name and a set of permissions. You need `org.manage`.",
        ),
        move |realm, ctx, args| define_role(realm, &p, ctx, args),
    );
    b.global_verb::<OrgSetRoleArgs>(
        spec("org_set_role", "Give a member a role of the organization. You need `org.manage`."),
        set_role,
    );
    b.global_verb::<OrgGrantArgs>(
        spec("org_grant", "As a government organization, give a `law.*`, `gov.*` or `bank.*` permission to another organization, or take it back. You need `gov.appoint`."),
        move |realm, ctx, args| grant(realm, &p, ctx, args),
    );
}
