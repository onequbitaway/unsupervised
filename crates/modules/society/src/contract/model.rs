//! The state of the contract engine: contracts, clauses and the indexes
//! that the engine needs to act in time.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::{Holder, ItemId, ResidentId, Resource, SiteId, Value};

/// One of the two parties of a contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub(crate) enum Side {
    /// The resident that made the offer.
    Offerer,
    /// The resident that signs the offer.
    Counterparty,
}

impl Side {
    /// Returns the other party.
    pub(crate) fn other(self) -> Side {
        match self {
            Side::Offerer => Side::Counterparty,
            Side::Counterparty => Side::Offerer,
        }
    }

    /// Returns the name of the party.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Side::Offerer => "offerer",
            Side::Counterparty => "counterparty",
        }
    }
}

/// A value that an event clause compares with a data field of an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Scalar {
    Int(i64),
    Text(String),
    Bool(bool),
}

impl Scalar {
    /// Returns true when a data field of an event has this value.
    pub(crate) fn matches(&self, field: &Value) -> bool {
        match self {
            Scalar::Int(want) => field.as_i64() == Some(*want),
            Scalar::Text(want) => field.as_str() == Some(want.as_str()),
            Scalar::Bool(want) => field.as_bool() == Some(*want),
        }
    }

    /// Returns the value as JSON.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            Scalar::Int(v) => Value::from(*v),
            Scalar::Text(v) => Value::from(v.as_str()),
            Scalar::Bool(v) => Value::from(*v),
        }
    }
}

/// What a clause demands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Term {
    /// Pay the other party when the contract is signed.
    PayAtSigning { amount: i64 },
    /// Pay the other party at the deadline.
    PayAtDeadline { amount: i64 },
    /// Pay the other party when an event of a kind happens whose data
    /// fields have the given values, at the deadline or before it.
    PayOnEvent {
        amount: i64,
        event: String,
        fields: Vec<(String, Scalar)>,
        actor: Option<ResidentId>,
    },
    /// Hand goods to the other party, or put them in a site, at the
    /// deadline or before it.
    Deliver { item: ItemId, qty: i64, site: Option<SiteId> },
    /// Money that the other party gets when this party breaches.
    Penalty { amount: i64 },
}

impl Term {
    /// Returns the cents that the clause locks in escrow.
    pub(crate) fn stake(&self) -> i64 {
        match self {
            Term::PayAtSigning { amount }
            | Term::PayAtDeadline { amount }
            | Term::PayOnEvent { amount, .. }
            | Term::Penalty { amount } => *amount,
            Term::Deliver { .. } => 0,
        }
    }

    /// Returns true when the clause has a deadline.
    pub(crate) fn has_deadline(&self) -> bool {
        matches!(self, Term::PayAtDeadline { .. } | Term::PayOnEvent { .. } | Term::Deliver { .. })
    }

    /// Returns the name of the kind of clause.
    pub(crate) fn kind(&self) -> &'static str {
        match self {
            Term::PayAtSigning { .. } => "pay_at_signing",
            Term::PayAtDeadline { .. } => "pay_at_deadline",
            Term::PayOnEvent { .. } => "pay_on_event",
            Term::Deliver { .. } => "deliver",
            Term::Penalty { .. } => "penalty",
        }
    }
}

/// What became of a clause.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Fate {
    /// Not decided yet.
    Open,
    /// The payment was made or the goods were delivered.
    Done,
    /// The money went back to the party that locked it.
    Refunded,
    /// The goods were not delivered in time.
    Missed,
    /// The penalty went to the other party.
    Forfeited,
    /// The contract ended before the clause was due.
    Void,
}

impl Fate {
    /// Returns the name of the state.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Fate::Open => "open",
            Fate::Done => "done",
            Fate::Refunded => "refunded",
            Fate::Missed => "missed",
            Fate::Forfeited => "forfeited",
            Fate::Void => "void",
        }
    }
}

/// One clause of a contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Clause {
    /// The party that pays or delivers.
    pub(crate) by: Side,
    pub(crate) term: Term,
    /// Game minutes from the signing to the deadline. 0 without a deadline.
    pub(crate) due_in: u32,
    /// The frame of the deadline. 0 before the signing and without a
    /// deadline.
    pub(crate) due: u64,
    pub(crate) fate: Fate,
    /// While a delivery is checked in a zone: the frame until which the
    /// engine waits for the answer. 0 otherwise.
    pub(crate) lock: u64,
}

impl Clause {
    /// Makes an open clause.
    pub(crate) fn new(by: Side, term: Term, due_in: u32) -> Clause {
        Clause { by, term, due_in, due: 0, fate: Fate::Open, lock: 0 }
    }

    /// Returns true when the engine still waits for the clause.
    pub(crate) fn is_open(&self) -> bool {
        self.fate == Fate::Open
    }
}

/// An offer or a signed contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Contract {
    pub(crate) id: u64,
    pub(crate) offerer: ResidentId,
    /// Before the signing: the resident that can sign, or nobody for an
    /// offer to everyone. After it: the resident that signed.
    pub(crate) other: Option<ResidentId>,
    pub(crate) signed: bool,
    pub(crate) clauses: Vec<Clause>,
    /// The last frame in which the offer can be signed.
    pub(crate) expires: u64,
}

impl Contract {
    /// Returns the escrow that holds the locked money of the contract.
    pub(crate) fn escrow(&self) -> Holder {
        Holder::Escrow(self.id)
    }

    /// Returns the resident on one side.
    pub(crate) fn party(&self, side: Side) -> Option<ResidentId> {
        match side {
            Side::Offerer => Some(self.offerer),
            Side::Counterparty => self.other,
        }
    }

    /// Returns the side of a resident.
    pub(crate) fn side_of(&self, resident: ResidentId) -> Option<Side> {
        if resident == self.offerer {
            Some(Side::Offerer)
        } else if self.other == Some(resident) {
            Some(Side::Counterparty)
        } else {
            None
        }
    }

    /// Returns the cents that one side locks in escrow.
    pub(crate) fn part(&self, side: Side) -> i64 {
        self.clauses.iter().filter(|c| c.by == side).map(|c| c.term.stake()).sum()
    }

    /// Returns the frame of the next deadline of an open clause.
    pub(crate) fn next_due(&self) -> Option<u64> {
        self.clauses.iter().filter(|c| c.is_open() && c.due > 0).map(|c| c.due).min()
    }
}

/// Every open offer and every signed contract that has not ended. A
/// resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contracts {
    pub(crate) open: BTreeMap<u64, Contract>,
    /// The contracts of each resident as offerer or as signer.
    pub(crate) by_party: BTreeMap<ResidentId, BTreeSet<u64>>,
    /// The offers that name a resident as the one that can sign.
    pub(crate) offered_to: BTreeMap<ResidentId, BTreeSet<u64>>,
    /// Frames at which the engine must look at a contract.
    pub(crate) wake: BTreeSet<(u64, u64)>,
    /// Event kind to the signed contracts with an open clause for it.
    pub(crate) watch: BTreeMap<String, BTreeSet<u64>>,
}

impl Resource for Contracts {
    const NAME: &'static str = "society.contracts";
}

fn link(index: &mut BTreeMap<ResidentId, BTreeSet<u64>>, resident: ResidentId, id: u64) {
    index.entry(resident).or_default().insert(id);
}

fn unlink(index: &mut BTreeMap<ResidentId, BTreeSet<u64>>, resident: ResidentId, id: u64) {
    if let Some(set) = index.get_mut(&resident) {
        set.remove(&id);
        if set.is_empty() {
            index.remove(&resident);
        }
    }
}

impl Contracts {
    /// Returns the number of offers and signed contracts that have not
    /// ended.
    #[must_use]
    pub fn len(&self) -> usize {
        self.open.len()
    }

    /// Returns true when there is no open offer and no signed contract.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.open.is_empty()
    }

    /// Returns how many contracts a resident has open as offerer or signer.
    pub(crate) fn count_of(&self, resident: ResidentId) -> usize {
        self.by_party.get(&resident).map_or(0, BTreeSet::len)
    }

    /// Adds a new offer.
    pub(crate) fn add_offer(&mut self, contract: Contract) {
        link(&mut self.by_party, contract.offerer, contract.id);
        if let Some(named) = contract.other {
            link(&mut self.offered_to, named, contract.id);
        }
        self.wake.insert((contract.expires, contract.id));
        self.open.insert(contract.id, contract);
    }

    /// Records that a resident signed an offer: the indexes follow.
    pub(crate) fn index_signed(&mut self, id: u64, signer: ResidentId) {
        let Some(contract) = self.open.get(&id) else {
            return;
        };
        if let Some(named) = contract.other {
            unlink(&mut self.offered_to, named, id);
        }
        link(&mut self.by_party, signer, id);
        self.wake.remove(&(contract.expires, id));
        for clause in &contract.clauses {
            if clause.due > 0 {
                self.wake.insert((clause.due, id));
            }
            if let Term::PayOnEvent { event, .. } = &clause.term {
                self.watch.entry(event.clone()).or_default().insert(id);
            }
        }
    }

    /// Removes a contract and everything that points to it.
    pub(crate) fn remove(&mut self, id: u64) -> Option<Contract> {
        let contract = self.open.remove(&id)?;
        unlink(&mut self.by_party, contract.offerer, id);
        if let Some(other) = contract.other {
            unlink(&mut self.by_party, other, id);
            unlink(&mut self.offered_to, other, id);
        }
        self.wake.remove(&(contract.expires, id));
        for clause in &contract.clauses {
            self.wake.remove(&(clause.due, id));
            self.wake.remove(&(clause.lock, id));
            if let Term::PayOnEvent { event, .. } = &clause.term
                && let Some(set) = self.watch.get_mut(event)
            {
                set.remove(&id);
                if set.is_empty() {
                    self.watch.remove(event);
                }
            }
        }
        Some(contract)
    }

    /// Returns the ids of the contracts that a resident can see: those it
    /// offered or signed, and the offers that name it.
    pub(crate) fn ids_of(&self, resident: ResidentId) -> Vec<u64> {
        let own = self.by_party.get(&resident).into_iter().flatten();
        let offered = self.offered_to.get(&resident).into_iter().flatten();
        let mut ids: Vec<u64> = own.chain(offered).copied().collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}
