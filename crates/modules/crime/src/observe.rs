//! The observation section `law`: what a resident knows about its own
//! place before the law.
//!
//! The section is small, and absent when it has nothing to say. Times are
//! ticks from now.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Builder, Idx, ObserveOpts, Realm, Scope};

use crate::hot;
use crate::ids::Handles;
use crate::names::perm;
use crate::powers;
use crate::record::{Docket, Record};

/// Order of the section among those of other modules.
const SECTION_ORDER: i32 = 0;

/// Returns the newest `most` entries of a list, and how many were left
/// out.
fn newest<T: Clone>(list: &[T], most: usize) -> (Vec<T>, usize) {
    let skip = list.len().saturating_sub(most);
    (list[skip..].to_vec(), skip)
}

/// Writes a list of trials or jury duties with the ticks that are left.
fn dockets(list: &[Docket], frame: u64, left: &str) -> Value {
    list.iter()
        .map(|entry| {
            json!({
                "case": entry.case,
                "kind": entry.kind,
                left: entry.deadline.saturating_sub(frame),
            })
        })
        .collect()
}

/// Builds the section.
fn law(realm: &Realm, h: &Handles, idx: Idx, opts: &ObserveOpts) -> Option<Value> {
    let frame = realm.frame();
    let most = if opts.is_full() { usize::MAX } else { h.count(realm, h.p.observe_list_max) };
    let mut out = Map::new();

    let record = realm.col::<Record>()[idx].clone();
    if !record.warrants.is_empty() {
        let (shown, more) = newest(&record.warrants, most);
        let list: Vec<Value> =
            shown.iter().map(|w| json!({ "case": w.case, "kind": w.kind })).collect();
        out.insert("warrants".into(), Value::Array(list));
        if more > 0 {
            out.insert("more_warrants".into(), json!(more));
        }
    }

    let detained = realm.has_condition(idx, h.detained);
    let jailed = realm.condition_until(idx, h.jailed);
    if detained || jailed.is_some() {
        let mut custody = Map::new();
        custody.insert("state".into(), json!(if jailed.is_some() { "jailed" } else { "detained" }));
        if let Some(Some(until)) = jailed {
            custody.insert("until".into(), json!(until));
            custody.insert("ticks_left".into(), json!(until.since(realm.tick())));
        }
        if let Some(held) = record.custody {
            custody.insert("site".into(), json!(held.site));
            custody.insert("arrested_by".into(), json!(held.officer));
        }
        out.insert("custody".into(), Value::Object(custody));
    }
    if !record.trials.is_empty() {
        out.insert("trials".into(), dockets(&record.trials, frame, "decided_in"));
    }
    if !record.jury.is_empty() {
        out.insert("jury_duty".into(), dockets(&record.jury, frame, "vote_in"));
    }
    if !record.debts.is_empty() {
        let next = record.debts.iter().map(|debt| debt.due).min().unwrap_or(frame);
        out.insert(
            "debts".into(),
            json!({ "owed": record.owed(), "due_in": next.saturating_sub(frame) }),
        );
    }

    let lots = hot::view(realm, h, idx);
    if !lots.is_empty() {
        let defs = realm.defs();
        let mut money = 0i64;
        let mut items: Map<String, Value> = Map::new();
        for lot in &lots {
            match lot.item {
                None => money = money.saturating_add(lot.qty),
                Some(item) => {
                    let name = defs.item_name(item).to_string();
                    let before = items.get(&name).and_then(Value::as_i64).unwrap_or(0);
                    items.insert(name, json!(before.saturating_add(lot.qty)));
                }
            }
        }
        out.insert("hot".into(), json!({ "money": money, "items": items }));
    }

    if !record.known.is_empty() {
        let (shown, more) = newest(&record.known, most);
        let list: Vec<Value> = shown
            .iter()
            .map(|known| {
                let mut entry = Map::new();
                entry.insert("case".into(), json!(known.case));
                entry.insert("kind".into(), json!(known.kind));
                entry.insert("role".into(), json!(known.role.name()));
                if let Some(suspect) = known.suspect {
                    entry.insert("suspect".into(), json!(suspect));
                }
                entry.insert("reported".into(), json!(known.reported));
                Value::Object(entry)
            })
            .collect();
        out.insert("known_cases".into(), Value::Array(list));
        if more > 0 {
            out.insert("more_known_cases".into(), json!(more));
        }
    }

    let city = powers::city_of(realm);
    let held: Vec<&str> = [perm::ARREST, perm::SEARCH, perm::JUDGE, perm::JAIL]
        .into_iter()
        .filter(|power| powers::has_power(realm, idx, power, city))
        .collect();
    if !held.is_empty() {
        out.insert("powers_here".into(), json!(held));
    }

    (!out.is_empty()).then_some(Value::Object(out))
}

/// Registers the section.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.observe("law", SECTION_ORDER, move |realm, idx, opts| law(realm, &h, idx, opts));
    }
}
