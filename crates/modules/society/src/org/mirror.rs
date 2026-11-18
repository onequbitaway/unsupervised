//! The mirror of memberships. The registry in the global realm is the
//! source of truth. The kernel keeps a copy of the memberships of each
//! resident in its zone, so that zone code can call `realm.can`. This file
//! is the only place of the whole game that writes that copy.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Realm, ResidentId, Value};

use super::state::Org;
use crate::text;

/// Writes the membership of one resident in one organization to its zone:
/// the role and the permissions that have an effect, or nothing when the
/// resident is not a member.
pub(crate) fn mirror(realm: &Realm, org: &Org, resident: ResidentId) {
    match org.role_of(resident) {
        Some(role) => {
            let perms = org.effective(role);
            let perms: Vec<&str> = perms.iter().map(String::as_str).collect();
            realm.grant(resident, org.id, role, &perms);
        }
        None => realm.revoke(resident, org.id),
    }
}

/// Writes the memberships of every member with a role again. Returns the
/// members.
pub(crate) fn mirror_role(realm: &Realm, org: &Org, role: &str) -> Vec<ResidentId> {
    let holders = org.holders(role);
    for &resident in &holders {
        mirror(realm, org, resident);
    }
    holders
}

/// Writes the memberships of every member whose role lists a permission
/// again. Returns the members.
pub(crate) fn mirror_permission(realm: &Realm, org: &Org, perm: &str) -> Vec<ResidentId> {
    let touched: Vec<ResidentId> = org
        .members()
        .filter(|(_, role)| org.role_permissions(role).iter().any(|p| p == perm))
        .map(|(resident, _)| resident)
        .collect();
    for &resident in &touched {
        mirror(realm, org, resident);
    }
    touched
}

/// Returns how an organization is named in a notice, an event or an
/// observation: its id, and its name as untrusted text. A government
/// organization has no name that a resident wrote; it has `government`
/// (`nation`, `central_bank` or `city`) and, for a city, `city`.
pub(crate) fn describe(org: &Org) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("org".into(), json!(org.id));
    if org.id == unsup_kernel::OrgId::NATION {
        out.insert("government".into(), json!("nation"));
    } else if org.id == unsup_kernel::OrgId::CENTRAL_BANK {
        out.insert("government".into(), json!("central_bank"));
    } else if let Some(city) = org.id.as_city().filter(|_| org.is_government()) {
        out.insert("government".into(), json!("city"));
        out.insert("city".into(), json!(city));
    } else if let Some(founder) = org.founder {
        out.insert("name".into(), text::untrusted(founder, &org.name));
    }
    out
}

/// Like [`describe`], with more fields.
pub(crate) fn describe_with(org: &Org, more: Value) -> Value {
    let mut out = describe(org);
    if let Value::Object(more) = more {
        out.extend(more);
    }
    Value::Object(out)
}
