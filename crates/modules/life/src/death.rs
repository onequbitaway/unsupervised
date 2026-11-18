//! Death and estates.
//!
//! A resident that dies leaves the world. Its money and goods are its
//! estate. The estate goes to the heir of its will, else to its spouse
//! (`Kin::successor`), else to the treasury of the city where it died.
//!
//! A zone cannot know if an heir in another zone still lives. So an estate
//! with an heir goes through the global realm: the kernel puts the holdings
//! in an escrow there, and the global realm sends them to the heir when the
//! heir lives, else to the city treasury.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{
    Audience, Builder, CityId, Dest, EventMeta, Holder, Idx, Kin, Message, Money, OrgId, Realm,
    ResidentId, Resource, Scope, Stage,
};

use crate::body::Body;
use crate::names::attr;

/// An estate that waits in the global realm for its heir.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estate {
    /// The escrow that holds the estate.
    pub escrow: u64,
    /// The resident that died.
    pub deceased: ResidentId,
    /// The resident that inherits, if it lives.
    pub heir: ResidentId,
    /// The city that inherits when the heir does not live.
    pub city: CityId,
    /// Ticks that the estate has waited for its holdings.
    #[serde(default)]
    waited: u32,
}

impl Message for Estate {
    const KIND: &'static str = "life.estate";
}

/// The estates that the global realm has not settled yet.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Estates {
    open: Vec<Estate>,
}

impl Estates {
    /// Returns the estates that wait.
    #[must_use]
    pub fn open(&self) -> &[Estate] {
        &self.open
    }
}

impl Resource for Estates {
    const NAME: &'static str = "life.estates";
}

/// Makes a resident die of `cause`, for example `starvation` or `old_age`.
///
/// The life module calls it for its own causes of death. A module that
/// depends on the life module calls it for its causes. It emits
/// `life.resident_died` and removes the resident at the end of the tick.
/// The estate goes to the heir, the spouse or the city. A second call for
/// the same resident in the same tick does nothing.
pub fn die(realm: &Realm, idx: Idx, cause: &str) {
    let Some(zone) = realm.zone_info() else {
        return;
    };
    {
        let mut bodies = realm.col_mut::<Body>();
        if bodies[idx].dead {
            return;
        }
        bodies[idx].dead = true;
    }
    let resident = realm.id_of(idx);
    let defs = realm.defs();
    let param = |name: &str| defs.param_id(name).map_or(0, |id| realm.param(id));
    let age = defs.attr_id(attr::AGE).map_or(0, |id| realm.attr(idx, id));
    let successor = realm.col::<Kin>()[idx].successor().filter(|heir| *heir != resident);
    let city = Holder::Org(OrgId::city(zone.city));
    let estate = realm.ledger().money(Holder::Resident(resident));
    let drama = u8::try_from(param("life.drama.died").clamp(0, 100)).unwrap_or(0);
    let radius = u32::try_from(param("kernel.sight_radius")).unwrap_or(0);
    realm.emit(
        EventMeta::public("life.resident_died", drama)
            .actor(resident)
            .at(realm.pos(idx))
            .audience(Audience::Nearby(radius)),
        &json!({
            "resident": resident,
            "cause": cause,
            "age": age,
            "heir": successor.map_or(city, Holder::Resident),
            "estate": estate,
        }),
    );
    match successor {
        Some(heir) => {
            let escrow = realm.alloc_id();
            realm.send(
                Dest::Global,
                &Estate { escrow, deceased: resident, heir, city: zone.city, waited: 0 },
            );
            realm.despawn(idx, Holder::Escrow(escrow), cause);
        }
        None => realm.despawn(idx, city, cause),
    }
}

/// Moves everything in the escrow of an estate to the heir or to the city.
/// Returns false when the escrow is empty.
fn settle(realm: &Realm, estate: &Estate) -> bool {
    let escrow = Holder::Escrow(estate.escrow);
    let (money, items) = {
        let ledger = realm.ledger();
        (ledger.money(escrow), ledger.items_of(escrow))
    };
    if money == Money::ZERO && items.is_empty() {
        return false;
    }
    let defs = realm.defs();
    let goods: Map<String, Value> =
        items.iter().map(|&(item, qty)| (defs.item_name(item).to_string(), json!(qty))).collect();
    let to_heir = realm.directory().is_alive(estate.heir);
    let receiver =
        if to_heir { Holder::Resident(estate.heir) } else { Holder::Org(OrgId::city(estate.city)) };
    // A transfer out of the escrow cannot fail: the amounts were read from
    // it in this call. If one does, the rest stays in the escrow.
    let mut moved = true;
    if to_heir {
        let to = Dest::Resident(estate.heir);
        moved &= realm.wire_money(escrow, to, receiver, money, "inherit").is_ok();
        for &(item, qty) in &items {
            moved &= realm.wire_item(escrow, to, receiver, item, qty, "inherit").is_ok();
        }
        realm.notify(
            estate.heir,
            "life.inheritance",
            Some(estate.deceased),
            json!({ "from": estate.deceased, "money": money, "items": goods }),
        );
    } else {
        let mut ledger = realm.ledger_mut();
        moved &= ledger.transfer_money(escrow, receiver, money, "inherit").is_ok();
        for &(item, qty) in &items {
            moved &= ledger.transfer_item(escrow, receiver, item, qty, "inherit").is_ok();
        }
    }
    let drama = defs.param_id("life.drama.estate").map_or(0, |id| realm.param(id));
    realm.emit(
        EventMeta::public("life.estate_settled", u8::try_from(drama.clamp(0, 100)).unwrap_or(0)),
        &json!({
            "resident": estate.deceased,
            "heir": receiver,
            "money": money,
            "items": goods,
            "complete": moved,
        }),
    );
    true
}

/// The system of the global realm that settles estates.
fn settle_estates(realm: &Realm) {
    if realm.res::<Estates>().open.is_empty() {
        return;
    }
    // The holdings come in the frame of the death. An estate that is
    // still empty after the wait had nothing.
    let wait = realm
        .defs()
        .param_id("life.estate_wait_ticks")
        .map_or(1, |id| u32::try_from(realm.param(id)).unwrap_or(1));
    let open = std::mem::take(&mut realm.res_mut::<Estates>().open);
    let mut waiting = Vec::new();
    for mut estate in open {
        if settle(realm, &estate) {
            continue;
        }
        estate.waited = estate.waited.saturating_add(1);
        if estate.waited < wait {
            waiting.push(estate);
        }
    }
    realm.res_mut::<Estates>().open = waiting;
}

/// Registers the estate handling of the global realm.
pub(crate) fn register(b: &mut Builder) {
    if b.scope() != Scope::Global {
        return;
    }
    b.resource::<Estates>();
    b.on_message::<Estate>(|realm, _from, estate| {
        realm.res_mut::<Estates>().open.push(estate);
    });
    b.system(Stage::Update, 0, "life.estates", settle_estates);
}
