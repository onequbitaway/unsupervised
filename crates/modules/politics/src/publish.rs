//! Public state for viewers. See "State for viewers" in
//! `docs/design/game-design.md`, section 9.
//!
//! The module publishes three series. The spectator service keeps the
//! newest event of each kind and key.
//!
//! | Kind | Key | Content |
//! | --- | --- | --- |
//! | `politics.government_published` | city number, or `nation` | Offices and posts with their holders, open elections, open petitions |
//! | `politics.election_published` | election id | Candidates with their funds and their votes, the phase, the result |
//! | `politics.bills_published` | city number, or `nation` | Open bills with their counts, the last closed bills, the laws in force |
//!
//! A series is published when it changed, at most one time in
//! `politics.publish_minutes` game minutes, and once more each game day.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, EventMeta, Holder, Realm, ResidentId, Scope, Stage, Value};

use crate::calendar;
use crate::election::{facts as election_facts, facts_with as election_facts_with, fund};
use crate::law::{ballot, reason_of};
use crate::model::Gov;
use crate::offices::{self, GOV_VOTE};
use crate::params::{self, Params};
use crate::state::Politics;
use crate::state::bill::{Bill, Stage as BillStage};
use crate::state::election::{Election, Phase};
use crate::text;

/// Order of the publication among the systems of the last stage.
const ORDER_PUBLISH: i32 = 0;

/// Returns the name of a resident, as the directory has it.
fn name_of(realm: &Realm, resident: ResidentId) -> Option<String> {
    realm.directory().get(resident).map(|entry| entry.name.clone())
}

/// Returns a resident with its name.
fn person(realm: &Realm, resident: ResidentId) -> Value {
    json!({ "resident": resident, "name": name_of(realm, resident) })
}

/// Makes the description of a publication.
fn published(realm: &Realm, p: &Params, kind: &'static str) -> EventMeta {
    EventMeta::public(kind, params::drama(realm, p.drama_routine))
}

/// Publishes the offices, the posts, the open elections and the open
/// petitions of one government.
fn government(realm: &Realm, p: &Params, state: &Politics, gov: Gov) {
    let org = gov.org();
    let seats: Vec<Value> = offices::seats(realm, p, gov)
        .into_iter()
        .map(|seat| {
            let holders: Vec<Value> = offices::holders(realm, seat.org, seat.role)
                .into_iter()
                .map(|holder| person(realm, holder))
                .collect();
            json!({
                "office": seat.name,
                "org": seat.org,
                "role": seat.role,
                "elected": seat.elected,
                "seats": seat.seats,
                "salary": seat.salary,
                "holders": holders,
            })
        })
        .collect();
    let elections: Vec<Value> = state
        .elections
        .values()
        .filter(|election| election.gov == gov)
        .map(|election| {
            json!({
                "election": election.id,
                "office": election.office.name(),
                "kind": election.cause.name(),
                "phase": election.phase.name(),
                "vote_day": calendar::date(realm, election.vote_day),
            })
        })
        .collect();
    let recalls: Vec<Value> = state
        .recalls
        .values()
        .filter(|recall| recall.gov == gov)
        .map(|recall| {
            json!({
                "recall": recall.id,
                "office": recall.office.name(),
                "starter": recall.starter,
                "started": recall.started,
                "signatures": recall.signatures(),
                "needed": recall.needed,
                "deadline": calendar::moment(realm, recall.deadline),
            })
        })
        .collect();
    let next_vote = state.next_vote.get(&org).map(|&day| calendar::date(realm, day));
    let treasury = realm.ledger().money(Holder::Org(org));
    realm.emit(
        published(realm, p, "politics.government_published"),
        &gov.describe_with(json!({
            "key": gov.key(),
            "org": org,
            "treasury": treasury,
            "next_vote": next_vote,
            "offices": seats,
            "elections": elections,
            "recalls": recalls,
        })),
    );
}

/// Returns the candidates of an election for viewers, the leader first.
fn candidates(realm: &Realm, election: &Election) -> Vec<Value> {
    election
        .standings()
        .into_iter()
        .filter_map(|(id, votes)| {
            let candidate = election.candidates.get(&id)?;
            Some(json!({
                "candidate": id,
                "name": name_of(realm, id),
                "platform": text::untrusted(id, &candidate.platform),
                "raised": candidate.raised,
                "spent": candidate.spent,
                "fund": fund::balance(realm, candidate),
                "votes": votes,
            }))
        })
        .collect()
}

/// Returns the state of an election for viewers. `phase` is the phase to
/// show.
fn election_view(realm: &Realm, election: &Election, phase: &str) -> Value {
    election_facts_with(
        election,
        json!({
            "key": election.id,
            "phase": phase,
            "seats": election.seats,
            "vote_day": calendar::date(realm, election.vote_day),
            "opens": election.opens,
            "campaign_from": election.campaign_from,
            "polls_open": election.polls_open,
            "polls_close": election.polls_close,
            "candidates": candidates(realm, election),
            "turnout": election.voters.len(),
        }),
    )
}

/// Publishes an open election. On the voting day it also tells the world
/// when another candidate took the lead.
fn election(realm: &Realm, p: &Params, election: &mut Election) {
    if election.phase == Phase::Voting {
        let leader = election.leading();
        if let Some(leader) = leader.filter(|&leader| election.leader != Some(leader)) {
            let standings = election.standings();
            let votes = standings.first().map_or(0, |&(_, votes)| votes);
            let behind = standings.get(1).map(|&(_, votes)| votes);
            let mut facts = election_facts(election);
            facts.insert("leader".into(), json!(leader));
            facts.insert("votes".into(), json!(votes));
            facts.insert("second".into(), json!(behind));
            facts.insert("previous".into(), json!(election.leader));
            realm.emit(
                EventMeta::public(
                    "politics.lead_changed",
                    params::drama(realm, p.drama_lead_changed),
                )
                .actor(leader),
                &Value::Object(facts),
            );
            election.leader = Some(leader);
        }
    }
    realm.emit(
        published(realm, p, "politics.election_published"),
        &election_view(realm, election, election.phase.name()),
    );
}

/// Publishes the last state of an election: the phase `decided`, the
/// final counts and the winners. The count calls it at once, so the result
/// does not wait for the next publication.
pub(crate) fn election_decided(
    realm: &Realm,
    p: &Params,
    election: &Election,
    winners: &[ResidentId],
    tie: bool,
) {
    let mut view = election_view(realm, election, "decided");
    view["winners"] = json!(winners);
    view["tie"] = json!(tie);
    realm.emit(published(realm, p, "politics.election_published"), &view);
}

/// Returns a bill for viewers. An open bill shows the votes of the
/// residents that are members now.
fn bill_view(realm: &Realm, bill: &Bill) -> Value {
    let (yes, no) = if bill.stage.takes_votes() {
        ballot::tally(bill, &offices::members_with(realm, bill.gov.org(), GOV_VOTE))
    } else {
        (bill.yes.len(), bill.no.len())
    };
    let (how, why) = match bill.stage {
        BillStage::Enacted(how) => (Some(how.name()), None),
        BillStage::Rejected(why) => (None, Some(why.name())),
        _ => (None, None),
    };
    json!({
        "bill": bill.id,
        "param": bill.param,
        "value": bill.value,
        "old": bill.old,
        "sponsor": bill.sponsor,
        "reason": reason_of(bill),
        "proposed": bill.proposed,
        "stage": bill.stage.name(),
        "deadline": bill.deadline,
        "yes": yes,
        "no": no,
        "passed": bill.passed.map(|(yes, no)| json!({ "yes": yes, "no": no })),
        "vetoed_by": bill.vetoed_by,
        "how": how,
        "why": why,
    })
}

/// Publishes the register of one government: the open bills, the last
/// closed bills and the laws in force.
fn bills(realm: &Realm, p: &Params, state: &Politics, gov: Gov) {
    let org = gov.org();
    let open: Vec<Value> = state.bills_of(gov).map(|bill| bill_view(realm, bill)).collect();
    let closed: Vec<Value> = state
        .closed
        .get(&org)
        .map(|register| register.iter().rev().map(|bill| bill_view(realm, bill)).collect())
        .unwrap_or_default();
    let laws: Vec<Value> = state
        .laws
        .get(&org)
        .map(|laws| {
            laws.iter()
                .map(|(param, law)| {
                    json!({
                        "param": param,
                        "value": law.value,
                        "old": law.old,
                        "bill": law.bill,
                        "enacted": law.enacted,
                        "how": law.how.name(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    realm.emit(
        published(realm, p, "politics.bills_published"),
        &gov.describe_with(json!({
            "key": gov.key(),
            "open": open,
            "closed": closed,
            "laws": laws,
        })),
    );
}

/// Publishes what changed. A global system calls it every frame, after
/// the other systems of the module.
fn run(realm: &Realm, p: &Params) {
    let frame = realm.frame();
    let today = calendar::today(realm);
    let mut state = realm.res_mut::<Politics>();
    if state.refresh_day < today {
        // Once each day everything is published again, so that a viewer
        // service that started late has all of it.
        state.refresh_day = today;
        for gov in offices::governments(realm) {
            state.touch_government(gov);
            state.touch_bills(gov);
        }
        let open: Vec<u64> = state.elections.keys().copied().collect();
        for id in open {
            state.touch_election(id);
        }
    }
    if state.dirty.is_empty() || frame < state.next_publish {
        return;
    }
    let dirty = std::mem::take(&mut state.dirty);
    let wait = u64::try_from(realm.param(p.publish_minutes).max(1)).unwrap_or(1);
    state.next_publish = frame + wait;
    for gov in dirty.governments.into_iter().filter_map(Gov::of_org) {
        government(realm, p, &state, gov);
    }
    for id in dirty.elections {
        if let Some(open) = state.elections.get_mut(&id) {
            election(realm, p, open);
        }
    }
    for gov in dirty.bills.into_iter().filter_map(Gov::of_org) {
        bills(realm, p, &state, gov);
    }
}

/// Registers the system.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.system(Stage::Post, ORDER_PUBLISH, "politics.publish", move |realm| run(realm, &p));
    }
}
