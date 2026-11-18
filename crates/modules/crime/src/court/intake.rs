//! What the registry does with the reports of the zones: new cases, more
//! evidence, testimony, arrests and releases.

use unsup_kernel::serde_json::json;
use unsup_kernel::{CityId, Dest, EventMeta, Realm, ResidentId};

use super::registry::{Case, Cases, Hold, Status, Timer};
use super::trial;
use crate::ids::Handles;
use crate::msg::{CaseFile, Order, Report, Source};
use crate::names::event;

/// Looks at a case again after its evidence or its suspect changed. A
/// case with evidence at or above `crime.warrant_evidence` gets a warrant.
/// When the suspect is under arrest already, the case waits for a charge
/// instead; the engine charges a suspect of the city watch at once.
///
/// `quiet` is true when the warrant is not new: viewers do not hear of it
/// again.
pub(crate) fn review(realm: &Realm, h: &Handles, cases: &mut Cases, id: u64, quiet: bool) {
    let Some(case) = cases.cases.get(&id) else {
        return;
    };
    if case.status != Status::Open {
        return;
    }
    let (perp, kind, city, pos) = (case.perp, case.kind, case.city, case.pos);
    if case.evidence.total() < realm.param_in(h.p.warrant_evidence, city) {
        return;
    }
    let held_by_watch = cases.holds.get(&perp).map(|hold| hold.officer.is_none());
    let status = if held_by_watch.is_some() { Status::Held } else { Status::Wanted };
    if let Some(case) = cases.cases.get_mut(&id) {
        case.status = status;
    }
    match held_by_watch {
        // The watch has no officer that could charge.
        Some(true) => trial::open(realm, h, cases, id, None),
        Some(false) => {}
        None => {
            realm.send(Dest::Resident(perp), &Order::Warrant { case: id, kind, on: true });
            if !quiet {
                // The event does not name the suspect.
                realm.emit(
                    EventMeta::public(event::WARRANT, h.drama(realm, h.p.drama_warrant)).at(pos),
                    &json!({ "case": id, "kind": kind, "city": city }),
                );
            }
        }
    }
}

/// A new case.
fn opened(realm: &Realm, h: &Handles, cases: &mut Cases, file: &CaseFile) {
    let mut case = Case::new(file);
    if file.caught {
        case.evidence.caught = realm.param(h.p.evidence_caught).max(0);
    }
    if file.watch {
        case.evidence.watch = realm.param(h.p.evidence_watch).max(0);
    }
    cases.insert(case);
    let expires = file.frame.saturating_add(h.frames(realm, h.p.case_ttl));
    cases.at(expires, Timer::Expire { case: file.id });
    review(realm, h, cases, file.id, false);
}

/// More evidence in a case. Evidence also counts during the trial.
fn evidence(realm: &Realm, h: &Handles, cases: &mut Cases, id: u64, source: Source, points: i64) {
    let Some(case) = cases.cases.get_mut(&id) else {
        return;
    };
    if case.status.is_closed() || points <= 0 {
        return;
    }
    let most = match source {
        Source::Investigation => realm.param(h.p.evidence_investigate_max).max(0),
        _ => i64::MAX,
    };
    let slot = case.evidence.slot(source);
    *slot = match source {
        // Several officers and several witnesses add up.
        Source::Investigation | Source::Testimony => slot.saturating_add(points).min(most),
        // The loot is found one time, and a crime is seen one time.
        Source::Search | Source::Caught | Source::Watch => (*slot).max(points),
    };
    review(realm, h, cases, id, false);
}

/// A victim or a witness reported a case. Each resident counts one time.
fn testimony(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    id: u64,
    witness: ResidentId,
    direct: bool,
    identified: bool,
) {
    let Some(case) = cases.cases.get_mut(&id) else {
        return;
    };
    if case.status.is_closed() || case.testified.contains(&witness) {
        return;
    }
    case.testified.push(witness);
    case.reported = true;
    if identified {
        let param = if direct { h.p.evidence_victim } else { h.p.evidence_witness };
        evidence(realm, h, cases, id, Source::Testimony, realm.param(param));
    }
}

/// A resident was arrested. Its warrants are served. The engine charges a
/// suspect of the city watch at once; a suspect of an officer waits for
/// `charge`, at most `crime.hold_ticks`.
fn arrested(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    target: ResidentId,
    officer: Option<ResidentId>,
    city: CityId,
    frame: u64,
) {
    // A suspect that an officer freed during a trial can be arrested
    // again. The trials that still run stay with the new custody.
    let trials = cases.holds.remove(&target).map(|hold| hold.trials).unwrap_or_default();
    cases.holds.insert(target, Hold { officer, city, since: frame, trials });
    for id in cases.open_of(target) {
        if let Some(case) = cases.cases.get_mut(&id)
            && case.status == Status::Wanted
        {
            case.status = Status::Held;
        }
    }
    if officer.is_some() {
        let until = frame.saturating_add(h.frames(realm, h.p.hold_ticks));
        cases.at(until, Timer::Hold { resident: target, since: frame });
    } else if trial::charge_ready(realm, h, cases, target) == 0 {
        end_hold(realm, h, cases, target, "no_case", None);
    }
}

/// Ends the custody of a resident that has no trial. Its cases that
/// waited for a charge are open again, and those with enough evidence get
/// their warrant back.
pub(crate) fn end_hold(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    resident: ResidentId,
    reason: &str,
    by: Option<ResidentId>,
) {
    if cases.holds.remove(&resident).is_none() {
        return;
    }
    realm.send(Dest::Resident(resident), &Order::Free { reason: reason.to_string(), by });
    reopen(realm, h, cases, resident);
}

/// Puts the cases of a resident that waited for a charge back to open.
pub(crate) fn reopen(realm: &Realm, h: &Handles, cases: &mut Cases, resident: ResidentId) {
    for id in cases.open_of(resident) {
        let waited = cases.cases.get_mut(&id).is_some_and(|case| {
            let waited = case.status == Status::Held;
            if waited {
                case.status = Status::Open;
            }
            waited
        });
        if waited {
            review(realm, h, cases, id, true);
        }
    }
}

/// An officer let a suspect go that was not charged. The zone did it
/// already; the registry follows. A suspect that was charged in the same
/// frame stays on trial: the verdict finds it wherever it is.
fn released(realm: &Realm, h: &Handles, cases: &mut Cases, target: ResidentId) {
    let on_trial = cases.holds.get(&target).is_some_and(|hold| !hold.trials.is_empty());
    if on_trial {
        return;
    }
    if cases.holds.remove(&target).is_some() {
        reopen(realm, h, cases, target);
    }
}

/// Handles one report of a zone.
pub(crate) fn on_report(realm: &Realm, h: &Handles, report: Report) {
    let mut cases = realm.res_mut::<Cases>();
    match report {
        Report::Opened(file) => opened(realm, h, &mut cases, &file),
        Report::Evidence { case, source, points } => {
            evidence(realm, h, &mut cases, case, source, points);
        }
        Report::Testimony { case, witness, direct, identified } => {
            testimony(realm, h, &mut cases, case, witness, direct, identified);
        }
        Report::Arrested { target, officer, city, frame } => {
            arrested(realm, h, &mut cases, target, officer, city, frame);
        }
        Report::Released { target } => released(realm, h, &mut cases, target),
    }
}
