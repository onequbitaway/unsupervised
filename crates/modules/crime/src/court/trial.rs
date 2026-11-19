//! Trials: the charge, the judge, the jury and the verdict.
//!
//! Each charge is a trial with a deadline. Nothing here waits for an
//! agent without a limit:
//!
//! 1. A resident with `law.judge` in the city can rule at any time before
//!    the verdict (`rule_case`).
//! 2. When the government of the city has no judge, the engine calls a
//!    jury at once. When it has one, the engine calls the jury after
//!    `crime.trial_bench_ticks` without a ruling. The jurors vote with
//!    `jury_vote`.
//! 3. At the deadline the engine decides: by the votes when at least
//!    `crime.jury_quorum` jurors voted, else by the evidence threshold
//!    `crime.convict_evidence`.

use unsup_kernel::serde_json::json;
use unsup_kernel::{CityId, Dest, EventMeta, Holder, OrgId, Realm, ResidentId};
use unsup_society::Orgs;

use super::intake;
use super::registry::{Cases, Decider, Juror, Sentence, Status, Timer, Trial};
use crate::ids::Handles;
use crate::msg::{Order, Verdict};
use crate::names::{event, notice, perm};

/// Returns true when a member of the government of the city, or of the
/// nation, can judge. A judge of another organization with a grant can
/// rule too, but the engine does not wait for it.
fn has_judge(realm: &Realm, city: CityId) -> bool {
    let orgs = realm.res::<Orgs>();
    [OrgId::city(city), OrgId::NATION].into_iter().any(|id| {
        orgs.get(id)
            .is_some_and(|org| org.members().any(|(member, _)| org.can(member, perm::JUDGE)))
    })
}

/// Starts the trial of a case. `officer` is `None` when the engine
/// charges.
pub(crate) fn open(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    id: u64,
    officer: Option<ResidentId>,
) {
    let frame = realm.frame();
    let Some(case) = cases.cases.get_mut(&id) else {
        return;
    };
    let (perp, kind, city, pos) = (case.perp, case.kind, case.city, case.pos);
    let length = h.frames(realm, h.p.trial_ticks).max(1);
    let deadline = frame.saturating_add(length);
    case.status = Status::Trial;
    case.trial =
        Some(Trial { opened: frame, deadline, officer, summoned: false, jury: Vec::new() });
    if let Some(hold) = cases.holds.get_mut(&perp) {
        hold.trials.push(id);
    }
    realm.send(Dest::Resident(perp), &Order::Charged { case: id, kind, deadline });
    realm.emit(
        EventMeta::public(event::CHARGED, h.drama(realm, h.p.drama_charged)).actor(perp).at(pos),
        &json!({
            "case": id, "kind": kind, "city": city, "target": perp, "officer": officer,
            "decided_in": length,
        }),
    );
    cases.at(deadline, Timer::TrialEnd { case: id });
    let bench = h.frames(realm, h.p.trial_bench_ticks);
    if bench > 0 && bench < length && has_judge(realm, city) {
        cases.at(frame.saturating_add(bench), Timer::Bench { case: id });
    } else {
        summon(realm, h, cases, id);
    }
}

/// Charges every case of a resident in custody that waits for a charge
/// and has evidence at or above the warrant threshold. The engine does
/// this when no officer did. Returns the number of trials that started.
pub(crate) fn charge_ready(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    resident: ResidentId,
) -> usize {
    let mut started = 0;
    for id in cases.open_of(resident) {
        let ready = cases.cases.get(&id).is_some_and(|case| {
            case.status == Status::Held
                && case.evidence.total() >= realm.param_in(h.p.warrant_evidence, case.city)
        });
        if ready {
            open(realm, h, cases, id, None);
            started += 1;
        }
    }
    started
}

/// Calls the jury of a trial: up to `crime.jury_size` residents of the
/// city, drawn with the random stream of the global realm from the
/// residents that live (`Directory::sample_alive`). The defendant, the
/// victim, the officer that charged, residents of the owner of the
/// defendant and residents under arrest do not sit on the jury.
pub(crate) fn summon(realm: &Realm, h: &Handles, cases: &mut Cases, id: u64) {
    let Some(case) = cases.cases.get(&id) else {
        return;
    };
    let Some(trial) = &case.trial else {
        return;
    };
    if trial.summoned || case.status != Status::Trial {
        return;
    }
    let (perp, kind, city, deadline, officer) =
        (case.perp, case.kind, case.city, trial.deadline, trial.officer);
    let victim = match case.victim {
        Some(Holder::Resident(victim)) => Some(victim),
        _ => None,
    };
    let size = h.count(realm, h.p.jury_size);
    let mut jury: Vec<Juror> = Vec::with_capacity(size);
    {
        let directory = realm.directory();
        let owner = directory.get(perp).map(|entry| entry.owner);
        let mut rng = realm.rng();
        for _ in 0..h.count(realm, h.p.jury_draws) {
            if jury.len() >= size {
                break;
            }
            // Each draw names a resident that lives now. The directory
            // knows the ids; nothing here depends on how the kernel gives
            // them out.
            let Some(pick) = directory.sample_alive(&mut rng) else {
                break;
            };
            let Some(entry) = directory.get(pick) else {
                continue;
            };
            let in_city = realm.layout().zone(entry.zone).is_some_and(|zone| zone.city == city);
            let barred = pick == perp
                || Some(pick) == victim
                || Some(pick) == officer
                || Some(entry.owner) == owner
                || cases.holds.contains_key(&pick)
                || jury.iter().any(|juror| juror.resident == pick);
            if in_city && !barred {
                jury.push(Juror { resident: pick, vote: None });
            }
        }
    }
    for juror in &jury {
        realm.send(
            Dest::Resident(juror.resident),
            &Order::Jury { case: id, kind, deadline, on: true },
        );
    }
    let jurors = jury.len();
    if let Some(trial) = cases.cases.get_mut(&id).and_then(|case| case.trial.as_mut()) {
        trial.summoned = true;
        trial.jury = jury;
    }
    realm.emit(
        EventMeta::public(event::JURY, h.drama(realm, h.p.drama_jury)),
        &json!({ "case": id, "kind": kind, "city": city, "jurors": jurors }),
    );
}

/// What a judge decided.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ruling {
    pub(crate) judge: ResidentId,
    pub(crate) guilty: bool,
    pub(crate) fine: i64,
    pub(crate) jail_ticks: i64,
    pub(crate) restitution: i64,
}

/// Returns the sentence that the engine gives for a case: the fine and the
/// jail time of the kind of crime, and the loss of the victim as
/// restitution.
pub(crate) fn engine_sentence(
    realm: &Realm,
    h: &Handles,
    cases: &Cases,
    id: u64,
) -> (i64, i64, i64) {
    let Some(case) = cases.cases.get(&id) else {
        return (0, 0, 0);
    };
    let kind = h.kind(case.kind);
    let restitution = if case.victim.is_some() { case.value.max(0) } else { 0 };
    (
        realm.param_in(kind.fine, case.city).max(0),
        realm.param_in(kind.jail, case.city).max(0),
        restitution,
    )
}

/// Ends the trial of a case. With a ruling, the judge decided. Without
/// one, the engine decides: by the votes of the jury when enough jurors
/// voted, else by the evidence.
pub(crate) fn conclude(
    realm: &Realm,
    h: &Handles,
    cases: &mut Cases,
    id: u64,
    ruling: Option<Ruling>,
) {
    let frame = realm.frame();
    let Some(case) = cases.cases.get(&id) else {
        return;
    };
    let Some(trial) = &case.trial else {
        return;
    };
    if case.status != Status::Trial {
        return;
    }
    let (perp, kind, city, zone, victim) =
        (case.perp, case.kind, case.city, case.zone, case.victim);
    let (yes, no) = trial.tally();
    let waiting: Vec<ResidentId> =
        trial.jury.iter().filter(|j| j.vote.is_none()).map(|j| j.resident).collect();
    let (fine, jail, restitution) = engine_sentence(realm, h, cases, id);
    let sentence = match ruling {
        Some(r) => Sentence {
            guilty: r.guilty,
            fine: r.fine,
            jail_ticks: r.jail_ticks,
            restitution: r.restitution,
            by: Decider::Judge,
            judge: Some(r.judge),
            frame,
        },
        None => {
            let quorum = h.count(realm, h.p.jury_quorum).max(1);
            let (guilty, by) = if yes + no >= quorum {
                (yes > no, Decider::Jury)
            } else {
                let needed = realm.param_in(h.p.convict_evidence, city);
                (case.evidence.total() >= needed, Decider::Evidence)
            };
            let part = |amount: i64| if guilty { amount } else { 0 };
            Sentence {
                guilty,
                fine: part(fine),
                jail_ticks: part(jail),
                restitution: part(restitution),
                by,
                judge: None,
                frame,
            }
        }
    };
    if let Some(case) = cases.cases.get_mut(&id) {
        case.sentence = Some(sentence);
    }
    let status = if sentence.guilty { Status::Convicted } else { Status::Acquitted };
    cases.close(id, status, frame);
    cases.at(frame.saturating_add(h.frames(realm, h.p.case_keep)), Timer::Prune { case: id });

    // Custody goes on while another trial runs. Before it ends, the
    // engine charges what an officer left without a charge.
    let mut last = true;
    if let Some(hold) = cases.holds.get_mut(&perp) {
        hold.trials.retain(|other| *other != id);
        if hold.trials.is_empty() && charge_ready(realm, h, cases, perp) == 0 {
            cases.holds.remove(&perp);
            intake::reopen(realm, h, cases, perp);
        } else {
            last = false;
        }
    }
    let due = frame.saturating_add(h.frames(realm, h.p.fine_due_ticks));
    realm.send(
        Dest::Resident(perp),
        &Order::Verdict(Verdict {
            case: id,
            kind,
            guilty: sentence.guilty,
            fine: sentence.fine,
            jail_ticks: sentence.jail_ticks,
            restitution: sentence.restitution,
            victim,
            zone,
            city,
            due,
            last,
        }),
    );
    if sentence.guilty && sentence.fine.saturating_add(sentence.restitution) > 0 {
        cases.at(due, Timer::FineDue { resident: perp, case: id });
    }
    for juror in waiting {
        realm.send(
            Dest::Resident(juror),
            &Order::Jury { case: id, kind, deadline: frame, on: false },
        );
    }
    if let Some(Holder::Resident(victim)) = victim {
        realm.notify(
            victim,
            notice::VERDICT,
            None,
            json!({
                "case": id, "kind": kind, "guilty": sentence.guilty, "defendant": perp,
                "restitution": sentence.restitution,
            }),
        );
    }
    // The verdict names the defendant.
    realm.emit(
        EventMeta::public(event::VERDICT, h.drama(realm, h.p.drama_verdict)).actor(perp),
        &json!({
            "case": id, "kind": kind, "city": city, "target": perp,
            "guilty": sentence.guilty, "by": sentence.by.name(), "judge": sentence.judge,
            "fine": sentence.fine, "jail_ticks": sentence.jail_ticks,
            "restitution": sentence.restitution,
            "votes_guilty": yes, "votes_not_guilty": no,
        }),
    );
}
