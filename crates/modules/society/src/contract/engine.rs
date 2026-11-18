//! The contract engine. It pays, refunds and punishes by itself: at the
//! signing, at each deadline and when a watched event happens. No party can
//! stop it, because every cent that a clause can pay is in escrow from the
//! start.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Event, Realm, ResidentId, Value};

use super::estate;
use super::model::{Contract, Contracts, Fate, Side, Term};
use crate::params::{self, Params};
use crate::{public, rep};

/// Ledger label of money that a party locks in escrow.
pub(crate) const ESCROW: &str = "contract.escrow";
/// Ledger label of a payment of a clause.
const PAY: &str = "contract.pay";
/// Ledger label of locked money that goes back.
pub(crate) const REFUND: &str = "contract.refund";
/// Ledger label of a penalty.
const PENALTY: &str = "contract.penalty";

/// How a signed contract ended.
enum Outcome {
    /// Every clause is decided and no delivery was missed.
    Settled,
    /// These parties missed a delivery.
    Breached(Vec<Side>),
}

/// Sends money from the escrow of a contract to the resident on one side.
fn pay_side(realm: &Realm, contract: &Contract, side: Side, amount: i64, reason: &str) {
    if let Some(resident) = contract.party(side) {
        estate::pay(realm, contract.escrow(), resident, amount, reason);
    }
}

/// Forgets the heirs of the parties of a contract that ended, when they
/// have no other contract.
fn settle_estates(realm: &Realm, contracts: &Contracts, contract: &Contract) {
    for side in [Side::Offerer, Side::Counterparty] {
        if let Some(resident) = contract.party(side) {
            estate::settle(realm, contracts, resident);
        }
    }
}

/// Tells both parties something about their contract.
fn notify_parties(realm: &Realm, contract: &Contract, kind: &str, data: &Value) {
    for side in [Side::Offerer, Side::Counterparty] {
        if let Some(resident) = contract.party(side) {
            realm.notify(resident, kind, None, data.clone());
        }
    }
}

/// Makes an offer a signed contract: fixes the deadlines, makes the
/// payments that are due at the signing, and tells the world.
pub(crate) fn activate(
    realm: &Realm,
    p: &Params,
    contracts: &mut Contracts,
    id: u64,
    signer: ResidentId,
) {
    let now = realm.frame();
    let Some(contract) = contracts.open.get_mut(&id) else {
        return;
    };
    // The index still knows the offer by the resident that it named.
    let named = contract.other;
    contract.signed = true;
    for clause in &mut contract.clauses {
        if clause.term.has_deadline() {
            clause.due = now + u64::from(clause.due_in);
        }
    }
    contracts.index_signed(id, signer);
    let Some(contract) = contracts.open.get_mut(&id) else {
        return;
    };
    debug_assert!(named.is_none() || named == Some(signer));
    contract.other = Some(signer);
    let at_signing: Vec<(usize, Side, i64)> = contract
        .clauses
        .iter()
        .enumerate()
        .filter_map(|(i, clause)| match clause.term {
            Term::PayAtSigning { amount } => Some((i, clause.by.other(), amount)),
            _ => None,
        })
        .collect();
    for (i, to, amount) in at_signing {
        pay_side(realm, contract, to, amount, PAY);
        contract.clauses[i].fate = Fate::Done;
    }
    realm.emit(
        public("society.contract_signed", params::drama(realm, p.drama_contract_signed))
            .actor(signer),
        &json!({ "contract": id, "parties": [contract.offerer, signer] }),
    );
    realm.notify(
        contract.offerer,
        "society.contract_signed",
        Some(signer),
        json!({ "contract": id }),
    );
    advance(realm, p, contracts, id);
}

/// Looks at one contract now: lets an offer lapse, makes the payments whose
/// deadline is here, gives back the money of events that did not happen,
/// and ends the contract when a delivery was missed or nothing is left.
pub(crate) fn advance(realm: &Realm, p: &Params, contracts: &mut Contracts, id: u64) {
    let now = realm.frame();
    let Some(contract) = contracts.open.get_mut(&id) else {
        return;
    };
    if !contract.signed {
        if now >= contract.expires {
            close_offer(realm, contracts, id, "expired", None);
        }
        return;
    }
    // A delivery is being checked in a zone. Wait for the answer, so that
    // the goods and the verdict cannot disagree.
    if let Some(lock) = contract.clauses.iter().map(|c| c.lock).filter(|&l| l > now).max() {
        contracts.wake.insert((lock, id));
        return;
    }
    let mut payments: Vec<(Side, i64, &'static str)> = Vec::new();
    let mut breachers: Vec<Side> = Vec::new();
    for clause in &mut contract.clauses {
        if !clause.is_open() || clause.due == 0 || clause.due > now {
            continue;
        }
        match &clause.term {
            Term::PayAtDeadline { amount } => {
                payments.push((clause.by.other(), *amount, PAY));
                clause.fate = Fate::Done;
            }
            Term::PayOnEvent { amount, .. } => {
                payments.push((clause.by, *amount, REFUND));
                clause.fate = Fate::Refunded;
            }
            Term::Deliver { .. } => {
                clause.fate = Fate::Missed;
                if !breachers.contains(&clause.by) {
                    breachers.push(clause.by);
                }
            }
            Term::PayAtSigning { .. } | Term::Penalty { .. } => {}
        }
    }
    for (to, amount, reason) in payments {
        pay_side(realm, contract, to, amount, reason);
    }
    let waits = contract
        .clauses
        .iter()
        .any(|clause| clause.is_open() && !matches!(clause.term, Term::Penalty { .. }));
    if !breachers.is_empty() {
        end(realm, p, contracts, id, Outcome::Breached(breachers));
    } else if !waits {
        end(realm, p, contracts, id, Outcome::Settled);
    }
}

/// Ends a signed contract: decides every clause that is still open, moves
/// the money, changes reputations and tells the world.
fn end(realm: &Realm, p: &Params, contracts: &mut Contracts, id: u64, outcome: Outcome) {
    let Some(mut contract) = contracts.remove(id) else {
        return;
    };
    let breachers = match &outcome {
        Outcome::Settled => &[][..],
        Outcome::Breached(sides) => sides.as_slice(),
    };
    let mut payments: Vec<(Side, i64, &'static str)> = Vec::new();
    for clause in &mut contract.clauses {
        if !clause.is_open() {
            continue;
        }
        match clause.term {
            Term::Penalty { amount } if breachers.contains(&clause.by) => {
                payments.push((clause.by.other(), amount, PENALTY));
                clause.fate = Fate::Forfeited;
            }
            Term::Deliver { .. } => clause.fate = Fate::Void,
            _ => {
                payments.push((clause.by, clause.term.stake(), REFUND));
                clause.fate = Fate::Refunded;
            }
        }
    }
    for (to, amount, reason) in payments {
        pay_side(realm, &contract, to, amount, reason);
    }
    debug_assert_eq!(
        realm.ledger().money(contract.escrow()).cents(),
        0,
        "the escrow of contract {id} is not empty at the end"
    );
    let parties = [contract.offerer, contract.other.unwrap_or(contract.offerer)];
    let lapsed = contract.clauses.iter().any(|clause| {
        clause.fate == Fate::Refunded && matches!(clause.term, Term::PayOnEvent { .. })
    });
    let report = if !breachers.is_empty() {
        let by: Vec<ResidentId> = breachers.iter().filter_map(|s| contract.party(*s)).collect();
        let loss = realm.param(p.rep_contract_breached);
        let drama = params::drama(realm, p.drama_contract_breached);
        for &resident in &by {
            rep::change(realm, resident, -loss);
            realm.emit(
                public("society.contract_breached", drama).actor(resident),
                &json!({ "contract": id, "by": resident, "parties": parties }),
            );
        }
        json!({ "contract": id, "outcome": "breached", "by": by })
    } else if lapsed {
        // An event that a clause waited for did not happen. Nobody broke a
        // promise, and nobody kept all of them.
        realm.emit(
            public("society.contract_expired", params::drama(realm, p.drama_contract_expired)),
            &json!({ "contract": id, "parties": parties }),
        );
        json!({ "contract": id, "outcome": "expired" })
    } else {
        let gain = realm.param(p.rep_contract_fulfilled);
        for resident in parties {
            rep::reward(realm, resident, gain);
        }
        realm.emit(
            public("society.contract_fulfilled", params::drama(realm, p.drama_contract_fulfilled)),
            &json!({ "contract": id, "parties": parties }),
        );
        json!({ "contract": id, "outcome": "fulfilled" })
    };
    notify_parties(realm, &contract, "society.contract_ended", &report);
    settle_estates(realm, contracts, &contract);
}

/// Ends an offer that nobody signed: the offerer gets its money back.
/// `actor` is the party that ended it, if a party did; it gets no notice.
pub(crate) fn close_offer(
    realm: &Realm,
    contracts: &mut Contracts,
    id: u64,
    reason: &str,
    actor: Option<ResidentId>,
) {
    let Some(contract) = contracts.remove(id) else {
        return;
    };
    let locked = realm.ledger().money(contract.escrow()).cents();
    estate::pay(realm, contract.escrow(), contract.offerer, locked, REFUND);
    settle_estates(realm, contracts, &contract);
    for resident in [Some(contract.offerer), contract.other].into_iter().flatten() {
        if Some(resident) != actor {
            realm.notify(
                resident,
                "society.contract_closed",
                actor,
                json!({ "contract": id, "reason": reason }),
            );
        }
    }
}

/// Handles an event that contracts can wait for: pays every open event
/// clause that the event satisfies.
pub(crate) fn on_event(realm: &Realm, p: &Params, event: &Event) {
    let mut contracts = realm.res_mut::<Contracts>();
    let Some(waiting) = contracts.watch.get(&event.kind) else {
        return;
    };
    let waiting: Vec<u64> = waiting.iter().copied().collect();
    for id in waiting {
        let Some(contract) = contracts.open.get_mut(&id) else {
            continue;
        };
        let mut payments: Vec<(Side, i64)> = Vec::new();
        for clause in &mut contract.clauses {
            if !clause.is_open() {
                continue;
            }
            if let Term::PayOnEvent { amount, event: kind, fields, actor } = &clause.term
                && *kind == event.kind
                && actor.is_none_or(|actor| event.actor == Some(actor))
                && fields.iter().all(|(name, want)| want.matches(&event.data[name.as_str()]))
            {
                payments.push((clause.by.other(), *amount));
                clause.fate = Fate::Done;
            }
        }
        if payments.is_empty() {
            continue;
        }
        for (to, amount) in payments {
            pay_side(realm, contract, to, amount, PAY);
        }
        advance(realm, p, &mut contracts, id);
    }
}

/// Runs once in every tick of the global realm: looks at every contract
/// with a deadline that is here.
pub(crate) fn tick(realm: &Realm, p: &Params) {
    let now = realm.frame();
    let mut contracts = realm.res_mut::<Contracts>();
    while let Some(&(frame, id)) = contracts.wake.first() {
        if frame > now {
            break;
        }
        contracts.wake.remove(&(frame, id));
        advance(realm, p, &mut contracts, id);
    }
}
