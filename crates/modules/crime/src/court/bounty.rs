//! Bounties: money in escrow for the officer that arrests a resident.
//!
//! The contract engine of the society module pays only a party that
//! signed the contract. A bounty has no such party: whoever makes the
//! arrest earns it. So the registry keeps its own escrow,
//! `Holder::Escrow(bounty id)` in the global realm, and pays it when the
//! event `crime.arrest_made` names the target and an officer.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, Builder, Dest, Event, EventMeta, Holder, Money, Realm, ResidentId, Scope, Value,
};

use super::registry::{Bounty, Cases, Timer};
use crate::ids::Handles;
use crate::names::{event, label, notice};
use crate::remote::{Call, Origin, remote_verb};
use crate::verbs::spec;

/// Arguments of `post_bounty`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PostBountyArgs {
    /// The resident whose arrest earns the money.
    pub target: ResidentId,
    /// Cents of your cash to lock. The officer that arrests the target
    /// gets them.
    pub amount: i64,
    /// Ticks until the money comes back to you when nobody makes the
    /// arrest. One tick is one game minute. Without it, the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
}

/// The global part of `post_bounty`.
#[derive(Serialize, Deserialize)]
struct Post {
    target: ResidentId,
    amount: i64,
    frames: u64,
}

impl Call for Post {
    const KIND: &'static str = "crime.post_bounty";
}

/// Sends money of the global realm to a resident, wherever it is. The
/// kernel gives a wire for a resident that no longer exists to the
/// unclaimed account, so no money is lost.
fn pay_out(realm: &Realm, from: Holder, to: ResidentId) -> i64 {
    let amount = realm.ledger().money(from);
    if !amount.is_positive() {
        return 0;
    }
    match realm.wire_money(from, Dest::Resident(to), Holder::Resident(to), amount, label::BOUNTY) {
        Ok(()) => amount.cents(),
        Err(_) => 0,
    }
}

/// Ends a bounty without an arrest. The money goes back to the poster.
pub(crate) fn expire(realm: &Realm, h: &Handles, cases: &mut Cases, id: u64, reason: &str) {
    let Some(bounty) = cases.bounties.remove(&id) else {
        return;
    };
    let back = pay_out(realm, Holder::Escrow(id), bounty.poster);
    realm.notify(
        bounty.poster,
        notice::BOUNTY,
        None,
        json!({ "bounty": id, "target": bounty.target, "refunded": back, "reason": reason }),
    );
    realm.emit(
        EventMeta::public(event::BOUNTY_EXPIRED, h.drama(realm, h.p.drama_bounty_expired)),
        &json!({ "bounty": id, "target": bounty.target, "amount": back, "reason": reason }),
    );
}

/// An arrest happened. Every bounty on the target goes to the officer.
/// The city watch earns nothing: the bounty stays open.
fn on_arrest(realm: &Realm, h: &Handles, arrest: &Event) {
    let id = |name: &str| arrest.data.get(name).and_then(Value::as_u64).map(ResidentId);
    let (Some(target), Some(officer)) = (id("target"), id("officer")) else {
        return;
    };
    let mut cases = realm.res_mut::<Cases>();
    let earned: Vec<Bounty> =
        cases.bounties.values().filter(|bounty| bounty.target == target).copied().collect();
    for bounty in earned {
        cases.bounties.remove(&bounty.id);
        let paid = pay_out(realm, Holder::Escrow(bounty.id), officer);
        let data = json!({
            "bounty": bounty.id, "target": target, "officer": officer, "amount": paid,
        });
        realm.notify(officer, notice::BOUNTY, None, data.clone());
        realm.notify(bounty.poster, notice::BOUNTY, None, data.clone());
        realm.emit(
            EventMeta::public(event::BOUNTY_PAID, h.drama(realm, h.p.drama_bounty_paid))
                .actor(officer),
            &data,
        );
    }
}

/// The global part of `post_bounty`: the registry takes the money out of
/// the holding escrow and opens the bounty.
fn post(realm: &Realm, h: &Handles, origin: &Origin, call: &Post) -> Result<Value, ActionError> {
    if !realm.directory().is_alive(call.target) {
        return Err(ActionError::not_found(format!("No resident has the id {}.", call.target)));
    }
    let mut cases = realm.res_mut::<Cases>();
    let open = cases.bounties.values().filter(|b| b.poster == origin.resident).count();
    if open >= h.count(realm, h.p.bounties_open_max) {
        return Err(ActionError::conflict("You have too many open bounties."));
    }
    let id = realm.alloc_id();
    realm.ledger_mut().transfer_money(
        origin.escrow(),
        Holder::Escrow(id),
        Money(call.amount),
        label::BOUNTY,
    )?;
    let expires = realm.frame().saturating_add(call.frames);
    cases.bounties.insert(
        id,
        Bounty { id, poster: origin.resident, target: call.target, amount: call.amount, expires },
    );
    cases.at(expires, Timer::BountyEnd { bounty: id });
    realm.emit(
        EventMeta::public(event::BOUNTY_POSTED, h.drama(realm, h.p.drama_bounty_posted))
            .actor(origin.resident),
        &json!({
            "bounty": id, "by": origin.resident, "target": call.target,
            "amount": call.amount, "open_for": call.frames,
        }),
    );
    Ok(json!({
        "bounty": id, "target": call.target, "amount": call.amount, "open_for": call.frames,
    }))
}

/// Registers `post_bounty` and the payment on an arrest.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Global {
        b.on_event(event::ARREST, move |realm, arrest| on_arrest(realm, &h, arrest));
    }
    remote_verb::<PostBountyArgs, Post>(
        b,
        h,
        spec(
            "post_bounty",
            "Lock cash for the officer that arrests a resident. An arrest needs grounds, so a bounty pays only for a suspect that the police can lawfully take. The money comes back when the time is over.",
        ),
        move |realm, ctx, args| {
            if args.target == ctx.resident {
                return Err(ActionError::invalid_args("The target must be another resident."));
            }
            let least = realm.param(h.p.bounty_min).max(1);
            if args.amount < least {
                return Err(ActionError::invalid_args(format!(
                    "`amount` must be at least {least}."
                )));
            }
            let most = h.ticks(realm, h.p.bounty_max_ticks);
            let ticks = args.ticks.unwrap_or_else(|| h.ticks(realm, h.p.bounty_ticks).min(most));
            if !(1..=most).contains(&ticks) {
                return Err(ActionError::invalid_args(format!(
                    "`ticks` must be from 1 to {most}."
                )));
            }
            let cash = realm.ledger().money(Holder::Resident(ctx.resident)).cents();
            if cash < args.amount {
                return Err(ActionError::insufficient("You do not have that much cash.")
                    .with_details(json!({ "needed": args.amount, "available": cash })));
            }
            let call = Post { target: args.target, amount: args.amount, frames: u64::from(ticks) };
            Ok((call, args.amount))
        },
        move |realm, origin, call| post(realm, &h, origin, &call),
    );
}
