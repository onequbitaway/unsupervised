//! Leases after they start: the daily rent, missed payments, the end of a
//! lease and the eviction.
//!
//! Rent is cash. The tenant can be in another zone when the rent is due,
//! so the zone of the home asks with a message that follows the tenant
//! ([`RentDue`]). The zone of the tenant pays, by a transfer or by a wire,
//! and answers ([`RentAnswer`]).

use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::{Dest, Holder, Idx, Message, Money, Realm, ResidentId, SiteId, ZoneId};

use crate::board;
use crate::events;
use crate::hiring::site_pos;
use crate::ids::Econ;
use crate::state::leases::Leases;
use crate::state::ties::{self, TieChange};

/// Notice to a tenant that was evicted.
pub const NOTICE_EVICTED: &str = "economy.evicted";
/// Notice to a tenant that could not pay the rent.
pub const NOTICE_RENT_MISSED: &str = "economy.rent_missed";
/// Notice to a tenant whose lease the owner ended.
pub const NOTICE_LEASE_ENDED: &str = "economy.lease_ended";

/// The zone of a home asks a tenant for the rent.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RentDue {
    site: SiteId,
    zone: ZoneId,
    amount: i64,
}

impl Message for RentDue {
    const KIND: &'static str = "economy.rent_due";
}

/// The zone of a tenant says what the tenant paid.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct RentAnswer {
    site: SiteId,
    tenant: ResidentId,
    paid: i64,
}

impl Message for RentAnswer {
    const KIND: &'static str = "economy.rent_answer";
}

/// Ends a lease: the tenant leaves the site and its ties. Returns false
/// when the resident was not a tenant.
pub(crate) fn end(realm: &Realm, site: SiteId, tenant: ResidentId) -> bool {
    if realm.res_mut::<Leases>().remove(site, tenant).is_none() {
        return false;
    }
    if let Some(entry) = realm.sites_mut().get_mut(site) {
        entry.remove_tenant(tenant);
    }
    ties::change(realm, tenant, TieChange::LeaseEnded { site });
    board::touch(realm);
    true
}

/// Evicts a tenant that did not pay.
fn evict(realm: &Realm, site: SiteId, tenant: ResidentId) {
    if end(realm, site, tenant) {
        events::tenant_evicted(realm, site, site_pos(realm, site), tenant);
        realm.notify(tenant, NOTICE_EVICTED, None, json!({ "site": site }));
    }
}

/// Ends every lease of a resident that left the world.
pub(crate) fn end_everywhere(realm: &Realm, tenant: ResidentId) {
    let leases = realm.res::<Leases>().all();
    for (site, _) in leases.into_iter().filter(|(_, who)| *who == tenant) {
        if end(realm, site, tenant) {
            events::lease_ended(realm, site, site_pos(realm, site), tenant, "gone");
        }
    }
}

/// The daily rent run of a zone: every lease owes one more day, and every
/// tenant is asked for what it owes. A tenant that missed more days in a
/// row than `economy.rent_grace_days` is evicted.
pub(crate) fn collect(econ: &Econ, realm: &Realm) {
    let Some(zone) = realm.zone_info().map(|zone| zone.id) else {
        return;
    };
    let grace = realm.param(econ.ids.rent_grace_days);
    let leases = realm.res::<Leases>().all();
    for (site, tenant) in leases {
        let (overdue, owed) = {
            let mut leases = realm.res_mut::<Leases>();
            let Some(lease) = leases.get_mut(site, tenant) else {
                continue;
            };
            if lease.asked {
                // No answer since the last run: the tenant cannot be
                // reached. That is a missed day.
                lease.overdue = lease.overdue.saturating_add(1);
            }
            lease.owed = lease.owed.saturating_add(lease.rent);
            lease.asked = true;
            (lease.overdue, lease.owed)
        };
        if i64::from(overdue) > grace {
            evict(realm, site, tenant);
        } else {
            realm.send(Dest::Resident(tenant), &RentDue { site, zone, amount: owed });
        }
    }
}

/// Handles a [`RentDue`] in the zone of the tenant: pays everything that is
/// owed, or nothing.
pub(crate) fn on_rent_due(econ: &Econ, realm: &Realm, idx: Idx, due: &RentDue) {
    let tenant = realm.id_of(idx);
    let me = Holder::Resident(tenant);
    let till = Holder::Site(due.site);
    let local = realm.zone_info().is_some_and(|zone| zone.id == due.zone);
    let amount = Money(due.amount);
    let cash = realm.ledger().money(me);
    let paid = cash >= amount
        && if local {
            realm.ledger_mut().transfer_money(me, till, amount, "rent").is_ok()
        } else {
            realm.wire_money(me, Dest::Zone(due.zone), till, amount, "rent").is_ok()
        };
    if !paid {
        realm.notify(
            tenant,
            NOTICE_RENT_MISSED,
            None,
            json!({ "site": due.site, "owed": due.amount, "cash": cash }),
        );
    }
    let answer = RentAnswer { site: due.site, tenant, paid: if paid { due.amount } else { 0 } };
    if local {
        on_rent_answer(econ, realm, &answer);
    } else {
        realm.send(Dest::Zone(due.zone), &answer);
    }
}

/// Handles a [`RentAnswer`] in the zone of the home.
pub(crate) fn on_rent_answer(econ: &Econ, realm: &Realm, answer: &RentAnswer) {
    let grace = realm.param(econ.ids.rent_grace_days);
    let evict_now = {
        let mut leases = realm.res_mut::<Leases>();
        let Some(lease) = leases.get_mut(answer.site, answer.tenant) else {
            // The lease ended while the answer was on the way.
            return;
        };
        lease.asked = false;
        if answer.paid > 0 {
            lease.owed = (lease.owed - answer.paid).max(0);
            lease.overdue = 0;
            false
        } else {
            lease.overdue = lease.overdue.saturating_add(1);
            i64::from(lease.overdue) > grace
        }
    };
    if evict_now {
        evict(realm, answer.site, answer.tenant);
    }
}
