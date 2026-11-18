//! Who holds a power of the law, and where.
//!
//! A `law.*` permission is a power of the state. The society module writes
//! it to the memberships of a resident one time by its name and one time
//! for each government that it comes from: `law.arrest@100` is `law.arrest`
//! by the power of organization 100. The crime module reads only those
//! names with a source, so that an officer of one city has no power in
//! another city.

use unsup_kernel::{CityId, Idx, OrgId, Realm, Site};
use unsup_society::permissions::scoped;

/// Returns true when a resident holds a power of the law in a city: from
/// the government of that city, or from the nation.
pub(crate) fn has_power(realm: &Realm, idx: Idx, perm: &str, city: CityId) -> bool {
    realm.has_perm(idx, &scoped(perm, OrgId::city(city))).is_some()
        || realm.has_perm(idx, &scoped(perm, OrgId::NATION)).is_some()
}

/// Returns true when a resident holds one of several powers in a city.
pub(crate) fn has_any_power(realm: &Realm, idx: Idx, perms: &[&str], city: CityId) -> bool {
    perms.iter().any(|perm| has_power(realm, idx, perm, city))
}

/// Returns the cities in which a resident holds a power of the law.
pub(crate) fn cities_with_power(realm: &Realm, idx: Idx, perm: &str) -> Vec<CityId> {
    let nation = realm.has_perm(idx, &scoped(perm, OrgId::NATION)).is_some();
    realm
        .layout()
        .cities()
        .iter()
        .map(|city| city.id)
        .filter(|city| nation || realm.has_perm(idx, &scoped(perm, OrgId::city(*city))).is_some())
        .collect()
}

/// Returns the city of a zone realm.
pub(crate) fn city_of(realm: &Realm) -> CityId {
    // Verbs, zone systems and zone handlers run only in zones, and every
    // zone has a city.
    realm.zone_info().map_or(CityId(0), |zone| zone.city)
}

/// Returns the site that covers the position of a resident, if any.
pub(crate) fn site_here(realm: &Realm, idx: Idx) -> Option<Site> {
    realm.sites().at(realm.pos(idx)).cloned()
}

/// Returns true when a site is "yours": the resident owns it, or it is a
/// member of the organization that owns it with `site.manage`.
pub(crate) fn controls(realm: &Realm, idx: Idx, site: &Site) -> bool {
    match site.owner {
        Some(unsup_kernel::Holder::Resident(owner)) => owner == realm.id_of(idx),
        Some(unsup_kernel::Holder::Org(org)) => realm.can(idx, org, "site.manage"),
        _ => false,
    }
}

/// Returns the holder that loses when a site loses value, and the resident
/// to tell about it. The owner of the site is the victim: a restitution
/// goes to it, also when the site has a new owner at the time of the
/// verdict. A site without an owner is its own victim.
pub(crate) fn site_victim(site: &Site) -> (unsup_kernel::Holder, Option<unsup_kernel::ResidentId>) {
    match site.owner {
        Some(unsup_kernel::Holder::Resident(owner)) => {
            (unsup_kernel::Holder::Resident(owner), Some(owner))
        }
        Some(owner) => (owner, None),
        None => (unsup_kernel::Holder::Site(site.id), None),
    }
}
