//! The city watch: the baseline enforcement of the engine.
//!
//! A world with no police still punishes crime. The watch does three
//! things, all weak by default and all tuned by policy parameters:
//!
//! - It sees each crime with the chance `crime.watch_detect_bp` (see
//!   `commit`). A crime that it sees gets a warrant.
//! - Once in each game hour it looks into the public buildings. A wanted
//!   resident there is arrested with the chance `crime.watch_arrest_bp`.
//! - In the same round, a resident there that carries contraband loses it
//!   with the chance `crime.watch_contraband_bp`, and gets a case.
//!
//! The registry charges a suspect of the watch at once, and the trial ends
//! by the evidence threshold when no judge and no jury decides.

use unsup_kernel::{Builder, Idx, ItemId, Realm, Scope, Stage};
use unsup_life::names::TICKS_PER_HOUR;

use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::custody;
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::names::{notice, tag};
use crate::record::Record;
use crate::seize;
use crate::verbs::in_custody;

/// Returns true when a resident stands in a public building.
fn in_public(realm: &Realm, idx: Idx) -> bool {
    realm.sites().at(realm.pos(idx)).is_some_and(|site| site.has_tag(tag::PUBLIC))
}

/// The watch arrests a wanted resident.
fn arrest(realm: &Realm, h: &Handles, idx: Idx) {
    let cases: Vec<u64> = realm.col::<Record>()[idx].warrants.iter().map(|w| w.case).collect();
    custody::take(realm, h, idx, None, &cases);
}

/// The watch takes the contraband of a resident and opens a case.
fn seize_contraband(realm: &Realm, h: &Handles, idx: Idx) {
    let found = seize::contraband_only(realm, h, idx);
    if found.contraband.is_empty() {
        return;
    }
    let case = commit::new_case(realm);
    let pos = realm.pos(idx);
    let deed = Deed {
        kind: CrimeKind::Smuggling,
        perp: idx,
        pos,
        site: realm.sites().at(pos).map(|site| site.id),
        victim: None,
        target: None,
        owner: None,
        value: found.contraband_value,
        failed: false,
        force: false,
    };
    commit::file(realm, h, case, &deed, Finder::Watch);
    let mut data = found.json(realm);
    if let Some(map) = data.as_object_mut() {
        map.insert("smuggling_case".into(), case.into());
        map.insert("found".into(), true.into());
    }
    realm.notify(realm.id_of(idx), notice::SEARCHED, None, data);
}

/// One round of the watch through the public buildings of the zone.
fn round(realm: &Realm, h: &Handles) {
    let arrest_chance = calc::plain_chance(realm.param(h.p.watch_arrest));
    let contraband_chance = if realm.param(h.p.contraband_illegal) == 0 {
        0
    } else {
        calc::plain_chance(realm.param(h.p.watch_contraband))
    };
    if arrest_chance == 0 && contraband_chance == 0 {
        return;
    }
    let contraband: Vec<ItemId> = if contraband_chance == 0 {
        Vec::new()
    } else {
        realm.defs().items_with_tag(tag::CONTRABAND).map(|(item, _)| item).collect()
    };
    // The order of the table is the same in every run with the same
    // inputs, so the rolls are too.
    for idx in realm.indices() {
        let wanted = arrest_chance > 0 && realm.has_condition(idx, h.wanted);
        let carries = !contraband.is_empty() && seize::carries_contraband(realm, idx, &contraband);
        if !(wanted || carries) || in_custody(realm, h, idx) || !in_public(realm, idx) {
            continue;
        }
        if carries && calc::roll(realm, contraband_chance) {
            seize_contraband(realm, h, idx);
        }
        if wanted && calc::roll(realm, arrest_chance) {
            arrest(realm, h, idx);
        }
    }
}

/// Registers the hourly round of the watch.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.every(TICKS_PER_HOUR, Stage::Update, 10, "crime.watch", move |realm| round(realm, &h));
    }
}
