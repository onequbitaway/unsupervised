//! How a contract looks in a notice and in an observation.
//!
//! Most of it is made by the engine from numbers and from the names of the
//! item catalogue. The offerer wrote two things: the kind of the event and
//! the data fields of an event clause. They passed the checks of `args.rs`
//! and have no spaces, but they are text of a resident. They go out as
//! `Untrusted`, like all such text.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Realm, ResidentId, Value};

use super::model::{Clause, Contract, Term};
use crate::text;

/// Returns one clause as JSON. `offerer` is the resident that wrote it.
pub(crate) fn clause(realm: &Realm, offerer: ResidentId, clause: &Clause) -> Value {
    let mut out = Map::new();
    out.insert("kind".into(), json!(clause.term.kind()));
    out.insert("by".into(), json!(clause.by.name()));
    match &clause.term {
        Term::PayAtSigning { amount }
        | Term::PayAtDeadline { amount }
        | Term::Penalty { amount } => {
            out.insert("amount".into(), json!(amount));
        }
        Term::PayOnEvent { amount, event, fields, actor } => {
            out.insert("amount".into(), json!(amount));
            out.insert("event".into(), text::untrusted(offerer, event));
            if !fields.is_empty() {
                // The fields as the text of a JSON object, names in order.
                let fields: Map<String, Value> =
                    fields.iter().map(|(name, value)| (name.clone(), value.to_value())).collect();
                let written = Value::Object(fields).to_string();
                out.insert("fields".into(), text::untrusted(offerer, &written));
            }
            if let Some(actor) = actor {
                out.insert("actor".into(), json!(actor));
            }
        }
        Term::Deliver { item, qty, site } => {
            out.insert("item".into(), json!(realm.defs().item_name(*item)));
            out.insert("qty".into(), json!(qty));
            if let Some(site) = site {
                out.insert("site".into(), json!(site));
            }
        }
    }
    if clause.term.has_deadline() {
        if clause.due > 0 {
            out.insert("due".into(), json!(clause.due));
        } else {
            out.insert("due_in".into(), json!(clause.due_in));
        }
    }
    out.insert("state".into(), json!(clause.fate.name()));
    Value::Object(out)
}

/// Returns every clause of a contract as JSON.
pub(crate) fn clauses(realm: &Realm, contract: &Contract) -> Value {
    Value::Array(contract.clauses.iter().map(|c| clause(realm, contract.offerer, c)).collect())
}
