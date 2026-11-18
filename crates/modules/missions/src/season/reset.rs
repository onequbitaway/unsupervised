//! The soft reset at the end of a season, in the zones.
//!
//! The global realm sends one order to every zone. A zone applies it to
//! each of its residents one time: reputations decay, and cash above a
//! floor is cut by a share through the sink `season.reset`.
//!
//! A resident carries the number of the last season whose reset it got.
//! Thus a resident that changes zone while one zone is late (time dilation)
//! gets the reset exactly once.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    AttrId, Builder, Component, EventMeta, Holder, Idx, Message, Money, Realm, Resource, Scope,
    Value,
};

use crate::params::{self, Params};

/// The order of the global realm to apply the soft reset of a season.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ResetOrder {
    /// Number of the season that ended.
    pub(crate) season: u32,
    /// Part of `rep.standing` that a resident keeps, in basis points.
    pub(crate) standing_keep_bp: i64,
    /// Part of `rep.notoriety` that a resident keeps, in basis points.
    pub(crate) notoriety_keep_bp: i64,
    /// Cash that the reset does not touch, in cents.
    pub(crate) wealth_floor: i64,
    /// Part of the cash above the floor that the reset destroys, in basis
    /// points.
    pub(crate) wealth_cut_bp: i64,
}

impl Message for ResetOrder {
    const KIND: &'static str = "missions.season_reset";
}

impl ResetOrder {
    /// Returns the numbers of the reset as JSON, for the event
    /// `missions.season_ended`.
    pub(crate) fn to_value(self) -> Value {
        json!({
            "standing_keep_bp": self.standing_keep_bp,
            "notoriety_keep_bp": self.notoriety_keep_bp,
            "wealth_floor": self.wealth_floor,
            "wealth_cut_bp": self.wealth_cut_bp,
        })
    }
}

/// The last reset order that a zone got. A resource of each zone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ZoneReset {
    last: ResetOrder,
}

impl Resource for ZoneReset {
    const NAME: &'static str = "missions.zone_reset";
}

/// The number of the last season whose soft reset a resident got. It moves
/// with the resident between zones.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeasonMark {
    /// Number of the season. 0: none yet.
    pub season: u32,
}

impl Component for SeasonMark {
    const NAME: &'static str = "missions.season_mark";
}

/// Returns `value * bp / 10,000`, rounded toward zero.
fn part(value: i64, bp: i64) -> i64 {
    let bp = i128::from(bp.clamp(0, 10_000));
    i64::try_from(i128::from(value) * bp / 10_000).unwrap_or(value)
}

/// Lets an attribute decay toward zero. Returns the new value.
fn decay(realm: &Realm, idx: Idx, attr: AttrId, keep_bp: i64) -> i64 {
    let before = realm.attr(idx, attr);
    let after = part(before, keep_bp);
    if after == before { before } else { realm.attr_set(idx, attr, after) }
}

/// Applies a reset order to one resident, unless the resident got the reset
/// of that season already. Returns the cents that were destroyed, or
/// nothing when the resident was skipped.
fn apply(realm: &Realm, p: &Params, order: &ResetOrder, idx: Idx) -> Option<i64> {
    {
        let mut marks = realm.col_mut::<SeasonMark>();
        if marks[idx].season >= order.season {
            return None;
        }
        marks[idx].season = order.season;
    }
    let id = realm.id_of(idx);
    let me = Holder::Resident(id);
    let standing = realm.attr(idx, p.standing);
    let notoriety = realm.attr(idx, p.notoriety);
    let standing_now = decay(realm, idx, p.standing, order.standing_keep_bp);
    let notoriety_now = decay(realm, idx, p.notoriety, order.notoriety_keep_bp);
    let cash = realm.ledger().money(me).cents();
    let above = cash.saturating_sub(order.wealth_floor.max(0)).max(0);
    let mut cut = part(above, order.wealth_cut_bp);
    if cut > 0 && realm.ledger_mut().burn(me, p.season_sink, Money(cut)).is_err() {
        cut = 0;
    }
    if cut > 0 || standing_now != standing || notoriety_now != notoriety {
        realm.notify(
            id,
            "missions.season_reset",
            None,
            json!({
                "season": order.season,
                "cash_cut": cut,
                "standing": standing_now,
                "notoriety": notoriety_now,
            }),
        );
    }
    Some(cut)
}

/// A zone gets the order: it applies the reset to every resident that is
/// there, and tells the world what it destroyed.
fn on_order(realm: &Realm, p: &Params, order: ResetOrder) {
    {
        let mut zone = realm.res_mut::<ZoneReset>();
        if order.season <= zone.last.season {
            return;
        }
        zone.last = order;
    }
    let mut residents = 0u64;
    let mut burned = 0i64;
    for idx in realm.indices() {
        if let Some(cut) = apply(realm, p, &order, idx) {
            residents += 1;
            burned += cut;
        }
    }
    let zone = realm.zone_info().map(|zone| zone.id);
    realm.emit(
        EventMeta::public("missions.season_reset_applied", params::drama(realm, p.drama_reset)),
        &json!({
            "season": order.season,
            "zone": zone,
            "residents": residents,
            "burned": burned,
        }),
    );
}

/// Registers the zone side of the soft reset.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() != Scope::Zone {
        return;
    }
    b.component::<SeasonMark>();
    b.resource::<ZoneReset>();
    b.on_message::<ResetOrder>(move |realm, _from, order| on_order(realm, &p, order));
    // A new resident has nothing to reset: it starts with the season of its
    // zone.
    b.on_spawn(0, |realm, idx| {
        let season = realm.res::<ZoneReset>().last.season;
        realm.col_mut::<SeasonMark>()[idx].season = season;
    });
    // A resident that left a late zone before the reset gets it on arrival.
    b.on_arrive(move |realm, idx| {
        let order = realm.res::<ZoneReset>().last;
        if order.season > 0 {
            apply(realm, &p, &order, idx);
        }
    });
}
