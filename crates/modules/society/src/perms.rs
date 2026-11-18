//! Permission strings. See `docs/design/game-design.md`, section 4.

use unsup_kernel::OrgId;

/// Lets a member define roles, set roles and remove members.
pub const ORG_MANAGE: &str = "org.manage";
/// Lets a member invite residents.
pub const ORG_INVITE: &str = "org.invite";
/// Lets a member pay from the treasury.
pub const TREASURY_SPEND: &str = "treasury.spend";
/// Lets a member of a government organization grant its powers to another
/// organization, and appoint officials.
pub const GOV_APPOINT: &str = "gov.appoint";

/// Every permission that a role can list.
pub const PERMISSIONS: [&str; 14] = [
    ORG_MANAGE,
    ORG_INVITE,
    TREASURY_SPEND,
    "site.manage",
    "job.hire",
    "law.arrest",
    "law.search",
    "law.judge",
    "law.jail",
    "gov.propose",
    "gov.vote",
    "gov.sign",
    GOV_APPOINT,
    "bank.central",
];

/// The permissions that the first role of a new organization gets: every
/// permission that needs no grant.
pub(crate) const OWNER_PERMISSIONS: [&str; 5] =
    [ORG_MANAGE, ORG_INVITE, TREASURY_SPEND, "site.manage", "job.hire"];

/// Name of the role of the founder of an organization.
pub(crate) const OWNER_ROLE: &str = "owner";
/// Name of the role of a resident that joins an organization.
pub(crate) const MEMBER_ROLE: &str = "member";

/// Returns true when a role can list the permission.
#[must_use]
pub fn is_known(perm: &str) -> bool {
    PERMISSIONS.contains(&perm)
}

/// Returns true for a power of the state: a `law.*`, `gov.*` or `bank.*`
/// permission. Such a permission has an effect only in an organization that
/// holds it: a government organization, or one with a grant.
#[must_use]
pub fn is_sovereign(perm: &str) -> bool {
    perm.starts_with("law.") || perm.starts_with("gov.") || perm.starts_with("bank.")
}

/// Returns true for the organizations with a reserved id: the nation, the
/// central bank and the city governments. They are the government
/// organizations.
#[must_use]
pub fn is_government(org: OrgId) -> bool {
    org == OrgId::NATION || org == OrgId::CENTRAL_BANK || org.as_city().is_some()
}

/// Returns true when a government organization holds a power of the state
/// by itself: the nation holds all of them, a city holds `law.*` and
/// `gov.*`, the central bank holds `bank.*`.
#[must_use]
pub fn holds_by_itself(org: OrgId, perm: &str) -> bool {
    if !is_sovereign(perm) {
        return false;
    }
    if org == OrgId::NATION {
        true
    } else if org == OrgId::CENTRAL_BANK {
        perm.starts_with("bank.")
    } else if org.as_city().is_some() {
        perm.starts_with("law.") || perm.starts_with("gov.")
    } else {
        false
    }
}

/// Returns the name of a permission as the zones see it for one grantor:
/// `law.arrest@100` is `law.arrest` by the power of organization 100. Zone
/// code uses it to check where a power comes from.
#[must_use]
pub fn scoped(perm: &str, grantor: OrgId) -> String {
    format!("{perm}@{}", grantor.0)
}
