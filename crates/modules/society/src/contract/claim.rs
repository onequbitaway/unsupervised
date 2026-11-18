//! `contract_claim`: a party shows that it delivered goods.
//!
//! Goods are in the zones and contracts are in the global realm, so a
//! delivery takes three steps:
//!
//! 1. The zone of the resident asks the global realm.
//! 2. The global realm finds the clause, locks it for a short time and
//!    sends the order back to the resident: which goods, how many, where.
//! 3. The zone where the resident is checks the place, moves the goods and
//!    tells the global realm what happened. That ends the action.
//!
//! While the clause is locked the engine does not judge the contract, so a
//! delivery that started in time counts. A claim that the resident stops
//! before step 3 moves nothing.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Begin, Builder, Dest, Holder, Idx, ItemId, Message, Realm, ResidentId, Scope,
    SiteId, Value,
};

use super::args::ClaimArgs;
use super::engine::advance;
use super::model::{Contracts, Fate, Term};
use super::offer::no_contract;
use crate::params::{self, Params};
use crate::remote::{Origin, origin_of};
use crate::spec;

/// Ledger label of goods that a contract moves.
const DELIVERY: &str = "contract.delivery";

/// Step 1: a resident wants to deliver.
#[derive(Serialize, Deserialize)]
struct ClaimAsk {
    origin: Origin,
    contract: u64,
    clause: Option<u32>,
}

impl Message for ClaimAsk {
    const KIND: &'static str = "society.contract_claim";
}

/// Step 2: what the resident must deliver, and where.
#[derive(Serialize, Deserialize)]
struct DeliverOrder {
    action_id: u64,
    contract: u64,
    clause: u32,
    item: ItemId,
    qty: i64,
    site: Option<SiteId>,
    receiver: ResidentId,
}

impl Message for DeliverOrder {
    const KIND: &'static str = "society.contract_deliver";
}

/// Step 3: what the zone did.
#[derive(Serialize, Deserialize)]
struct DeliverResult {
    contract: u64,
    clause: u32,
    delivered: bool,
}

impl Message for DeliverResult {
    const KIND: &'static str = "society.contract_delivered";
}

/// The global realm chooses the clause and locks it.
fn order(realm: &Realm, p: &Params, ask: &ClaimAsk) -> Result<DeliverOrder, ActionError> {
    let mut contracts = realm.res_mut::<Contracts>();
    let now = realm.frame();
    let contract =
        contracts.open.get_mut(&ask.contract).ok_or_else(|| no_contract(ask.contract))?;
    if !contract.signed {
        return Err(ActionError::conflict("The contract is not signed."));
    }
    let Some(side) = contract.side_of(ask.origin.resident) else {
        return Err(ActionError::not_allowed("You are not a party of this contract."));
    };
    let receiver = contract.party(side.other()).unwrap_or(contract.offerer);
    let owed = |index: usize| {
        contract.clauses.get(index).is_some_and(|clause| {
            clause.by == side && clause.is_open() && matches!(clause.term, Term::Deliver { .. })
        })
    };
    let index = match ask.clause {
        Some(index) => Some(index as usize).filter(|&index| owed(index)),
        None => (0..contract.clauses.len()).find(|&index| owed(index)),
    };
    let Some(index) = index else {
        return Err(ActionError::conflict("You have no open delivery in this contract."));
    };
    let clause = &mut contract.clauses[index];
    if clause.lock > now {
        return Err(ActionError::conflict("This delivery is being checked already."));
    }
    let Term::Deliver { item, qty, site } = clause.term else {
        return Err(ActionError::internal("The clause is not a delivery."));
    };
    clause.lock = now + realm.param(p.global_timeout).max(1).unsigned_abs();
    Ok(DeliverOrder {
        action_id: ask.origin.action_id,
        contract: ask.contract,
        clause: index as u32,
        item,
        qty,
        site,
        receiver,
    })
}

/// The zone checks the place and moves the goods.
fn deliver(
    realm: &Realm,
    p: &Params,
    idx: Idx,
    order: &DeliverOrder,
) -> Result<Value, ActionError> {
    let resident = realm.id_of(idx);
    let pos = realm.pos(idx);
    let to = match order.site {
        Some(site) => {
            if !realm.sites().get(site).is_some_and(|found| found.rect.contains(pos)) {
                return Err(ActionError::not_here(format!(
                    "Go to site {site} and use `contract_claim` there."
                ))
                .with_details(json!({ "site": site })));
            }
            Holder::Site(site)
        }
        None => {
            let radius = params::span(realm, p.contract_handover_radius);
            let near = realm
                .idx_of(order.receiver)
                .is_some_and(|other| realm.pos(other).dist(pos) <= radius);
            if !near {
                return Err(ActionError::not_here(format!(
                    "Go to resident {} and use `contract_claim` within {radius} tiles.",
                    order.receiver
                ))
                .with_details(json!({ "resident": order.receiver })));
            }
            Holder::Resident(order.receiver)
        }
    };
    realm.ledger_mut().transfer_item(
        Holder::Resident(resident),
        to,
        order.item,
        order.qty,
        DELIVERY,
    )?;
    Ok(json!({
        "contract": order.contract,
        "clause": order.clause,
        "item": realm.defs().item_name(order.item),
        "qty": order.qty,
    }))
}

/// The global realm records what the zone did.
fn record(realm: &Realm, p: &Params, result: &DeliverResult) {
    let mut contracts = realm.res_mut::<Contracts>();
    let Some(contract) = contracts.open.get_mut(&result.contract) else {
        return;
    };
    let Some(clause) = contract.clauses.get_mut(result.clause as usize) else {
        return;
    };
    clause.lock = 0;
    if result.delivered && clause.is_open() {
        clause.fate = Fate::Done;
        let by = clause.by;
        if let (Some(from), Some(to)) = (contract.party(by), contract.party(by.other())) {
            realm.notify(
                to,
                "society.contract_delivery",
                Some(from),
                json!({ "contract": result.contract, "clause": result.clause }),
            );
        }
    }
    advance(realm, p, &mut contracts, result.contract);
}

/// Registers the verb and the handlers of the three steps.
pub(crate) fn build(b: &mut Builder, p: Params) {
    match b.scope() {
        Scope::Zone => {
            b.on_resident_message::<DeliverOrder>(move |realm, idx, _from, order| {
                // The resident can stop the claim before the order arrives
                // (a cancel, an interrupt, a timeout). The kernel reported
                // that already. Nothing moves, and the clause is free again.
                let waits =
                    realm.activity(idx).is_some_and(|action| action.action_id == order.action_id);
                let result = waits.then(|| deliver(realm, &p, idx, &order));
                realm.send(
                    Dest::Global,
                    &DeliverResult {
                        contract: order.contract,
                        clause: order.clause,
                        delivered: matches!(result, Some(Ok(_))),
                    },
                );
                if let Some(result) = result {
                    realm.complete_action(realm.id_of(idx), order.action_id, result);
                }
            });
        }
        Scope::Global => {
            b.on_message::<ClaimAsk>(move |realm, _from, ask| match order(realm, &p, &ask) {
                Ok(order) => realm.send(Dest::Resident(ask.origin.resident), &order),
                Err(error) => {
                    realm.complete_action(ask.origin.resident, ask.origin.action_id, Err(error));
                }
            });
            b.on_message::<DeliverResult>(move |realm, _from, result| record(realm, &p, &result));
        }
    }
    b.verb::<ClaimArgs>(
        spec("contract_claim", "Deliver the goods of a `deliver` clause. Carry them to the site of the clause, or to the other party."),
        move |realm, ctx, args| {
            let origin = origin_of(realm, ctx, 0)?;
            realm.send(Dest::Global, &ClaimAsk { origin, contract: args.contract, clause: args.clause });
            Ok(Begin::Wait { timeout: params::span(realm, p.global_timeout) })
        },
        |_realm, _ctx, _args| Ok(Value::Null),
    );
}
