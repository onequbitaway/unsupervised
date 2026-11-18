//! The zone carries out the orders of the case registry.
//!
//! The registry of the global realm decides: a warrant, a charge, a
//! verdict, the end of the time to pay. The zone of the resident does the
//! work at once and by itself: it sets and clears conditions, moves the
//! resident to the jail, takes the money and changes the reputation. No
//! resident has to agree.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Builder, Holder, Idx, OrgId, Realm, Scope};

use crate::custody;
use crate::debts;
use crate::ids::Handles;
use crate::jail;
use crate::kinds::CrimeKind;
use crate::msg::{Order, Verdict};
use crate::names::notice;
use crate::record::{Debt, Docket, Known, Record, Role, Warrant};

/// Brings the condition `wanted` in line with the record: a resident is
/// wanted while a warrant names it and it is not under arrest.
pub(crate) fn sync_wanted(realm: &Realm, h: &Handles, idx: Idx) {
    let named = !realm.col::<Record>()[idx].warrants.is_empty();
    let wanted = named && !realm.has_condition(idx, h.detained);
    if wanted && !realm.has_condition(idx, h.wanted) {
        realm.set_condition(idx, h.wanted, None);
    } else if !wanted {
        realm.clear_condition(idx, h.wanted);
    }
}

/// A warrant names the resident, or a warrant ended.
fn warrant(realm: &Realm, h: &Handles, idx: Idx, case: u64, kind: CrimeKind, on: bool) {
    let resident = realm.id_of(idx);
    if on {
        realm.col_mut::<Record>()[idx].add_warrant(Warrant { case, kind });
    } else if !realm.col_mut::<Record>()[idx].drop_warrant(case) {
        return;
    }
    sync_wanted(realm, h, idx);
    realm.notify(
        resident,
        notice::WANTED,
        None,
        json!({ "case": case, "kind": kind, "wanted": on }),
    );
}

/// The end of a trial: the zone carries out the sentence.
fn verdict(realm: &Realm, h: &Handles, idx: Idx, v: &Verdict) {
    let resident = realm.id_of(idx);
    {
        let mut records = realm.col_mut::<Record>();
        let record = &mut records[idx];
        record.trials.retain(|trial| trial.case != v.case);
        record.drop_warrant(v.case);
        // The trial is over: what is left of the loot is no evidence any
        // more. A convict pays for it with the restitution.
        record.hot.retain(|lot| lot.case != v.case);
    }
    sync_wanted(realm, h, idx);
    let mut paid = 0;
    let mut until = None;
    if v.guilty {
        let kind = h.kind(v.kind);
        realm.attr_add(idx, h.notoriety, realm.param(kind.notoriety).max(0));
        realm.attr_add(idx, h.standing, -realm.param(h.p.conviction_standing).max(0));
        {
            let mut records = realm.col_mut::<Record>();
            let record = &mut records[idx];
            // The victim comes before the treasury.
            if let Some(victim) = v.victim {
                let zone = matches!(victim, Holder::Site(_)).then_some(v.zone);
                record.add_debt(Debt {
                    case: v.case,
                    to: victim,
                    zone,
                    amount: v.restitution,
                    due: v.due,
                    fine: false,
                });
            }
            record.add_debt(Debt {
                case: v.case,
                to: Holder::Org(OrgId::city(v.city)),
                zone: None,
                amount: v.fine,
                due: v.due,
                fine: true,
            });
        }
        // The court takes what the convict has on it now.
        paid = debts::pay(realm, h, idx, i64::MAX, Some(v.case));
        if v.jail_ticks > 0 {
            until = Some(jail::add_time(realm, h, idx, v.jail_ticks));
        }
    }
    let owed = realm.col::<Record>()[idx].owed();
    let mut data = Map::new();
    data.insert("case".into(), json!(v.case));
    data.insert("kind".into(), json!(v.kind));
    data.insert("guilty".into(), json!(v.guilty));
    data.insert("fine".into(), json!(v.fine));
    data.insert("restitution".into(), json!(v.restitution));
    data.insert("jail_ticks".into(), json!(v.jail_ticks));
    data.insert("paid".into(), json!(paid));
    data.insert("owed".into(), json!(owed));
    if let Some(until) = until {
        data.insert("jailed_until".into(), json!(until));
    }
    realm.notify(resident, notice::VERDICT, None, Value::Object(data));
    if v.last {
        let reason = if v.guilty { "no_jail_time" } else { "acquitted" };
        custody::end(realm, h, idx, reason, None);
    }
}

/// Carries out one order for a resident of this zone.
fn carry_out(realm: &Realm, h: &Handles, idx: Idx, order: Order) {
    let resident = realm.id_of(idx);
    match order {
        Order::Warrant { case, kind, on } => warrant(realm, h, idx, case, kind, on),
        Order::Heard { case, kind, site, value } => {
            let max = h.count(realm, h.p.known_max);
            let known = Known { case, kind, role: Role::Victim, suspect: None, reported: false };
            let news = {
                let mut records = realm.col_mut::<Record>();
                let news = !records[idx].known.iter().any(|k| k.case == case);
                records[idx].add_known(known, max);
                news
            };
            // An owner that was there has its notice already.
            if news {
                realm.notify(
                    resident,
                    notice::WITNESSED,
                    None,
                    json!({
                        "case": case, "kind": kind, "role": Role::Victim.name(), "site": site,
                        "value": value,
                    }),
                );
            }
        }
        Order::Charged { case, kind, deadline } => {
            {
                let mut records = realm.col_mut::<Record>();
                let record = &mut records[idx];
                record.add_trial(Docket { case, kind, deadline });
                record.drop_warrant(case);
            }
            sync_wanted(realm, h, idx);
            realm.notify(
                resident,
                notice::CHARGED,
                None,
                json!({
                    "case": case, "kind": kind,
                    "decided_in": deadline.saturating_sub(realm.frame()),
                }),
            );
        }
        Order::Verdict(v) => verdict(realm, h, idx, &v),
        Order::Due { case } => debts::default_on(realm, h, idx, case),
        Order::Free { reason, by } => custody::end(realm, h, idx, &reason, by),
        Order::Jury { case, kind, deadline, on } => {
            if on {
                realm.col_mut::<Record>()[idx].add_jury(Docket { case, kind, deadline });
                realm.notify(
                    resident,
                    notice::JURY,
                    None,
                    json!({
                        "case": case, "kind": kind,
                        "vote_in": deadline.saturating_sub(realm.frame()),
                    }),
                );
            } else {
                realm.col_mut::<Record>()[idx].jury.retain(|duty| duty.case != case);
            }
        }
        Order::Lesson { xp } => {
            realm.attr_add(idx, h.law, xp.max(0));
        }
    }
}

/// Registers the handler of the orders.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.on_resident_message::<Order>(move |realm, idx, _from, order| {
            carry_out(realm, &h, idx, order);
        });
    }
}
