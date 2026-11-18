//! Government organizations and their offices.
//!
//! The society module makes the records of the organizations with reserved
//! ids at world generation: the nation, the central bank and one government
//! for each city. They have no roles and no members then.
//!
//! Other modules fill them with the functions of this file. The politics
//! module, for example, defines the role `mayor` in a city government and
//! sets its holder after each election. Call the functions in the global
//! realm, for example from a system, a message handler or a global verb.

use std::fmt;

use unsup_kernel::serde_json::json;
use unsup_kernel::{OrgId, Realm, ResidentId, Scope, WorldGen};

use super::mirror::{describe_with, mirror, mirror_role};
use super::state::{Org, Orgs};
use crate::{params, perms, text};

/// The parameter with the greatest length of the name of a role, in bytes.
const ROLE_NAME_MAX: &str = "society.role_name_max";

/// Returns the greatest length of the name of a role. The functions of this
/// file have no builder, so they find the parameter by its name.
fn role_name_max(realm: &Realm) -> usize {
    realm.defs().param_id(ROLE_NAME_MAX).map_or(0, |param| params::count(realm, param))
}

/// Why a change of an office was refused. Nothing changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OfficeError {
    /// The call was not made in the global realm.
    NotGlobal,
    /// The organization is not the nation, the central bank or the
    /// government of a city of this world.
    NotGovernment(OrgId),
    /// The role name is not made of lower-case letters, digits and `_`, or
    /// it is longer than the parameter `society.role_name_max`.
    BadRoleName(String),
    /// No permission has this name.
    UnknownPermission(String),
    /// The organization has no role with this name.
    UnknownRole(String),
    /// The resident is not in the world.
    UnknownResident(ResidentId),
}

impl fmt::Display for OfficeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OfficeError::NotGlobal => f.write_str("offices change only in the global realm"),
            OfficeError::NotGovernment(org) => {
                write!(f, "organization {org} is not a government organization of this world")
            }
            OfficeError::BadRoleName(role) => write!(f, "`{role}` is not a valid role name"),
            OfficeError::UnknownPermission(perm) => write!(f, "no permission is named `{perm}`"),
            OfficeError::UnknownRole(role) => write!(f, "the organization has no role `{role}`"),
            OfficeError::UnknownResident(id) => write!(f, "resident {id} is not in the world"),
        }
    }
}

impl std::error::Error for OfficeError {}

/// Makes the records of the government organizations. It runs once, in the
/// global realm, when the world is made.
pub(crate) fn worldgen(realm: &Realm, context: &WorldGen<'_>) {
    let mut orgs = realm.res_mut::<Orgs>();
    let cities = context.layout.cities().iter().map(|city| OrgId::city(city.id));
    for id in [OrgId::NATION, OrgId::CENTRAL_BANK].into_iter().chain(cities) {
        if orgs.get(id).is_none() {
            orgs.insert(Org { id, ..Org::default() });
        }
    }
}

/// Checks that an organization is a government organization of this world.
fn government(orgs: &Orgs, org: OrgId) -> Result<(), OfficeError> {
    match orgs.get(org) {
        Some(found) if found.is_government() => Ok(()),
        _ => Err(OfficeError::NotGovernment(org)),
    }
}

/// Makes or replaces a role of a government organization. The members that
/// have the role get the new permissions in their zone in the next frame.
///
/// `perms` are permission strings of `docs/design/game-design.md`, section
/// 4. A `law.*`, `gov.*` or `bank.*` permission has an effect only when the
/// organization holds it: the nation holds all of them, a city holds
/// `law.*` and `gov.*`, the central bank holds `bank.*`.
pub fn define_role(
    realm: &Realm,
    org: OrgId,
    role: &str,
    perms: &[&str],
) -> Result<(), OfficeError> {
    if realm.scope() != Scope::Global {
        return Err(OfficeError::NotGlobal);
    }
    let mut orgs = realm.res_mut::<Orgs>();
    government(&orgs, org)?;
    if !text::is_ident(role, role_name_max(realm)) {
        return Err(OfficeError::BadRoleName(role.to_string()));
    }
    if let Some(unknown) = perms.iter().find(|perm| !perms::is_known(perm)) {
        return Err(OfficeError::UnknownPermission((*unknown).to_string()));
    }
    let mut list: Vec<String> = perms.iter().map(|perm| (*perm).to_string()).collect();
    list.sort();
    list.dedup();
    if let Some(found) = orgs.get_mut(org) {
        found.roles.insert(role.to_string(), list);
    }
    if let Some(found) = orgs.get(org) {
        mirror_role(realm, found, role);
    }
    Ok(())
}

/// Sets who holds a role of a government organization. After the call the
/// residents in `holders` are the members with that role, and no one else
/// is. A resident that had the role and is not in the list leaves the
/// organization. An empty list leaves the office vacant.
///
/// A resident has one role in an organization, so a holder that had another
/// role there loses that one. Each resident whose role changed gets the
/// notice `society.org_role_changed` or `society.org_removed`.
pub fn set_holders(
    realm: &Realm,
    org: OrgId,
    role: &str,
    holders: &[ResidentId],
) -> Result<(), OfficeError> {
    if realm.scope() != Scope::Global {
        return Err(OfficeError::NotGlobal);
    }
    let mut orgs = realm.res_mut::<Orgs>();
    government(&orgs, org)?;
    let current = match orgs.get(org) {
        Some(found) if found.roles.contains_key(role) => found.holders(role),
        _ => return Err(OfficeError::UnknownRole(role.to_string())),
    };
    if let Some(&gone) = holders.iter().find(|&&holder| !realm.directory().is_alive(holder)) {
        return Err(OfficeError::UnknownResident(gone));
    }
    for resident in current.iter().copied().filter(|resident| !holders.contains(resident)) {
        orgs.remove_member(org, resident);
        realm.revoke(resident, org);
        if let Some(found) = orgs.get(org) {
            realm.notify(resident, "society.org_removed", None, describe_with(found, json!({})));
        }
    }
    for &resident in holders {
        if current.contains(&resident) {
            continue;
        }
        orgs.set_member(org, resident, role);
        if let Some(found) = orgs.get(org) {
            mirror(realm, found, resident);
            realm.notify(
                resident,
                "society.org_role_changed",
                None,
                describe_with(found, json!({ "role": role })),
            );
        }
    }
    Ok(())
}

/// Returns the residents that hold a role of an organization, ordered by
/// id. It is empty when the organization or the role does not exist. Call
/// it in the global realm.
#[must_use]
pub fn holders(realm: &Realm, org: OrgId, role: &str) -> Vec<ResidentId> {
    if realm.scope() != Scope::Global {
        return Vec::new();
    }
    realm.res::<Orgs>().get(org).map(|found| found.holders(role)).unwrap_or_default()
}
