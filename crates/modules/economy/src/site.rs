//! Helpers for sites: where a resident is, who controls a site, and the
//! names of the site tags and attributes that the module uses.
//!
//! The contract with the other modules is in
//! `docs/design/game-design.md`, section 9.

use unsup_kernel::{ActionError, CityId, Holder, Idx, OrgId, Realm, Site};

use crate::calc;
use crate::rules::KindRule;

/// Tag of a public building. It is never sold.
pub const TAG_PUBLIC: &str = "public";
/// Tag of a business with a permit.
pub const TAG_OPEN: &str = "open";
/// Attribute: the assessed value of the site in cents. The life module
/// writes it.
pub const ATTR_VALUE: &str = "value";
/// Attribute: the price in cents at which the owner sells the site.
pub const ATTR_PRICE: &str = "price";
/// Attribute: the rent in cents for one day at which the owner lets the
/// site.
pub const ATTR_RENT: &str = "rent";
/// Attribute: how many tenants the site takes.
pub const ATTR_CAPACITY: &str = "capacity";
/// Attribute: property tax in cents that the site owes.
pub const ATTR_TAX_DUE: &str = "tax_due";
/// Attribute: days in a row that the site owed property tax.
pub const ATTR_TAX_DAYS: &str = "tax_days";

/// Permission to manage the sites of an organization.
pub(crate) const PERM_MANAGE: &str = "site.manage";
/// Permission to hire and fire for an organization.
pub(crate) const PERM_HIRE: &str = "job.hire";

/// Returns the site where a resident is.
pub(crate) fn here(realm: &Realm, idx: Idx) -> Result<Site, ActionError> {
    realm.sites().at(realm.pos(idx)).cloned().ok_or_else(|| {
        ActionError::not_here("You are not at a site. Walk into a building or a lot first.")
    })
}

/// Returns the city of a zone realm.
pub(crate) fn city(realm: &Realm) -> CityId {
    // Verbs, zone systems and zone handlers run only in zones, and every
    // zone has a city.
    realm.zone_info().map_or(CityId(0), |zone| zone.city)
}

/// Returns the treasury of the city of a zone realm.
pub(crate) fn city_holder(realm: &Realm) -> Holder {
    Holder::Org(OrgId::city(city(realm)))
}

/// Returns true when the holder is a government: a city or the nation.
pub(crate) fn is_government(owner: Option<Holder>) -> bool {
    matches!(owner, Some(Holder::Org(org)) if org == OrgId::NATION || org.as_city().is_some())
}

/// Returns true when the site is a public building.
pub(crate) fn is_public(site: &Site) -> bool {
    site.has_tag(TAG_PUBLIC)
}

/// Returns true when the site is a business with a permit.
pub(crate) fn is_open(site: &Site) -> bool {
    site.has_tag(TAG_OPEN)
}

/// Returns true when a resident controls a site: it is the owner, or a
/// member of the organization that owns the site with `site.manage`.
pub(crate) fn controls(realm: &Realm, idx: Idx, site: &Site) -> bool {
    match site.owner {
        Some(Holder::Resident(owner)) => owner == realm.id_of(idx),
        Some(Holder::Org(org)) => realm.can(idx, org, PERM_MANAGE),
        _ => false,
    }
}

/// Returns true when a resident can hire and fire at a site: it controls
/// the site, or it is a member of the owner with `job.hire`.
pub(crate) fn may_hire(realm: &Realm, idx: Idx, site: &Site) -> bool {
    controls(realm, idx, site)
        || matches!(site.owner, Some(Holder::Org(org)) if realm.can(idx, org, PERM_HIRE))
}

/// The error of a verb that only the controller of a site can use.
pub(crate) fn not_controller() -> ActionError {
    ActionError::not_allowed(
        "Only the owner of this site, or a member of its owner with `site.manage`, can do this.",
    )
}

/// Returns the price at which a site is for sale, if it is. A public
/// building is never for sale. A site of a government is for sale at its
/// value unless the owner listed another price.
pub(crate) fn asking_price(site: &Site) -> Option<i64> {
    if is_public(site) {
        return None;
    }
    let listed = site.attr(ATTR_PRICE);
    if listed > 0 {
        return Some(listed);
    }
    let value = site.attr(ATTR_VALUE);
    (is_government(site.owner) && value > 0).then_some(value)
}

/// Returns the rent for one day at which a site takes tenants, if it does.
/// A lettable site of a government takes tenants at `city_rent_bp` of its
/// value unless the owner listed another rent.
pub(crate) fn asking_rent(site: &Site, rule: &KindRule, city_rent_bp: i64) -> Option<i64> {
    if !rule.lettable {
        return None;
    }
    let listed = site.attr(ATTR_RENT);
    if listed > 0 {
        return Some(listed);
    }
    let value = site.attr(ATTR_VALUE);
    (is_government(site.owner) && value > 0 && city_rent_bp > 0)
        .then(|| calc::share(value, city_rent_bp).max(1))
}

/// Returns how many tenants a site takes.
pub(crate) fn units(site: &Site, rule: &KindRule) -> usize {
    let capacity = site.attr(ATTR_CAPACITY);
    if capacity > 0 { usize::try_from(capacity).unwrap_or(usize::MAX) } else { rule.units as usize }
}
