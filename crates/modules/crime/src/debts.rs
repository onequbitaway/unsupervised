//! The debts of a convict: fines and restitutions.
//!
//! A fine goes to the treasury of the city of the case. A restitution
//! goes to the victim. A restitution to a site that cannot be reached goes
//! to the city treasury. A debt that is not paid when its time is over
//! becomes jail time.

use unsup_kernel::serde_json::json;
use unsup_kernel::{CityId, Dest, EventMeta, Holder, Idx, Money, OrgId, Realm};

use crate::calc;
use crate::hot;
use crate::ids::Handles;
use crate::jail;
use crate::names::{event, label, notice};
use crate::record::{Debt, Record};

/// Where the money of a debt goes.
enum Route {
    /// To a holder in the ledger of this zone.
    Here(Holder),
    /// By wire to a holder in another realm.
    Wire(Dest, Holder),
}

/// Returns the treasury of the city that gets a restitution whose site
/// cannot be reached: the city of the zone that the debt names, when the
/// world has that zone, else the city where the convict is now.
fn treasury_for(realm: &Realm, debt: &Debt) -> Holder {
    let named = debt.zone.and_then(|zone| realm.layout().zone(zone)).map(|zone| zone.city);
    let city = named.or_else(|| realm.zone_info().map(|zone| zone.city)).unwrap_or(CityId(0));
    Holder::Org(OrgId::city(city))
}

/// Finds the way of the money of a debt.
///
/// A site is reached in two ways: it is a site of this zone, or the debt
/// names its zone and the world has that zone. A zone cannot see the sites
/// of another zone, so a wire to a zone of the world goes out as it is.
///
/// Every other site cannot be reached: the debt names no zone and the site
/// is not here, the debt names a zone that the world does not have, or the
/// site of this zone is gone. That money goes to the city treasury, so
/// that the convict can always pay what it owes.
fn route(realm: &Realm, debt: &Debt) -> Route {
    let here = realm.zone_info().map(|zone| zone.id);
    let treasury = || Route::Wire(Dest::Global, treasury_for(realm, debt));
    match debt.to {
        Holder::Resident(to) if realm.idx_of(to).is_some() => Route::Here(debt.to),
        Holder::Resident(to) => Route::Wire(Dest::Resident(to), debt.to),
        Holder::Site(site) => match debt.zone {
            Some(zone) if Some(zone) != here => {
                if realm.layout().zone(zone).is_some() {
                    Route::Wire(Dest::Zone(zone), debt.to)
                } else {
                    treasury()
                }
            }
            _ if realm.sites().get(site).is_some() => Route::Here(debt.to),
            _ => treasury(),
        },
        _ => Route::Wire(Dest::Global, debt.to),
    }
}

/// Sends money of a resident to the holder of a debt, in whatever realm
/// that holder is. Returns true when the money left the resident.
fn send(realm: &Realm, idx: Idx, debt: &Debt, amount: i64) -> bool {
    let from = Holder::Resident(realm.id_of(idx));
    let reason = if debt.fine { label::FINE } else { label::RESTITUTION };
    let money = Money(amount);
    let moved = match route(realm, debt) {
        Route::Here(to) => realm.ledger_mut().transfer_money(from, to, money, reason).is_ok(),
        Route::Wire(dest, to) => realm.wire_money(from, dest, to, money, reason).is_ok(),
    };
    if moved
        && !debt.fine
        && let Holder::Resident(to) = debt.to
    {
        realm.notify(to, notice::RESTITUTION, None, json!({ "case": debt.case, "amount": amount }));
    }
    moved
}

/// Pays the debts of a resident from its cash, the oldest first, with at
/// most `budget` cents. With `case`, only the debts of that case. Returns
/// the cents paid.
pub(crate) fn pay(realm: &Realm, h: &Handles, idx: Idx, budget: i64, case: Option<u64>) -> i64 {
    let cash = realm.ledger().money(Holder::Resident(realm.id_of(idx))).cents();
    let mut left = budget.min(cash).max(0);
    let mut paid = 0;
    let debts = realm.col::<Record>()[idx].debts.clone();
    let mut rest = Vec::with_capacity(debts.len());
    for mut debt in debts {
        let part =
            if case.is_none_or(|only| only == debt.case) { debt.amount.min(left) } else { 0 };
        if part > 0 && send(realm, idx, &debt, part) {
            debt.amount -= part;
            left -= part;
            paid += part;
        }
        if debt.amount > 0 {
            rest.push(debt);
        }
    }
    realm.col_mut::<Record>()[idx].debts = rest;
    // Cash that went to the court is no longer hot.
    hot::settle(realm, h, idx);
    paid
}

/// The time to pay the debts of a case is over. The resident pays what it
/// can. What is still owed becomes jail time: one tick for each
/// `crime.fine_cents_per_jail_tick` cents, rounded up, at most
/// `crime.fine_jail_max_ticks`.
pub(crate) fn default_on(realm: &Realm, h: &Handles, idx: Idx, case: u64) {
    pay(realm, h, idx, i64::MAX, Some(case));
    let owed = {
        let mut records = realm.col_mut::<Record>();
        let record = &mut records[idx];
        let owed = record
            .debts
            .iter()
            .filter(|debt| debt.case == case)
            .fold(0i64, |sum, debt| sum.saturating_add(debt.amount));
        record.debts.retain(|debt| debt.case != case);
        owed
    };
    if owed <= 0 {
        return;
    }
    let ticks = calc::div_up(owed, realm.param(h.p.fine_per_jail_tick).max(1))
        .min(realm.param(h.p.fine_jail_max).max(0));
    let resident = realm.id_of(idx);
    if ticks > 0 {
        jail::add_time(realm, h, idx, ticks);
    }
    realm.notify(
        resident,
        notice::FINE_DEFAULTED,
        None,
        json!({ "case": case, "owed": owed, "jail_ticks": ticks }),
    );
    realm.emit(
        EventMeta::public(event::FINE_DEFAULTED, h.drama(realm, h.p.drama_fine_defaulted))
            .actor(resident)
            .at(realm.pos(idx)),
        &json!({ "resident": resident, "case": case, "owed": owed, "jail_ticks": ticks }),
    );
}
