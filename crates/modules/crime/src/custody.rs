//! Taking a resident into custody, and letting it go.
//!
//! An officer (`arrest`) and the city watch use the same steps: stop what
//! the suspect does, set `detained`, move it to the jail of the city, tell
//! the suspect, tell the case registry and emit `crime.arrest_made`.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Dest, EventMeta, Idx, Realm, ResidentId, SiteId};

use crate::enforce;
use crate::ids::Handles;
use crate::jail;
use crate::msg::Report;
use crate::names::{event, notice};
use crate::powers;
use crate::record::{Custody, Record};

/// Takes a resident into custody for some cases. `officer` is `None` for
/// the city watch. Returns the jail that holds the resident, if the city
/// has one.
pub(crate) fn take(
    realm: &Realm,
    h: &Handles,
    target: Idx,
    officer: Option<ResidentId>,
    cases: &[u64],
) -> Option<SiteId> {
    let resident = realm.id_of(target);
    let city = powers::city_of(realm);
    let place = realm.pos(target);
    // The blocking condition comes first, so that the next planned action
    // of the suspect cannot start.
    realm.set_condition(target, h.detained, None);
    realm.clear_condition(target, h.wanted);
    let jail = jail::lock_up(realm, target).map(|jail| jail.site);
    {
        let mut records = realm.col_mut::<Record>();
        let record = &mut records[target];
        record.custody = Some(Custody { since: realm.frame(), officer, site: jail });
        record.warrants.clear();
        record.caught.clear();
    }
    realm.notify(
        resident,
        notice::ARRESTED,
        officer,
        json!({ "officer": officer, "cases": cases, "jail": jail }),
    );
    let mut meta = EventMeta::public(event::ARREST, h.drama(realm, h.p.drama_arrest)).at(place);
    if let Some(officer) = officer {
        meta = meta.actor(officer);
    }
    realm.emit(
        meta,
        &json!({
            "officer": officer,
            "target": resident,
            "case": cases.first(),
            "cases": cases,
            "city": city,
            "watch": officer.is_none(),
        }),
    );
    realm.send(
        Dest::Global,
        &Report::Arrested { target: resident, officer, city, frame: realm.frame() },
    );
    jail
}

/// Ends the custody of a resident: it is no longer under arrest. A
/// resident with jail time stays in jail and serves it. A resident
/// without jail time is free: it gets a notice and viewers see
/// `crime.suspect_released`.
pub(crate) fn end(realm: &Realm, h: &Handles, idx: Idx, reason: &str, by: Option<ResidentId>) {
    let resident = realm.id_of(idx);
    let had = {
        let mut records = realm.col_mut::<Record>();
        let record = &mut records[idx];
        record.trials.clear();
        record.custody.take().is_some()
    };
    let held = realm.has_condition(idx, h.detained);
    realm.clear_condition(idx, h.detained);
    // A warrant that came during the custody counts again.
    enforce::sync_wanted(realm, h, idx);
    if !(had || held) || realm.has_condition(idx, h.jailed) {
        return;
    }
    realm.notify(resident, notice::RELEASED, by, json!({ "reason": reason, "free": true }));
    realm.emit(
        EventMeta::public(event::RELEASED, h.drama(realm, h.p.drama_released))
            .actor(resident)
            .at(realm.pos(idx)),
        &json!({ "target": resident, "reason": reason, "by": by }),
    );
}
