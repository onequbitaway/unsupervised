//! The clerk of the registry: everything that happens at a deadline, and
//! what happens when a resident leaves the world.
//!
//! All deadlines count frames. One frame is one game minute of the
//! calendar. The local tick of a zone does not count, so a slow zone does
//! not move a deadline.

use unsup_kernel::{Dest, Realm, ResidentId};

use super::registry::{Cases, Status, Timer};
use super::{bounty, intake, trial};
use crate::ids::Handles;
use crate::msg::Order;

/// The time that a suspect can be held without a charge is over. The
/// engine charges what has enough evidence. With nothing to charge, the
/// suspect goes free.
fn hold_over(realm: &Realm, h: &Handles, cases: &mut Cases, resident: ResidentId, since: u64) {
    let waits = cases
        .holds
        .get(&resident)
        .is_some_and(|hold| hold.since == since && hold.trials.is_empty());
    if waits && trial::charge_ready(realm, h, cases, resident) == 0 {
        intake::end_hold(realm, h, cases, resident, "hold_expired", None);
    }
}

/// A case is too old. Without a charge it goes cold and its warrant ends.
fn expire(realm: &Realm, h: &Handles, cases: &mut Cases, id: u64) {
    let frame = realm.frame();
    let Some(case) = cases.cases.get(&id) else {
        return;
    };
    let keep = h.frames(realm, h.p.case_keep).max(1);
    match case.status {
        Status::Open | Status::Wanted => {
            if case.status == Status::Wanted {
                let order = Order::Warrant { case: id, kind: case.kind, on: false };
                realm.send(Dest::Resident(case.perp), &order);
            }
            cases.close(id, Status::Cold, frame);
            cases.at(frame.saturating_add(keep), Timer::Prune { case: id });
        }
        // The suspect is in custody. Look again later.
        Status::Held | Status::Trial => {
            cases.at(frame.saturating_add(keep), Timer::Expire { case: id });
        }
        _ => {}
    }
}

/// Does everything that is due in this frame.
pub(crate) fn run(realm: &Realm, h: &Handles) {
    let frame = realm.frame();
    if realm.res::<Cases>().timers.first_key_value().is_none_or(|(due, _)| *due > frame) {
        return;
    }
    let mut cases = realm.res_mut::<Cases>();
    for timer in cases.take_due(frame) {
        match timer {
            Timer::Hold { resident, since } => hold_over(realm, h, &mut cases, resident, since),
            Timer::Bench { case } => trial::summon(realm, h, &mut cases, case),
            Timer::TrialEnd { case } => trial::conclude(realm, h, &mut cases, case, None),
            Timer::FineDue { resident, case } => {
                realm.send(Dest::Resident(resident), &Order::Due { case });
            }
            Timer::BountyEnd { bounty } => bounty::expire(realm, h, &mut cases, bounty, "expired"),
            Timer::Expire { case } => expire(realm, h, &mut cases, case),
            Timer::Prune { case } => cases.prune(case),
        }
    }
}

/// A resident left the world. Its cases are moot, its custody ends and
/// the bounties on it go back.
pub(crate) fn on_despawn(realm: &Realm, h: &Handles, resident: ResidentId) {
    let frame = realm.frame();
    let mut cases = realm.res_mut::<Cases>();
    let keep = h.frames(realm, h.p.case_keep).max(1);
    for id in cases.open_of(resident) {
        // The jurors of a trial that ends here have no duty any more.
        let jurors: Vec<(ResidentId, _)> = cases
            .cases
            .get(&id)
            .and_then(|case| case.trial.as_ref().map(|trial| (case.kind, trial)))
            .map(|(kind, trial)| {
                trial
                    .jury
                    .iter()
                    .filter(|juror| juror.vote.is_none())
                    .map(|juror| (juror.resident, kind))
                    .collect()
            })
            .unwrap_or_default();
        for (juror, kind) in jurors {
            let order = Order::Jury { case: id, kind, deadline: frame, on: false };
            realm.send(Dest::Resident(juror), &order);
        }
        cases.close(id, Status::Moot, frame);
        cases.at(frame.saturating_add(keep), Timer::Prune { case: id });
    }
    cases.holds.remove(&resident);
    let on_target: Vec<u64> =
        cases.bounties.values().filter(|b| b.target == resident).map(|b| b.id).collect();
    for id in on_target {
        bounty::expire(realm, h, &mut cases, id, "target_gone");
    }
}
