//! Loot: which goods a thief takes, and the move of stolen value.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Holder, Idx, ItemId, Money, Realm};

use crate::hot;
use crate::ids::Handles;
use crate::names::{key, label};

/// Returns the normal price of one unit of a good in cents: its attribute
/// `base_price`, or 0 when the good has none.
pub(crate) fn unit_value(realm: &Realm, item: ItemId) -> i64 {
    realm.defs().item(item).map_or(0, |def| def.attr_value(key::BASE_PRICE).max(0))
}

/// Returns the normal price of some units of a good in cents.
pub(crate) fn value_of(realm: &Realm, item: ItemId, qty: i64) -> i64 {
    unit_value(realm, item).saturating_mul(qty.max(0))
}

/// Returns true when a good has a tag.
pub(crate) fn has_tag(realm: &Realm, item: ItemId, tag: &str) -> bool {
    realm.defs().item(item).is_some_and(|def| def.has_tag(tag))
}

/// Chooses at most `max_units` units of the goods of a holder: the good
/// with the highest normal price first, then the lower item id.
pub(crate) fn pick(realm: &Realm, from: Holder, max_units: i64) -> Vec<(ItemId, i64)> {
    let mut goods = realm.ledger().items_of(from);
    goods.retain(|&(_, qty)| qty > 0);
    goods.sort_by_key(|&(item, _)| (std::cmp::Reverse(unit_value(realm, item)), item));
    let mut left = max_units.max(0);
    let mut picked = Vec::new();
    for (item, qty) in goods {
        if left == 0 {
            break;
        }
        let part = qty.min(left);
        left -= part;
        picked.push((item, part));
    }
    picked
}

/// What a thief took.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Haul {
    /// Cents of cash.
    pub(crate) cash: i64,
    /// Units of each good.
    pub(crate) goods: Vec<(ItemId, i64)>,
    /// Cash plus the normal price of the goods, in cents.
    pub(crate) value: i64,
}

impl Haul {
    /// Returns the goods by name, for a result or a notice.
    pub(crate) fn goods_json(&self, realm: &Realm) -> Value {
        let defs = realm.defs();
        let map: Map<String, Value> = self
            .goods
            .iter()
            .map(|&(item, qty)| (defs.item_name(item).to_string(), json!(qty)))
            .collect();
        Value::Object(map)
    }
}

/// Moves cash and goods from a holder of this zone to a thief and marks
/// them as hot. It moves at most what the holder has. Returns what moved.
pub(crate) fn take(
    realm: &Realm,
    h: &Handles,
    case: u64,
    from: Holder,
    thief: Idx,
    cash: i64,
    goods: &[(ItemId, i64)],
) -> Haul {
    let to = Holder::Resident(realm.id_of(thief));
    let mut haul = Haul::default();
    {
        let mut ledger = realm.ledger_mut();
        let cash = cash.min(ledger.money(from).cents());
        if cash > 0 && ledger.transfer_money(from, to, Money(cash), label::LOOT).is_ok() {
            haul.cash = cash;
        }
        for &(item, qty) in goods {
            let qty = qty.min(ledger.qty(from, item));
            if qty > 0 && ledger.transfer_item(from, to, item, qty, label::LOOT).is_ok() {
                haul.goods.push((item, qty));
            }
        }
    }
    hot::mark(realm, h, thief, case, None, haul.cash);
    haul.value = haul.cash;
    for &(item, qty) in &haul.goods {
        hot::mark(realm, h, thief, case, Some(item), qty);
        haul.value = haul.value.saturating_add(value_of(realm, item, qty));
    }
    haul
}
