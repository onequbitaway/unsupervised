//! The observation section `politics`: the next voting days, the open
//! elections and petitions where the resident is, its own offices and
//! candidacies, and the bills that wait for it.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Builder, ObserveOpts, OrgId, Realm, ResidentId, Value};
use unsup_society::Orgs;

use crate::calendar;
use crate::election::fund;
use crate::model::Gov;
use crate::offices::{self, GOV_SIGN, GOV_VOTE};
use crate::params::{self, Params};
use crate::state::Politics;
use crate::state::bill::{Bill, Stage};
use crate::state::election::{Election, Phase};
use crate::text;

/// Order of the section among the sections of the global realm.
const ORDER_SECTION: i32 = 60;

/// Starts an entry with the city of a city government. An entry without
/// `city` is about the nation.
fn entry(gov: Gov) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(city) = gov.city() {
        out.insert("city".into(), json!(city));
    }
    out
}

/// Puts a list in the section when it is not empty: at most `max` entries,
/// and the number of the others under `more_<name>`.
fn put(out: &mut Map<String, Value>, name: &str, mut list: Vec<Value>, max: usize) {
    if list.is_empty() {
        return;
    }
    let more = list.len().saturating_sub(max);
    list.truncate(max);
    out.insert(name.into(), json!(list));
    if more > 0 {
        out.insert(format!("more_{name}"), json!(more));
    }
}

/// Returns one open election as a resident sees it.
fn election_entry(realm: &Realm, election: &Election, resident: ResidentId, full: bool) -> Value {
    let mut out = entry(election.gov);
    out.insert("election".into(), json!(election.id));
    out.insert("office".into(), json!(election.office.name()));
    out.insert("phase".into(), json!(election.phase.name()));
    out.insert("minutes_left".into(), json!(calendar::minutes_left(realm, election.phase_ends())));
    out.insert("candidates".into(), json!(election.candidates.len()));
    if election.phase == Phase::Voting {
        out.insert("voted".into(), json!(election.voters.contains_key(&resident)));
    }
    if full {
        let list: Vec<Value> = election
            .standings()
            .into_iter()
            .filter_map(|(id, votes)| {
                let candidate = election.candidates.get(&id)?;
                Some(json!({
                    "candidate": id,
                    "platform": text::untrusted(id, &candidate.platform),
                    "votes": votes,
                }))
            })
            .collect();
        out.insert("ballot".into(), json!(list));
    }
    Value::Object(out)
}

/// Returns one bill that waits for the resident, with the verb to use.
fn bill_entry(realm: &Realm, bill: &Bill, verb: &str) -> Value {
    let mut out = entry(bill.gov);
    out.insert("bill".into(), json!(bill.id));
    out.insert("param".into(), json!(bill.param));
    out.insert("value".into(), json!(bill.value));
    out.insert("now".into(), json!(bill.old));
    out.insert("stage".into(), json!(bill.stage.name()));
    out.insert("minutes_left".into(), json!(calendar::minutes_left(realm, bill.deadline)));
    out.insert("use".into(), json!(verb));
    Value::Object(out)
}

/// Builds the section, or nothing when the module has nothing to say.
fn section(realm: &Realm, p: &Params, resident: ResidentId, opts: &ObserveOpts) -> Option<Value> {
    let state = realm.res::<Politics>();
    let layout = realm.layout();
    let city =
        realm.directory().zone_of(resident).and_then(|zone| layout.zone(zone)).map(|z| z.city);
    // The governments whose elections the resident can take part in now.
    let here = |gov: Gov| gov == Gov::Nation || (city.is_some() && gov.city() == city);
    let full = opts.is_full();
    let max = if full { usize::MAX } else { params::count(realm, p.brief_list_max) };
    let mut out = Map::new();

    let mut next = Map::new();
    let city_vote = city.and_then(|c| state.next_vote.get(&OrgId::city(c)));
    if let Some(&day) = city_vote {
        next.insert("city".into(), calendar::date(realm, day));
    }
    if let Some(&day) = state.next_vote.get(&OrgId::NATION) {
        next.insert("nation".into(), calendar::date(realm, day));
    }
    if !next.is_empty() {
        out.insert("next_vote".into(), Value::Object(next));
    }

    let elections: Vec<Value> = state
        .elections
        .values()
        .filter(|election| here(election.gov))
        .map(|election| election_entry(realm, election, resident, full))
        .collect();
    put(&mut out, "elections", elections, max);

    let recalls: Vec<Value> = state
        .recalls
        .values()
        .filter(|recall| here(recall.gov))
        .map(|recall| {
            let mut item = entry(recall.gov);
            item.insert("recall".into(), json!(recall.id));
            item.insert("office".into(), json!(recall.office.name()));
            item.insert("signatures".into(), json!(recall.signatures()));
            item.insert("needed".into(), json!(recall.needed));
            item.insert("signed".into(), json!(recall.signers.contains(&resident)));
            item.insert(
                "minutes_left".into(),
                json!(calendar::minutes_left(realm, recall.deadline)),
            );
            Value::Object(item)
        })
        .collect();
    put(&mut out, "recalls", recalls, max);

    let own: Vec<Value> = realm
        .res::<Orgs>()
        .of_member(resident)
        .filter_map(|org| {
            let office = offices::seat_name(org.role_of(resident)?)?;
            // The central banker is a post of the nation.
            let gov = Gov::of_org(org.id()).unwrap_or(Gov::Nation);
            let mut item = entry(gov);
            item.insert("office".into(), json!(office));
            Some(Value::Object(item))
        })
        .collect();
    put(&mut out, "offices", own, usize::MAX);

    let candidacies: Vec<Value> = state
        .elections
        .values()
        .filter_map(|election| {
            let candidate = election.candidates.get(&resident)?;
            let mut item = entry(election.gov);
            item.insert("election".into(), json!(election.id));
            item.insert("office".into(), json!(election.office.name()));
            item.insert("phase".into(), json!(election.phase.name()));
            item.insert("fund".into(), json!(fund::balance(realm, candidate)));
            Some(Value::Object(item))
        })
        .collect();
    put(&mut out, "candidacies", candidacies, usize::MAX);

    let bills: Vec<Value> = state
        .bills
        .values()
        .filter_map(|bill| {
            let org = bill.gov.org();
            match bill.stage {
                Stage::Vote | Stage::Override
                    if !bill.has_voted(resident)
                        && bill.admits(resident)
                        && offices::can(realm, org, resident, GOV_VOTE) =>
                {
                    Some(bill_entry(realm, bill, "vote_law"))
                }
                Stage::Sign if offices::can(realm, org, resident, GOV_SIGN) => {
                    Some(bill_entry(realm, bill, "sign_law or veto_law"))
                }
                _ => None,
            }
        })
        .collect();
    put(&mut out, "bills", bills, max);

    (!out.is_empty()).then_some(Value::Object(out))
}

/// Registers the observer.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.observe_global("politics", ORDER_SECTION, move |realm, resident, opts| {
        section(realm, &p, resident, opts)
    });
}
