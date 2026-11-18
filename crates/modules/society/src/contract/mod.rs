//! Contracts: a small general engine with parties, clauses and escrow.

mod args;
mod claim;
mod engine;
mod estate;
mod inspect;
pub(crate) mod model;
mod observe;
mod offer;
mod sign;
mod view;

use unsup_kernel::{Builder, Realm, ResidentId, Scope, Stage};

use self::model::Contracts;
use crate::params::Params;

/// The events that a clause can wait for, as prefixes of the event kind:
/// the events of every game module. The global realm gets each event with
/// one of these prefixes from every zone.
///
/// The speech events of this module (`society.said`, `society.posted`) are
/// left out on purpose: they are many, and no contract needs them.
pub(crate) const WATCHED: [&str; 12] = [
    "life.",
    "economy.",
    "finance.",
    "crime.",
    "politics.",
    "missions.",
    "society.org_",
    "society.contract_",
    "society.married",
    "society.divorced",
    "society.household_",
    "society.opinion_",
];

/// Ends the offers of a resident that left the world, and the offers that
/// name it. A signed contract goes on: the engine carries it out to the
/// end.
pub(crate) fn on_death(realm: &Realm, _p: &Params, dead: ResidentId) {
    let mut contracts = realm.res_mut::<Contracts>();
    for id in contracts.ids_of(dead) {
        if contracts.open.get(&id).is_some_and(|contract| !contract.signed) {
            engine::close_offer(realm, &mut contracts, id, "party_left", None);
        }
    }
}

/// Registers the engine, the verbs and the observer.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.resource::<Contracts>();
        b.resource::<estate::Estates>();
        // Registered before the handlers of the engine, so that the heir is
        // known when the engine pays.
        b.on_event("life.resident_died", estate::on_died);
        b.system(Stage::Update, 0, "society.contracts", move |realm| engine::tick(realm, &p));
        for prefix in WATCHED {
            b.on_event(prefix, move |realm, event| engine::on_event(realm, &p, event));
        }
    }
    offer::build(b, p);
    sign::build(b, p);
    claim::build(b, p);
    inspect::build(b);
    b.observe_global("contracts", 31, move |realm, resident, opts| {
        observe::section(realm, &p, resident, opts)
    });
}
