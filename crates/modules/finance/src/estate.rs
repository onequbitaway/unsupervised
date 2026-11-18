//! The estate of a resident that leaves the world: what happens to its
//! bank account, its loan, its shares and its orders.
//!
//! The kernel moves the cash and the goods that a resident carries to its
//! heir. This file does the same for what the finance module holds for the
//! resident. The heir comes from the event `life.resident_died`. A resident
//! that leaves without that event (an administrator removed it) has no
//! heir: its holdings go to the unclaimed account, like its cash.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::json;
use unsup_kernel::{
    Builder, Dest, EventMeta, Holder, Message, Money, OrgId, Realm, ResidentId, Resource, Scope,
    Stage, UNCLAIMED, ZoneId,
};

use crate::bank::{self, Bank};
use crate::exchange::{Collected, Exchange, collect_order};
use crate::params::Params;
use crate::shares::{self, Shares};
use crate::util::drama;

/// What the global realm remembers for estates.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Estates {
    /// The zones whose exchange has ever had an order. Only they hear
    /// about an estate.
    pub(crate) exchange_zones: BTreeSet<u32>,
    /// The heir that `life.resident_died` named for a resident, until the
    /// resident has left the world.
    pub(crate) heirs: BTreeMap<u64, Option<Holder>>,
    /// Residents that left the world in this frame. Their estates are
    /// settled at the end of the tick.
    pub(crate) gone: Vec<u64>,
}

impl Resource for Estates {
    const NAME: &'static str = "finance.estates";
}

/// A zone tells the global realm that its exchange has its first order.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ExchangeOpened {}

impl Message for ExchangeOpened {
    const KIND: &'static str = "finance.exchange_opened";
}

/// The global realm tells a zone to close the orders of a resident that
/// left the world and to send what they hold to `heir`.
#[derive(Debug, Serialize, Deserialize)]
struct EstateMsg {
    resident: ResidentId,
    heir: Holder,
}

impl Message for EstateMsg {
    const KIND: &'static str = "finance.estate";
}

/// Registers the estate handlers.
pub(crate) fn register(b: &mut Builder, p: Params) {
    match b.scope() {
        Scope::Global => {
            b.resource::<Estates>();
            b.on_message::<ExchangeOpened>(|realm, from, _msg| {
                if let Some(zone) = from.zone() {
                    realm.res_mut::<Estates>().exchange_zones.insert(zone.0);
                }
            });
            // The two events of one death can come in either order, so the
            // estate is settled after both: at the end of the tick.
            b.on_event("life.resident_died", |realm, event| {
                let named = event.data["resident"].as_u64().map(ResidentId);
                if let Some(resident) = named.or(event.actor) {
                    let heir = serde_json::from_value::<Holder>(event.data["heir"].clone()).ok();
                    realm.res_mut::<Estates>().heirs.insert(resident.0, heir);
                }
            });
            b.on_event("kernel.despawned", |realm, event| {
                if let Some(resident) = event.actor {
                    realm.res_mut::<Estates>().gone.push(resident.0);
                }
            });
            b.system(Stage::Post, 50, "finance.estates", move |realm| {
                if realm.res::<Estates>().gone.is_empty() {
                    return;
                }
                let (gone, zones) = {
                    let mut estates = realm.res_mut::<Estates>();
                    let gone: Vec<(ResidentId, Option<Option<Holder>>)> =
                        std::mem::take(&mut estates.gone)
                            .into_iter()
                            .map(|id| (ResidentId(id), estates.heirs.remove(&id)))
                            .collect();
                    let zones: Vec<u32> = estates.exchange_zones.iter().copied().collect();
                    (gone, zones)
                };
                for (resident, named) in gone {
                    let heir = match named {
                        Some(heir) => heir_of(realm, resident, heir),
                        None => UNCLAIMED,
                    };
                    settle(realm, &p, resident, heir);
                    for &zone in &zones {
                        realm.send(Dest::Zone(ZoneId(zone)), &EstateMsg { resident, heir });
                    }
                }
            });
        }
        Scope::Zone => {
            b.on_message::<EstateMsg>(|realm, _from, msg| close_orders(realm, &msg));
        }
    }
}

/// Returns the holder that gets the estate of a resident that died: the
/// heir when it is another resident or an organization, else the treasury
/// of the city where the resident was, else the unclaimed account.
fn heir_of(realm: &Realm, resident: ResidentId, heir: Option<Holder>) -> Holder {
    match heir {
        Some(Holder::Resident(other)) if other != resident => Holder::Resident(other),
        Some(Holder::Org(org)) => Holder::Org(org),
        _ => realm
            .directory()
            .get(resident)
            .and_then(|entry| realm.layout().zone(entry.zone))
            .map_or(UNCLAIMED, |zone| Holder::Org(OrgId::city(zone.city))),
    }
}

/// Settles the estate of a resident in the global realm: share orders,
/// loan, shares and bank account.
fn settle(realm: &Realm, p: &Params, resident: ResidentId, heir: Holder) {
    let mut bank = realm.res_mut::<Bank>();
    let mut shares = realm.res_mut::<Shares>();

    // Open share orders end. The cash of a bid goes back to the account.
    let orders: Vec<u64> = shares.orders_of(resident).map(|order| order.id).collect();
    for id in orders {
        shares::cancel_order(realm, &mut shares, &mut bank, id);
    }

    // The loan ends: the bank takes what the account can pay and gives up
    // the rest. This is not a default; no property is foreclosed.
    let unpaid = bank::end_loan(&mut bank, realm, p, resident).map_or(0, |end| end.unpaid);

    // The shares go to an heir that is a resident. Any other heir cannot
    // hold shares: they are cancelled.
    let holdings: Vec<(OrgId, i64)> =
        shares.portfolio(resident).map(|(org, holding)| (org, holding.held)).collect();
    let mut passed = 0i64;
    for &(org, held) in &holdings {
        shares.add_held(org, resident, -held);
        if let Holder::Resident(other) = heir {
            shares.add_held(org, other, held);
            passed += held;
        }
    }

    // The balance goes to the heir, and the account is closed.
    let had_account = bank.is_open(resident);
    let balance = Bank::balance(realm, resident);
    if had_account {
        // None of these can fail: the amount is the balance.
        let _ = match heir {
            Holder::Resident(other) if bank.is_open(other) => {
                bank.move_between(realm, resident, other, balance, "estate")
            }
            Holder::Resident(other) => bank.wire_out(realm, resident, other, balance, "estate"),
            other => bank.pay_out(realm, resident, other, balance, "estate"),
        };
        bank.close(resident);
    }
    if had_account || !holdings.is_empty() {
        realm.emit(
            EventMeta::public("finance.estate_settled", drama(realm, p.drama_routine))
                .actor(resident),
            &json!({
                "resident": resident,
                "heir": heir,
                "balance": balance,
                "loan_unpaid": unpaid,
                "shares": passed,
            }),
        );
    }
}

/// Closes the orders of a resident at the exchange of this zone and sends
/// what their escrows hold to the heir.
fn close_orders(realm: &Realm, msg: &EstateMsg) {
    let mut exchange = realm.res_mut::<Exchange>();
    let ids: Vec<u64> = exchange.orders_of(msg.resident).map(|order| order.id).collect();
    let Some(&first) = ids.first() else {
        return;
    };
    // Everything is gathered in the escrow of the first order, then sent.
    let pot = Holder::Escrow(first);
    let mut collected = Collected::default();
    for id in ids {
        collect_order(realm, &mut exchange, id, pot, true, &mut collected);
    }
    let (local, to) = match msg.heir {
        Holder::Resident(heir) => (realm.idx_of(heir).is_some(), Dest::Resident(heir)),
        _ => (false, Dest::Global),
    };
    // None of these can fail: the amounts are what the pot holds.
    if local {
        let mut ledger = realm.ledger_mut();
        let _ = ledger.transfer_money(pot, msg.heir, Money(collected.cash), "estate");
        for (&item, &qty) in &collected.items {
            let _ = ledger.transfer_item(pot, msg.heir, item, qty, "estate");
        }
    } else {
        let _ = realm.wire_money(pot, to, msg.heir, Money(collected.cash), "estate");
        for (&item, &qty) in &collected.items {
            let _ = realm.wire_item(pot, to, msg.heir, item, qty, "estate");
        }
    }
}
