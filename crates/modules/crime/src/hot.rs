//! Hot value: stolen cash and goods that a resident still holds.
//!
//! The ledger does not know where a coin or a unit came from. So the
//! record of the thief counts the stolen cents and units of each good, by
//! case. Value that left the hands of the thief (spent, given, dropped,
//! fenced) is no longer hot: the module lowers the count to what the
//! resident holds each time it looks. It looks in each game hour, and
//! before a search, a sale to a fence and a payment to a court. A lot is
//! cold after `crime.hot_ticks` frames, and when the trial of its case
//! ends.

use unsup_kernel::{Builder, Holder, Idx, ItemId, Realm, Scope, Stage};
use unsup_life::names::TICKS_PER_HOUR;

use crate::ids::Handles;
use crate::record::{HotLot, Record};

/// Returns what a resident holds of one good, or its cash for `None`.
fn held(realm: &Realm, holder: Holder, item: Option<ItemId>) -> i64 {
    let ledger = realm.ledger();
    match item {
        None => ledger.money(holder).cents(),
        Some(item) => ledger.qty(holder, item),
    }
}

/// Brings a list of hot lots up to date: old lots go, and no good is
/// hotter than what the resident holds.
fn refresh(realm: &Realm, h: &Handles, idx: Idx, record: &mut Record) {
    if record.hot.is_empty() {
        return;
    }
    record.cool(realm.frame(), h.frames(realm, h.p.hot_ticks));
    let holder = Holder::Resident(realm.id_of(idx));
    for kind in record.hot_kinds() {
        record.clamp_hot(kind, held(realm, holder, kind));
    }
}

/// Brings the hot lots of a resident up to date.
pub(crate) fn settle(realm: &Realm, h: &Handles, idx: Idx) {
    if realm.col::<Record>()[idx].hot.is_empty() {
        return;
    }
    let mut records = realm.col_mut::<Record>();
    refresh(realm, h, idx, &mut records[idx]);
}

/// Returns the hot lots of a resident as they are now, without a change of
/// state. The observation uses it.
pub(crate) fn view(realm: &Realm, h: &Handles, idx: Idx) -> Vec<HotLot> {
    let records = realm.col::<Record>();
    if records[idx].hot.is_empty() {
        return Vec::new();
    }
    let mut copy = Record::default();
    copy.hot.clone_from(&records[idx].hot);
    drop(records);
    refresh(realm, h, idx, &mut copy);
    copy.hot
}

/// Marks value that a resident just stole as hot.
pub(crate) fn mark(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    case: u64,
    item: Option<ItemId>,
    qty: i64,
) {
    if qty <= 0 {
        return;
    }
    let max = h.count(realm, h.p.hot_lots_max);
    let lot = HotLot { case, item, qty, frame: realm.frame() };
    realm.col_mut::<Record>()[idx].add_hot(lot, max);
}

/// Brings the hot lots of every resident of the zone up to date.
fn settle_all(realm: &Realm, h: &Handles) {
    let marked: Vec<Idx> = {
        let records = realm.col::<Record>();
        realm.indices().filter(|&idx| !records[idx].hot.is_empty()).collect()
    };
    for idx in marked {
        settle(realm, h, idx);
    }
}

/// Registers the hourly look at the hot lots.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.every(TICKS_PER_HOUR, Stage::Post, 0, "crime.hot", move |realm| settle_all(realm, &h));
    }
}
