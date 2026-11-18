//! Recall: the verbs `start_recall` and `sign_recall`.
//!
//! A petition asks for an early election of one office. Residents of the
//! city sign it, or residents of the whole nation for a national office.
//! With enough signatures the early election opens at once. The holders
//! stay in office until its count.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, EventMeta, Realm, Scope, Stage, Value};

use crate::calendar::{self, DAY};
use crate::election::schedule::{self, Opening};
use crate::model::{Gov, Office};
use crate::offices;
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb, require_age, spec};
use crate::state::Politics;
use crate::state::election::Cause;
use crate::state::recall::{self, Recall};

/// Order of the system that ends petitions, among the systems of the
/// module.
const ORDER_RECALLS: i32 = 20;

/// Arguments of `start_recall`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct StartArgs {
    /// The office: `mayor` or `council` of the city in which you are,
    /// `president` or `legislature` of the nation.
    pub(crate) office: Office,
}

/// The global part of `start_recall`.
#[derive(Serialize, Deserialize)]
struct Start {
    office: Office,
}

impl Call for Start {
    const KIND: &'static str = "politics.start_recall";
}

/// Arguments of `sign_recall`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
pub(crate) struct SignArgs {
    /// Id of the petition. The `politics` section of your observation
    /// lists the open petitions. For a city office you must be in that
    /// city.
    pub(crate) recall: u64,
}

/// The global part of `sign_recall`.
#[derive(Serialize, Deserialize)]
struct Sign {
    recall: u64,
}

impl Call for Sign {
    const KIND: &'static str = "politics.sign_recall";
}

/// Returns the fields that name a petition in an event, a notice or a
/// result.
fn facts_with(recall: &Recall, more: Value) -> Value {
    let mut out = recall.gov.describe();
    out.insert("recall".into(), json!(recall.id));
    out.insert("office".into(), json!(recall.office.name()));
    out.insert("signatures".into(), json!(recall.signatures()));
    out.insert("needed".into(), json!(recall.needed));
    if let Value::Object(more) = more {
        out.extend(more);
    }
    Value::Object(out)
}

/// Returns the number of residents that are in the city of a city
/// government now, or in the world for the nation.
fn residents_of(realm: &Realm, gov: Gov) -> u64 {
    let directory = realm.directory();
    match gov {
        Gov::Nation => directory.living(),
        Gov::City(city) => {
            let layout = realm.layout();
            let here = |zone| layout.zone(zone).is_some_and(|info| info.city == city);
            directory.iter().filter(|(_, entry)| entry.alive && here(entry.zone)).count() as u64
        }
    }
}

/// Ends a petition that has its signatures: the early election opens, when
/// the regular election does not come first. Returns the id of the early
/// election.
fn succeed(realm: &Realm, p: &Params, state: &mut Politics, id: u64) -> Option<u64> {
    let recall = state.recalls.remove(&id)?;
    let (gov, office) = (recall.gov, recall.office);
    let vote_day = calendar::early_vote_day(realm, p, gov, calendar::today(realm));
    let regular_first = state.next_vote.get(&gov.org()).is_some_and(|&next| next <= vote_day);
    let election = if regular_first || state.election_for(gov, office).is_some() {
        None
    } else {
        // The registration opens now, not at the start of a day.
        let mut windows = calendar::windows(realm, p, gov, vote_day);
        windows.opens = realm.frame();
        let cause = Cause::Recall;
        Some(schedule::open(realm, p, state, Opening { gov, office, cause, vote_day, windows }))
    };
    let facts = facts_with(
        &recall,
        json!({
            "election": election,
            "vote_day": election.map(|_| calendar::date(realm, vote_day)),
        }),
    );
    let role = offices::office_role(office);
    for holder in offices::holders(realm, gov.org(), role) {
        realm.notify(holder, "politics.recall_succeeded", None, facts.clone());
    }
    realm.emit(
        EventMeta::public(
            "politics.recall_succeeded",
            params::drama(realm, p.drama_recall_succeeded),
        ),
        &facts,
    );
    state.touch_government(gov);
    election
}

fn start(realm: &Realm, p: &Params, origin: &Origin, call: Start) -> Result<Value, ActionError> {
    let office = call.office;
    let gov = if office.is_national() { Gov::Nation } else { Gov::City(origin.city) };
    let mut state = realm.res_mut::<Politics>();
    if let Some(open) = state.recall_for(gov, office) {
        return Err(ActionError::conflict(format!(
            "Petition {} against this office is open. Sign it with `sign_recall`.",
            open.id
        ))
        .with_details(json!({ "recall": open.id })));
    }
    if state.election_for(gov, office).is_some() {
        return Err(ActionError::conflict("An election for this office is open."));
    }
    // A petition must be able to end, and its early election to be held,
    // before the regular election.
    let days = params::amount(realm, p.recall_days, gov).max(1);
    let today = calendar::today(realm);
    let last_early = calendar::early_vote_day(realm, p, gov, today.saturating_add(days));
    if let Some(&next) = state.next_vote.get(&gov.org()).filter(|&&next| next <= last_early) {
        return Err(ActionError::conflict(
            "The regular election for this office is near. A recall cannot start now.",
        )
        .with_details(json!({ "regular_vote_day": calendar::date(realm, next) })));
    }
    let needed = recall::needed(
        residents_of(realm, gov),
        params::value(realm, p.recall_permille, gov),
        params::value(realm, p.recall_signatures_min, gov),
    );
    let frame = realm.frame();
    let petition = Recall {
        id: realm.alloc_id(),
        gov,
        office,
        starter: origin.resident,
        started: frame,
        deadline: frame.saturating_add(days.saturating_mul(DAY)),
        needed,
        signers: BTreeSet::from([origin.resident]),
    };
    let id = petition.id;
    let holders = offices::holders(realm, gov.org(), offices::office_role(office));
    let facts = facts_with(
        &petition,
        json!({
            "starter": origin.resident,
            "holders": holders,
            "deadline": calendar::moment(realm, petition.deadline),
        }),
    );
    for holder in holders {
        realm.notify(holder, "politics.recall_started", Some(origin.resident), facts.clone());
    }
    realm.emit(
        EventMeta::public("politics.recall_started", params::drama(realm, p.drama_recall_started))
            .actor(origin.resident)
            .at(origin.pos),
        &facts,
    );
    let succeeded = petition.signatures() >= needed;
    let mut result = facts_with(&petition, json!({ "deadline": petition.deadline }));
    state.recalls.insert(id, petition);
    state.touch_government(gov);
    let election = if succeeded { succeed(realm, p, &mut state, id) } else { None };
    result["succeeded"] = json!(succeeded);
    result["election"] = json!(election);
    Ok(result)
}

fn sign(realm: &Realm, p: &Params, origin: &Origin, call: Sign) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Politics>();
    let Some(petition) = state.recalls.get_mut(&call.recall) else {
        return Err(ActionError::not_found(format!(
            "No open recall petition has the id {}.",
            call.recall
        )));
    };
    if let Gov::City(city) = petition.gov
        && city != origin.city
    {
        return Err(ActionError::not_here(format!(
            "This petition is for city {city}. You are in city {}.",
            origin.city
        )));
    }
    if !petition.signers.insert(origin.resident) {
        return Err(ActionError::conflict("You signed this petition already."));
    }
    let gov = petition.gov;
    let succeeded = petition.signatures() >= petition.needed;
    let mut result = facts_with(petition, json!({ "succeeded": succeeded }));
    state.touch_government(gov);
    let election = if succeeded { succeed(realm, p, &mut state, call.recall) } else { None };
    result["election"] = json!(election);
    Ok(result)
}

/// Ends the petitions whose time is over without enough signatures. A
/// global system calls it every frame.
fn expire(realm: &Realm, p: &Params) {
    let frame = realm.frame();
    let mut state = realm.res_mut::<Politics>();
    let late: Vec<u64> =
        state.recalls.values().filter(|r| frame >= r.deadline).map(|r| r.id).collect();
    for id in late {
        let Some(petition) = state.recalls.remove(&id) else {
            continue;
        };
        realm.emit(
            EventMeta::public(
                "politics.recall_failed",
                params::drama(realm, p.drama_recall_failed),
            ),
            &facts_with(&petition, json!({ "starter": petition.starter })),
        );
        state.touch_government(petition.gov);
    }
}

/// Registers the verbs and the system.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<StartArgs, Start>(
        b,
        spec(
            "start_recall",
            "Start a petition for an early election of an office. Your signature is the first.",
        ),
        p,
        move |realm, ctx, args| {
            require_age(realm, &p, ctx)?;
            Ok((Start { office: args.office }, 0))
        },
        move |realm, origin, call| start(realm, &p, origin, call),
    );
    remote_verb::<SignArgs, Sign>(
        b,
        spec(
            "sign_recall",
            "Sign a recall petition. Enough signatures open an early election for the office.",
        ),
        p,
        move |realm, ctx, args| {
            require_age(realm, &p, ctx)?;
            Ok((Sign { recall: args.recall }, 0))
        },
        move |realm, origin, call| sign(realm, &p, origin, call),
    );
    if b.scope() == Scope::Global {
        b.system(Stage::Update, ORDER_RECALLS, "politics.recalls", move |realm| expire(realm, &p));
    }
}
