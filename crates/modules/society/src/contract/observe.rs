//! The observation section `contracts`: the open offers and the signed
//! contracts of a resident.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ObserveOpts, Realm, ResidentId, Value};

use super::model::{Contracts, Side};
use super::view;
use crate::params::{self, Params};

/// Builds the section, or nothing when the resident has no contract.
pub(crate) fn section(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    opts: &ObserveOpts,
) -> Option<Value> {
    let contracts = realm.res::<Contracts>();
    let ids = contracts.ids_of(resident);
    if ids.is_empty() {
        return None;
    }
    // A resident controls how many contracts it offers and signs, but not
    // how many offers name it. Show a bounded list: short with brief
    // detail, and up to the limit of open contracts with full detail.
    let max = if opts.is_full() {
        params::count(realm, p.contracts_open_max)
    } else {
        params::count(realm, p.brief_list_max).min(params::count(realm, p.contracts_open_max))
    };
    let list: Vec<Value> = ids
        .iter()
        .filter_map(|id| contracts.open.get(id))
        .take(max)
        .map(|contract| {
            let mut entry = Map::new();
            entry.insert("contract".into(), json!(contract.id));
            let side =
                if contract.offerer == resident { Side::Offerer } else { Side::Counterparty };
            entry.insert("you_are".into(), json!(side.name()));
            if let Some(other) = contract.party(side.other()) {
                entry.insert("with".into(), json!(other));
            }
            if contract.signed {
                entry.insert("status".into(), json!("active"));
                if let Some(due) = contract.next_due() {
                    entry.insert("next_due".into(), json!(due));
                }
            } else {
                entry.insert("status".into(), json!("offered"));
                entry.insert("expires".into(), json!(contract.expires));
                entry.insert("your_part".into(), json!(contract.part(side)));
            }
            if opts.is_full() {
                entry.insert("clauses".into(), view::clauses(realm, contract));
            } else {
                entry.insert("clauses".into(), json!(contract.clauses.len()));
            }
            Value::Object(entry)
        })
        .collect();
    let mut out = Map::new();
    if ids.len() > list.len() {
        out.insert("more".into(), json!(ids.len() - list.len()));
    }
    out.insert("list".into(), json!(list));
    Some(Value::Object(out))
}
