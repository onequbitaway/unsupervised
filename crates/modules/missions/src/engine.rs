//! The mission engine. It counts events, pays a taker when its objectives
//! are met and closes a mission at its deadline. Nobody can stop it, because
//! the whole reward is in escrow from the start.

use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Event, EventMeta, Holder, Money, OwnerId, Realm, ResidentId, Value};

use crate::board;
use crate::model::{Mission, Missions, Poster, Receipt};
use crate::params::{self, Params};
use crate::pay::{self, REFUND, REWARD};
use crate::{text, view};

/// Adds a new mission, whose reward is in its escrow, and tells the world.
/// `request` is the number that a sponsor gave to its request.
pub(crate) fn open(
    realm: &Realm,
    p: &Params,
    state: &mut Missions,
    mission: Mission,
    request: Option<u64>,
) {
    let mut data = Map::new();
    data.insert("mission".into(), json!(mission.id));
    data.insert("title".into(), text::untrusted(mission.author(), &mission.def.title));
    data.insert("reward".into(), json!(mission.def.reward));
    data.insert("takers".into(), json!(mission.def.takers));
    data.insert("mode".into(), json!(view::mode(&mission)));
    if let Some(org) = mission.def.org {
        data.insert("org".into(), json!(org));
    }
    data.insert("deadline".into(), json!(mission.deadline));
    data.insert("objectives".into(), json!(mission.def.objectives.len()));
    let meta = match &mission.poster {
        Poster::Resident { id, .. } => {
            data.insert("poster".into(), json!(id));
            EventMeta::public("missions.mission_posted", params::drama(realm, p.drama_posted))
                .actor(*id)
        }
        Poster::Sponsor { name } => {
            data.insert("sponsor".into(), text::untrusted(mission.author(), name));
            if let Some(request) = request {
                data.insert("request".into(), json!(request));
            }
            EventMeta::public("missions.mission_posted", params::drama(realm, p.drama_sponsored))
        }
    };
    state.insert(mission);
    realm.emit(meta, &Value::Object(data));
}

/// Handles an event that missions can count: adds it to the progress of
/// every mission that the actor works on, and pays each taker whose
/// objectives are now met.
pub(crate) fn on_event(realm: &Realm, p: &Params, event: &Event) {
    let Some(actor) = event.actor else {
        return;
    };
    let mut state = realm.res_mut::<Missions>();
    let Some(list) = state.enrolled.get(&actor) else {
        return;
    };
    let list: Vec<(u64, u64)> = list.iter().map(|(&mission, &key)| (mission, key)).collect();
    for (id, key) in list {
        let Some(Mission { def, takers, .. }) = state.open.get_mut(&id) else {
            continue;
        };
        let Some(taker) = takers.get_mut(&key) else {
            continue;
        };
        let mut counted = 0u32;
        for (objective, done) in def.objectives.iter().zip(taker.progress.iter_mut()) {
            if *done < objective.count
                && objective.event == event.kind
                && (objective.team || taker.assignee == actor)
                && objective
                    .fields
                    .iter()
                    .all(|(name, want)| want.matches(&event.data[name.as_str()]))
            {
                *done += 1;
                counted += 1;
            }
        }
        if counted == 0 {
            continue;
        }
        let events = taker.members.entry(actor).or_default();
        *events = events.saturating_add(counted);
        if taker.is_complete(&def.objectives) {
            complete(realm, p, &mut state, id, key);
        }
    }
}

/// Sends each share from the escrow and writes down what really moved. A
/// wire sends at most what the escrow holds, so a share can arrive short;
/// the receipt then says by how much.
fn pay_shares(realm: &Realm, escrow: Holder, owed: &[(ResidentId, i64)], frame: u64) -> Receipt {
    let mut receipt = Receipt { frame, shares: Vec::with_capacity(owed.len()), short: Vec::new() };
    for &(resident, cents) in owed {
        let moved = pay::wire_to_resident(realm, escrow, resident, cents, REWARD);
        receipt.shares.push((resident, moved));
        if moved < cents {
            receipt.short.push((resident, cents - moved));
        }
    }
    receipt
}

/// Pays a taker whose objectives are met: the whole reward to a solo taker,
/// or a share to each member of a team that added events. Then the mission
/// closes when no place is left.
///
/// The receipt, the notices and the event carry the cents that the wires
/// moved. When that is less than the reward, they also carry `short`: the
/// cents that are missing.
fn complete(realm: &Realm, p: &Params, state: &mut Missions, id: u64, key: u64) {
    let now = realm.frame();
    let Some(mission) = state.open.get_mut(&id) else {
        return;
    };
    let escrow = mission.escrow();
    let reward = mission.def.reward;
    let team = mission.def.team;
    let poster = mission.poster.resident();
    let poster_owner: Option<OwnerId> = match mission.poster {
        Poster::Resident { owner, .. } => Some(owner),
        Poster::Sponsor { .. } => None,
    };
    let Some(taker) = mission.takers.get_mut(&key) else {
        return;
    };
    let assignee = taker.assignee;
    let members: Vec<(ResidentId, u32)> = taker.members.iter().map(|(&r, &e)| (r, e)).collect();
    let owed = if team { pay::split(reward, &members) } else { vec![(assignee, reward)] };
    let receipt = pay_shares(realm, escrow, &owed, now);
    taker.done = Some(receipt.clone());
    let finished = mission.is_full() && mission.active() == 0;
    for &(member, _) in &members {
        state.unenrol(member, id);
    }

    // Each member that earned a share also earns points, also when the
    // escrow paid the share short. A member that added no event gets the
    // news and nothing else.
    for &(member, _) in &members {
        let earned = owed.iter().any(|(who, _)| *who == member);
        let points = if earned { board::credit_points(realm, p, member, poster_owner) } else { 0 };
        let mut news = Map::new();
        news.insert("mission".into(), json!(id));
        news.insert("paid".into(), json!(receipt.paid_to(member)));
        news.insert("points".into(), json!(points));
        view::insert_short(&mut news, receipt.short_of(member));
        realm.notify(member, "missions.mission_completed", None, Value::Object(news));
    }
    let mut data = Map::new();
    data.insert("mission".into(), json!(id));
    data.insert("resident".into(), json!(assignee));
    if team {
        data.insert("org".into(), json!(key));
    }
    data.insert("paid".into(), json!(receipt.paid()));
    let list: Vec<Value> = receipt
        .shares
        .iter()
        .map(|(who, cents)| json!({ "resident": who, "amount": cents }))
        .collect();
    data.insert("shares".into(), Value::Array(list));
    view::insert_short(&mut data, receipt.missing());
    if let Some(poster) = poster {
        realm.notify(poster, "missions.mission_paid", Some(assignee), Value::Object(data.clone()));
    }
    realm.emit(
        EventMeta::public("missions.mission_completed", params::drama(realm, p.drama_completed))
            .actor(assignee),
        &Value::Object(data),
    );
    if finished {
        close(realm, p, state, id, "completed");
    }
}

/// Gives the unpaid reward of a mission of a resident back: to the poster,
/// or to its heir when the poster left the world. Returns the cents sent.
fn refund(realm: &Realm, state: &Missions, from: Holder, poster: ResidentId, cents: i64) -> i64 {
    if realm.directory().is_alive(poster) {
        return pay::wire_to_resident(realm, from, poster, cents, REFUND);
    }
    match state.heirs.get(&poster).copied() {
        Some(Holder::Resident(heir)) => pay::wire_to_resident(realm, from, heir, cents, REFUND),
        // An organization, an account and an escrow are in this ledger.
        Some(heir @ (Holder::Org(_) | Holder::Account(_) | Holder::Escrow(_))) => {
            pay::transfer(realm, from, heir, cents, REFUND)
        }
        // No heir that the global realm can reach: the kernel gives a wire
        // for a resident that is gone to the unclaimed account.
        Some(Holder::Site(_)) | None => pay::wire_to_resident(realm, from, poster, cents, REFUND),
    }
}

/// Closes a mission: the takers that still work on it get the news, and the
/// unpaid reward goes back to the poster or, for a sponsor mission, is
/// destroyed through the sink `sponsor.expired`. Returns the cents that
/// left the escrow.
///
/// `reason` is `expired` (the deadline), `completed` (no place is left) or
/// `withdrawn` (the poster took it back).
pub(crate) fn close(realm: &Realm, p: &Params, state: &mut Missions, id: u64, reason: &str) -> i64 {
    let Some(mission) = state.remove(id) else {
        return 0;
    };
    let escrow = mission.escrow();
    let rest = realm.ledger().money(escrow).cents();
    let (refunded, destroyed) = match &mission.poster {
        Poster::Resident { id: poster, .. } => (refund(realm, state, escrow, *poster, rest), 0),
        Poster::Sponsor { .. } => {
            let burned = realm.ledger_mut().burn(escrow, p.sponsor_sink, Money(rest.max(0)));
            (0, if burned.is_ok() { rest.max(0) } else { 0 })
        }
    };
    for taker in mission.takers.values().filter(|taker| taker.done.is_none()) {
        for &member in taker.members.keys() {
            realm.notify(
                member,
                "missions.mission_ended",
                None,
                json!({ "mission": id, "reason": reason }),
            );
        }
    }
    let completed = mission.completed();
    if let Some(poster) = mission.poster.resident() {
        state.settle_heir(poster);
        if reason != "withdrawn" {
            realm.notify(
                poster,
                "missions.mission_closed",
                None,
                json!({ "mission": id, "reason": reason, "completed": completed, "refunded": refunded }),
            );
        }
    }
    realm.emit(
        EventMeta::public("missions.mission_closed", params::drama(realm, p.drama_closed)),
        &json!({
            "mission": id,
            "reason": reason,
            "completed": completed,
            "refunded": refunded,
            "destroyed": destroyed,
        }),
    );
    refunded + destroyed
}

/// Runs once in every tick of the global realm: closes every mission whose
/// deadline is here. The events of the zones of this frame came before.
pub(crate) fn tick(realm: &Realm, p: &Params) {
    let now = realm.frame();
    let mut state = realm.res_mut::<Missions>();
    while let Some(&(deadline, id)) = state.wake.first() {
        if deadline > now {
            break;
        }
        state.wake.remove(&(deadline, id));
        close(realm, p, &mut state, id, "expired");
    }
}
