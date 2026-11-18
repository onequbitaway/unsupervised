//! How an organization changes when a member goes: by its own will, by a
//! kick or by death.

use unsup_kernel::serde_json::json;
use unsup_kernel::{EventMeta, Holder, OrgId, Realm, ResidentId};

use super::mirror::{describe_with, mirror};
use super::state::Orgs;
use crate::params::{self, Params};
use crate::{pay, perms};

/// Ledger label of the treasury of an organization that ended.
const DISSOLVED: &str = "org.dissolved";

/// Removes an organization that has no member. Its treasury goes to `heir`,
/// or to the national treasury when there is no heir.
fn dissolve(realm: &Realm, p: &Params, orgs: &mut Orgs, id: OrgId, heir: Option<ResidentId>) {
    let Some(org) = orgs.remove(id) else {
        return;
    };
    let treasury = Holder::Org(id);
    let left = realm.ledger().money(treasury).cents();
    let paid = match heir {
        Some(resident) => pay::wire_to_resident(realm, treasury, resident, left, DISSOLVED),
        None => pay::transfer(realm, treasury, Holder::Org(OrgId::NATION), left, DISSOLVED),
    };
    realm.emit(
        EventMeta::public("society.org_dissolved", params::drama(realm, p.drama_org_dissolved)),
        &describe_with(&org, json!({ "treasury": paid, "to": heir })),
    );
}

/// Gives the member with the lowest id a role that can manage, when no
/// member can. Only a death can leave an organization in that state.
fn promote(realm: &Realm, orgs: &mut Orgs, id: OrgId, role_of_gone: &str) {
    let Some(org) = orgs.get_mut(id) else {
        return;
    };
    let manages = |perms: &[String]| perms.iter().any(|p| p == perms::ORG_MANAGE);
    let role = if manages(org.role_permissions(role_of_gone)) {
        role_of_gone.to_string()
    } else if let Some((name, _)) = org.roles.iter().find(|(_, perms)| manages(perms)) {
        name.clone()
    } else {
        let mut perms: Vec<String> =
            perms::OWNER_PERMISSIONS.iter().map(|p| (*p).to_string()).collect();
        perms.sort();
        org.roles.insert(perms::OWNER_ROLE.to_string(), perms);
        perms::OWNER_ROLE.to_string()
    };
    let Some(&heir) = org.members.keys().next() else {
        return;
    };
    orgs.set_member(id, heir, &role);
    if let Some(org) = orgs.get(id) {
        // A role that was defined again can change other members too.
        for (member, _) in org.members() {
            mirror(realm, org, member);
        }
        realm.notify(
            heir,
            "society.org_role_changed",
            None,
            describe_with(org, json!({ "role": role })),
        );
    }
}

/// Keeps an organization in order after `gone` left it with role
/// `role_of_gone`. An organization of residents without members ends, and
/// its treasury goes to `heir`. One without a manager gets a new one. A
/// government organization can be empty.
pub(crate) fn after_departure(
    realm: &Realm,
    p: &Params,
    orgs: &mut Orgs,
    id: OrgId,
    role_of_gone: &str,
    heir: Option<ResidentId>,
) {
    let Some(org) = orgs.get(id) else {
        return;
    };
    if org.is_government() {
        return;
    }
    if org.member_count() == 0 {
        dissolve(realm, p, orgs, id, heir);
    } else if org.managers() == 0 {
        promote(realm, orgs, id, role_of_gone);
    }
}

/// Removes a resident that left the world from every organization and
/// every invitation.
pub(crate) fn on_death(realm: &Realm, p: &Params, orgs: &mut Orgs, dead: ResidentId) {
    let (member_of, invited_by) = orgs.links_of(dead);
    for id in invited_by {
        orgs.remove_invite(id, dead);
    }
    for id in member_of {
        let Some(role) = orgs.remove_member(id, dead) else {
            continue;
        };
        if let Some(org) = orgs.get(id) {
            realm.emit(
                EventMeta::public("society.org_left", params::drama(realm, p.drama_org_left)),
                &describe_with(org, json!({ "resident": dead, "cause": "death" })),
            );
        }
        after_departure(realm, p, orgs, id, &role, None);
    }
}
