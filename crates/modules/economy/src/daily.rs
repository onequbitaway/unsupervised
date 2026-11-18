//! What a zone does once each game day: the rent, the property tax and the
//! income of the city from its own sites.
//!
//! The zones do not all do this work in the frame of midnight. Each zone
//! starts its day some minutes later, by its number, so that a large world
//! has no frame with the daily work of every zone.

use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::{Dest, Holder, Money, Realm, Resource, SiteId, TICKS_PER_DAY};

use crate::calc;
use crate::ids::Econ;
use crate::ownership;
use crate::site::{self, ATTR_TAX_DAYS, ATTR_TAX_DUE, ATTR_VALUE};
use crate::state::jobs::JobBook;
use crate::tenancy;

/// Notice to an owner whose site could not pay the property tax.
pub const NOTICE_TAX_DUE: &str = "economy.tax_due";

/// The last game day for which the zone did its daily work.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Calendar {
    day: u64,
}

impl Resource for Calendar {
    const NAME: &'static str = "economy.calendar";
}

/// Returns the day of a zone for its daily work. The day of a zone starts
/// `zone number % economy.daily_spread_minutes` minutes after midnight.
fn day_of(econ: &Econ, realm: &Realm) -> u64 {
    let spread = u64::try_from(realm.param(econ.ids.daily_spread_minutes)).unwrap_or(1).max(1);
    let offset = realm.zone_info().map_or(0, |zone| u64::from(zone.id.0) % spread);
    realm.frame().saturating_sub(offset) / TICKS_PER_DAY
}

/// The daily system of a zone. It runs each tick and does work only in the
/// first tick of a new day of the zone.
pub(crate) fn run(econ: &Econ, realm: &Realm) {
    let day = day_of(econ, realm);
    {
        let mut calendar = realm.res_mut::<Calendar>();
        if calendar.day >= day {
            return;
        }
        calendar.day = day;
    }
    tenancy::collect(econ, realm);
    property_tax(econ, realm);
    sweep(realm);
}

/// One site that pays property tax.
struct Taxed {
    site: SiteId,
    owner: Option<Holder>,
    value: i64,
    arrears: i64,
    days: i64,
}

/// Takes the property tax of the day from the till of every private site.
/// A site that cannot pay owes the rest. The city takes a site that owed
/// tax for more days in a row than `economy.property_tax_grace_days`.
fn property_tax(econ: &Econ, realm: &Realm) {
    let rate = realm.param(econ.ids.property_tax_bp);
    let grace = realm.param(econ.ids.property_tax_grace_days);
    let city = site::city_holder(realm);
    let taxed: Vec<Taxed> = realm
        .sites()
        .iter()
        .filter(|s| s.owner.is_some() && !site::is_government(s.owner) && !site::is_public(s))
        .map(|s| Taxed {
            site: s.id,
            owner: s.owner,
            value: s.attr(ATTR_VALUE),
            arrears: s.attr(ATTR_TAX_DUE),
            days: s.attr(ATTR_TAX_DAYS),
        })
        .collect();
    for entry in taxed {
        let due = calc::share(entry.value, rate).saturating_add(entry.arrears);
        if due <= 0 {
            continue;
        }
        let till = Holder::Site(entry.site);
        let available = realm.ledger().money(till).0;
        let wanted = due.min(available);
        let paid = match realm.wire_money(till, Dest::Global, city, Money(wanted), "property_tax") {
            Ok(()) => wanted,
            Err(_) => 0,
        };
        let left = due - paid;
        if left == 0 {
            if let Some(s) = realm.sites_mut().get_mut(entry.site) {
                s.attrs.remove(ATTR_TAX_DUE);
                s.attrs.remove(ATTR_TAX_DAYS);
            }
            continue;
        }
        let days = entry.days + 1;
        if days > grace {
            ownership::seize(realm, entry.site, "property_tax");
            continue;
        }
        if let Some(s) = realm.sites_mut().get_mut(entry.site) {
            s.set_attr(ATTR_TAX_DUE, left);
            s.set_attr(ATTR_TAX_DAYS, days);
        }
        if let Some(Holder::Resident(owner)) = entry.owner {
            realm.notify(
                owner,
                NOTICE_TAX_DUE,
                None,
                json!({ "site": entry.site, "owed": left, "days_left": grace - days }),
            );
        }
    }
}

/// Sends the money in the tills of the sites of a government to its
/// treasury: rent from city homes, and what a site held when the city took
/// it. A site with a job keeps its till, because the till pays the wages.
fn sweep(realm: &Realm) {
    let owned: Vec<(SiteId, Holder)> = {
        let book = realm.res::<JobBook>();
        let sites = realm.sites();
        sites
            .iter()
            .filter(|s| site::is_government(s.owner) && book.at_site(s.id).next().is_none())
            .filter_map(|s| s.owner.map(|owner| (s.id, owner)))
            .collect()
    };
    for (id, treasury) in owned {
        let till = Holder::Site(id);
        let money = realm.ledger().money(till);
        if money.0 > 0 {
            // The amount is the whole balance, so the wire cannot fail.
            let _ = realm.wire_money(till, Dest::Global, treasury, money, "public_revenue");
        }
    }
}
