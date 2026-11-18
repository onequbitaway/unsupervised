//! Changes of the owner of a site, and the end of a business.
//!
//! The economy module is the only writer of `owner` and `tenants`
//! (`docs/design/game-design.md`, section 9).

use serde_json::json;
use unsup_kernel::{Holder, Realm, SiteId};

use crate::board;
use crate::events;
use crate::hiring;
use crate::site::{self, ATTR_PRICE, ATTR_RENT, ATTR_TAX_DAYS, ATTR_TAX_DUE, TAG_OPEN};
use crate::state::prices::PriceBook;
use crate::state::ties::{self, SiteTie, TieChange};

/// Notice to a resident that lost a site to the city.
pub const NOTICE_PROPERTY_LOST: &str = "economy.property_lost";

/// Gives a site to a new owner.
///
/// The offers of the old owner (sale price, rent) end and the jobs at the
/// site close. The permit, the price list, the stock, the till and the
/// leases stay with the site.
pub(crate) fn transfer(realm: &Realm, site_id: SiteId, to: Holder) {
    let (old, kind) = {
        let mut sites = realm.sites_mut();
        let Some(entry) = sites.get_mut(site_id) else {
            return;
        };
        let old = entry.owner.replace(to);
        entry.attrs.remove(ATTR_PRICE);
        entry.attrs.remove(ATTR_RENT);
        (old, entry.kind.clone())
    };
    hiring::close_site_jobs(realm, site_id, "closed");
    if let Some(Holder::Resident(resident)) = old {
        ties::change(realm, resident, TieChange::Disowned { site: site_id });
    }
    if let (Holder::Resident(resident), Some(zone)) = (to, realm.zone_info()) {
        let tie = SiteTie { site: site_id, zone: zone.id, kind };
        ties::change(realm, resident, TieChange::Owned(tie));
    }
    board::touch(realm);
}

/// Ends the permit of a business: the price list and the jobs go, the stock
/// and the till stay. Returns false when the site has no permit. `reason`
/// is `owner`, `converted`, `foreclosure` or `estate`.
pub(crate) fn close_business(realm: &Realm, site_id: SiteId, reason: &str) -> bool {
    let (owner, kind, pos) = {
        let mut sites = realm.sites_mut();
        let Some(entry) = sites.get_mut(site_id) else {
            return false;
        };
        if !site::is_open(entry) {
            return false;
        }
        entry.tags.retain(|tag| tag != TAG_OPEN);
        (entry.owner, entry.kind.clone(), entry.rect.centre())
    };
    realm.res_mut::<PriceBook>().clear(site_id);
    hiring::close_site_jobs(realm, site_id, "closed");
    events::business_closed(realm, site_id, pos, owner, &kind, reason);
    board::touch(realm);
    true
}

/// The city takes a site. `reason` is `loan_default` or `property_tax`.
pub(crate) fn seize(realm: &Realm, site_id: SiteId, reason: &str) {
    let Some((owner, pos)) = realm.sites().get(site_id).map(|s| (s.owner, s.rect.centre())) else {
        return;
    };
    close_business(realm, site_id, "foreclosure");
    transfer(realm, site_id, site::city_holder(realm));
    if let Some(entry) = realm.sites_mut().get_mut(site_id) {
        entry.attrs.remove(ATTR_TAX_DUE);
        entry.attrs.remove(ATTR_TAX_DAYS);
    }
    events::property_foreclosed(realm, site_id, pos, owner, reason);
    if let Some(Holder::Resident(resident)) = owner {
        realm.notify(
            resident,
            NOTICE_PROPERTY_LOST,
            None,
            json!({ "site": site_id, "reason": reason }),
        );
    }
}
