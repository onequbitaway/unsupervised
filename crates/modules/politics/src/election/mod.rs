//! Elections: the calendar, the candidates, the campaign, the vote and the
//! count.

pub(crate) mod campaign;
pub(crate) mod fund;
pub(crate) mod register;
pub(crate) mod schedule;
pub(crate) mod tally;
pub(crate) mod vote;

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ActionError, Builder, Scope, Stage, Value};

use crate::model::Gov;
use crate::params::Params;
use crate::remote::Origin;
use crate::state::Politics;
use crate::state::election::{Election, Phase};

/// Order of the calendar system among the systems of the module. It runs
/// first, so that the other systems see the offices of this frame.
pub(crate) const ORDER_CALENDAR: i32 = 0;
/// Order of the system that ends rallies.
const ORDER_RALLIES: i32 = 5;

/// Returns the fields that name an election in an event, a notice or a
/// result: the government, `election`, `office` and `kind`.
pub(crate) fn facts(election: &Election) -> Map<String, Value> {
    let mut out = election.gov.describe();
    out.insert("election".into(), json!(election.id));
    out.insert("office".into(), json!(election.office.name()));
    out.insert("kind".into(), json!(election.cause.name()));
    out
}

/// Like [`facts`], with more fields.
pub(crate) fn facts_with(election: &Election, more: Value) -> Value {
    let mut out = facts(election);
    if let Value::Object(more) = more {
        out.extend(more);
    }
    Value::Object(out)
}

/// Finds an open election.
pub(crate) fn find(state: &Politics, id: u64) -> Result<&Election, ActionError> {
    state
        .elections
        .get(&id)
        .ok_or_else(|| ActionError::not_found(format!("No open election has the id {id}.")))
}

/// The error of a verb that came in the wrong phase of an election.
pub(crate) fn wrong_phase(election: &Election, need: &str) -> ActionError {
    ActionError::conflict(format!(
        "This election is in the {} phase. {need}",
        election.phase.name()
    ))
    .with_details(json!({
        "phase": election.phase.name(),
        "phase_ends": election.phase_ends(),
        "polls_open": election.polls_open,
        "polls_close": election.polls_close,
    }))
}

/// Checks that a resident is in the city of a city election. A national
/// election has no place.
pub(crate) fn require_place(election: &Election, origin: &Origin) -> Result<(), ActionError> {
    match election.gov {
        Gov::City(city) if city != origin.city => Err(ActionError::not_here(format!(
            "This election is in city {city}. You are in city {}.",
            origin.city
        ))),
        _ => Ok(()),
    }
}

/// Returns true while donors can give to a candidate.
pub(crate) fn takes_donations(phase: Phase) -> bool {
    matches!(phase, Phase::Registration | Phase::Campaign)
}

/// Registers the verbs and the systems of the elections.
pub(crate) fn build(b: &mut Builder, p: Params) {
    register::build(b, p);
    fund::build(b, p);
    campaign::build(b, p);
    vote::build(b, p);
    if b.scope() == Scope::Global {
        b.system(Stage::Update, ORDER_CALENDAR, "politics.calendar", move |realm| {
            schedule::run(realm, &p);
        });
        b.system(Stage::Update, ORDER_RALLIES, "politics.rallies", campaign::end_due);
    }
}
