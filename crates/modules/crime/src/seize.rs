//! Seizure: hot value and contraband go from a resident to the police.
//!
//! What a search takes goes to the police station of the city that is
//! nearest: cash into its till, goods into its stock. A city without a
//! police station sends both to its treasury.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Dest, Holder, Idx, ItemId, Money, OrgId, Realm};

use crate::hot;
use crate::ids::Handles;
use crate::loot;
use crate::names::{label, tag};
use crate::places::LawSites;
use crate::powers;
use crate::record::Record;

/// Where seized value goes.
#[derive(Clone, Copy, Debug)]
struct Store {
    dest: Option<Dest>,
    holder: Holder,
}

/// Returns the store for value that is seized at the place of a resident.
fn store(realm: &Realm, idx: Idx) -> Store {
    let city = powers::city_of(realm);
    let here = realm.zone_info().map(|zone| zone.id);
    match realm.res::<LawSites>().station(city, realm.pos(idx)) {
        Some(station) if Some(station.zone) == here => {
            Store { dest: None, holder: Holder::Site(station.site) }
        }
        Some(station) => {
            Store { dest: Some(Dest::Zone(station.zone)), holder: Holder::Site(station.site) }
        }
        None => Store { dest: Some(Dest::Global), holder: Holder::Org(OrgId::city(city)) },
    }
}

/// Moves cash (`None`) or units of a good from a resident to the store.
/// Returns true when the value moved.
fn confiscate(realm: &Realm, from: Holder, to: Store, item: Option<ItemId>, qty: i64) -> bool {
    if qty <= 0 {
        return false;
    }
    match (to.dest, item) {
        (None, None) => {
            realm.ledger_mut().transfer_money(from, to.holder, Money(qty), label::SEIZED).is_ok()
        }
        (None, Some(item)) => {
            realm.ledger_mut().transfer_item(from, to.holder, item, qty, label::SEIZED).is_ok()
        }
        (Some(dest), None) => {
            realm.wire_money(from, dest, to.holder, Money(qty), label::SEIZED).is_ok()
        }
        (Some(dest), Some(item)) => {
            realm.wire_item(from, dest, to.holder, item, qty, label::SEIZED).is_ok()
        }
    }
}

/// What a seizure took.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Seized {
    /// Hot cents.
    pub(crate) money: i64,
    /// Hot units of each good.
    pub(crate) goods: Vec<(ItemId, i64)>,
    /// The cases of the hot value, each one time.
    pub(crate) cases: Vec<u64>,
    /// Units of each contraband good.
    pub(crate) contraband: Vec<(ItemId, i64)>,
    /// Normal price of the contraband in cents.
    pub(crate) contraband_value: i64,
}

impl Seized {
    /// Returns true when the seizure took nothing.
    pub(crate) fn is_empty(&self) -> bool {
        self.money == 0 && self.goods.is_empty() && self.contraband.is_empty()
    }

    /// Returns the seizure for a result or a notice.
    pub(crate) fn json(&self, realm: &Realm) -> Value {
        let defs = realm.defs();
        let names = |list: &[(ItemId, i64)]| -> Value {
            let map: Map<String, Value> = list
                .iter()
                .map(|&(item, qty)| (defs.item_name(item).to_string(), json!(qty)))
                .collect();
            Value::Object(map)
        };
        json!({
            "money": self.money,
            "goods": names(&self.goods),
            "contraband": names(&self.contraband),
            "cases": self.cases,
        })
    }
}

/// Takes the contraband that a resident carries, when the parameter
/// `crime.contraband_illegal` is 1.
fn contraband(realm: &Realm, h: &Handles, idx: Idx, to: Store, out: &mut Seized) {
    if realm.param(h.p.contraband_illegal) == 0 {
        return;
    }
    let from = Holder::Resident(realm.id_of(idx));
    let carried = realm.ledger().items_of(from);
    for (item, qty) in carried {
        if qty > 0
            && loot::has_tag(realm, item, tag::CONTRABAND)
            && confiscate(realm, from, to, Some(item), qty)
        {
            out.contraband.push((item, qty));
            out.contraband_value =
                out.contraband_value.saturating_add(loot::value_of(realm, item, qty));
        }
    }
}

/// Takes the hot value that a resident still holds.
fn hot_value(realm: &Realm, h: &Handles, idx: Idx, to: Store, out: &mut Seized) {
    hot::settle(realm, h, idx);
    let from = Holder::Resident(realm.id_of(idx));
    let kinds = realm.col::<Record>()[idx].hot_kinds();
    for kind in kinds {
        let qty = realm.col::<Record>()[idx].hot_of(kind);
        if !confiscate(realm, from, to, kind, qty) {
            continue;
        }
        let taken = realm.col_mut::<Record>()[idx].take_hot(kind, qty);
        for (case, _) in taken {
            if !out.cases.contains(&case) {
                out.cases.push(case);
            }
        }
        match kind {
            None => out.money = qty,
            Some(item) => out.goods.push((item, qty)),
        }
    }
}

/// Takes the hot value and the contraband of a resident.
pub(crate) fn search(realm: &Realm, h: &Handles, idx: Idx) -> Seized {
    let to = store(realm, idx);
    let mut out = Seized::default();
    hot_value(realm, h, idx, to, &mut out);
    contraband(realm, h, idx, to, &mut out);
    out
}

/// Takes only the contraband of a resident. The city watch uses it.
pub(crate) fn contraband_only(realm: &Realm, h: &Handles, idx: Idx) -> Seized {
    let to = store(realm, idx);
    let mut out = Seized::default();
    contraband(realm, h, idx, to, &mut out);
    out
}

/// Returns true when a resident carries contraband.
pub(crate) fn carries_contraband(realm: &Realm, idx: Idx, contraband: &[ItemId]) -> bool {
    let holder = Holder::Resident(realm.id_of(idx));
    let ledger = realm.ledger();
    contraband.iter().any(|&item| ledger.qty(holder, item) > 0)
}
