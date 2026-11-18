//! Reputation: the attribute `rep.standing` and the verb `opine`.
//!
//! The standing of a resident is in its zone. The contract engine and the
//! opinions of other residents change it with a message, so it works for a
//! resident in any zone.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{
    ActionError, AttrDef, AttrId, Builder, Dest, ErrorCode, EventMeta, Idx, Message, OwnerId,
    Realm, ResidentId, Scope, VerbSpec,
};

use crate::params::{self, Params};
use crate::social::{Channel, DAY, Social};
use crate::spec;
use crate::speech::meter::limited;

/// Name of the attribute. See `docs/design/game-design.md`, section 3.
pub const STANDING: &str = "rep.standing";

/// Declares the attribute. The first declaration of a name wins, so the
/// bounds here are those of the game design table.
pub(crate) fn declare(b: &mut Builder) -> AttrId {
    b.attr(AttrDef::new(STANDING).range(-1_000, 1_000).default(0).summary(
        "Public reputation. Kept contracts and praise raise it. Broken contracts and blame lower it.",
    ))
}

/// A change of the standing of a resident, sent to its zone.
#[derive(Serialize, Deserialize)]
struct Standing {
    delta: i64,
    /// True for the reward of a kept contract. The zone counts the rewards
    /// of one day and drops those above the limit.
    reward: bool,
}

impl Message for Standing {
    const KIND: &'static str = "society.standing";
}

/// Changes the standing of a resident, wherever it is. The change arrives
/// in its zone in the next frame.
pub(crate) fn change(realm: &Realm, resident: ResidentId, delta: i64) {
    if delta != 0 {
        realm.send(Dest::Resident(resident), &Standing { delta, reward: false });
    }
}

/// Raises the standing of a resident for a contract that it kept, wherever
/// it is. A resident gets at most `society.rep_contract_rewards_per_day`
/// rewards in one game day, so two residents cannot build a name with many
/// small contracts.
pub(crate) fn reward(realm: &Realm, resident: ResidentId, gain: i64) {
    if gain > 0 {
        realm.send(Dest::Resident(resident), &Standing { delta: gain, reward: true });
    }
}

/// The zone of a resident applies a change of its standing.
fn apply(realm: &Realm, p: &Params, standing: AttrId, idx: Idx, change: &Standing) {
    if change.reward {
        let frame = realm.frame();
        let limit = realm.param(p.rep_contract_rewards_per_day);
        let mut column = realm.col_mut::<Social>();
        let used = column[idx].get().map(|state| state.rewards).unwrap_or_default();
        if !used.has_room(frame, DAY, limit) {
            return;
        }
        column[idx].make().rewards.count(frame, DAY);
    }
    realm.attr_add(idx, standing, change.delta);
}

/// What a resident thinks of another one.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
enum Verdict {
    /// Raise the standing of the resident.
    Praise,
    /// Lower the standing of the resident.
    Condemn,
}

impl Verdict {
    fn name(self) -> &'static str {
        match self {
            Verdict::Praise => "praise",
            Verdict::Condemn => "condemn",
        }
    }
}

/// Arguments of `opine`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
struct OpineArgs {
    /// The resident that you judge.
    resident: ResidentId,
    /// `praise` or `condemn`.
    verdict: Verdict,
}

/// An opinion on its way to the global realm.
#[derive(Serialize, Deserialize)]
struct Opinion {
    from: ResidentId,
    owner: OwnerId,
    about: ResidentId,
    verdict: Verdict,
}

impl Message for Opinion {
    const KIND: &'static str = "society.opinion";
}

/// The global realm checks an opinion and passes it on.
fn deliver(realm: &Realm, p: &Params, opinion: &Opinion) {
    let refuse = |reason: &str| {
        realm.notify(
            opinion.from,
            "society.opinion_refused",
            None,
            json!({ "about": opinion.about, "reason": reason }),
        );
    };
    let owner = match realm.directory().get(opinion.about) {
        Some(entry) if entry.alive => entry.owner,
        _ => return refuse("no_such_resident"),
    };
    // An owner cannot build the name of one resident with another.
    if owner == opinion.owner {
        return refuse("same_owner");
    }
    let weight = realm.param(p.opinion_weight);
    change(realm, opinion.about, if opinion.verdict == Verdict::Praise { weight } else { -weight });
    let facts =
        json!({ "from": opinion.from, "about": opinion.about, "verdict": opinion.verdict.name() });
    realm.notify(opinion.about, "society.opinion_received", Some(opinion.from), facts.clone());
    realm.emit(
        EventMeta::public("society.opinion_given", params::drama(realm, p.drama_opinion))
            .actor(opinion.from),
        &facts,
    );
}

/// Registers the handlers and the verb.
pub(crate) fn build(b: &mut Builder, p: Params, standing: AttrId) {
    match b.scope() {
        Scope::Zone => b.on_resident_message::<Standing>(move |realm, idx, _from, change| {
            apply(realm, &p, standing, idx, &change);
        }),
        Scope::Global => b.on_message::<Opinion>(move |realm, _from, opinion| {
            deliver(realm, &p, &opinion);
        }),
    }
    b.instant_verb::<OpineArgs>(
        VerbSpec {
            free: true,
            ..spec("opine", "Praise or condemn another resident in public. It moves the `rep.standing` of that resident.")
        },
        move |realm, ctx, args| {
            if args.resident == ctx.resident {
                return Err(ActionError::invalid_args("You cannot judge yourself."));
            }
            let frame = realm.frame();
            let min_age = realm.param(p.opinion_min_age).max(0).unsigned_abs();
            if frame.saturating_sub(realm.identity(ctx.idx).born) < min_age {
                return Err(ActionError::not_allowed(format!(
                    "A resident must live {min_age} game minutes before it can judge others."
                )));
            }
            let today = frame / DAY;
            let repeated = realm.col::<Social>()[ctx.idx]
                .get()
                .is_some_and(|state| state.opined.contains(&(args.resident, today)));
            if repeated {
                return Err(ActionError::new(
                    ErrorCode::RateLimited,
                    "You judged this resident today already.",
                ));
            }
            let out = limited(realm, ctx, Channel::Opinion, p.opinions_per_day, || {
                let opinion = Opinion {
                    from: ctx.resident,
                    owner: realm.identity(ctx.idx).owner,
                    about: args.resident,
                    verdict: args.verdict,
                };
                realm.send(Dest::Global, &opinion);
                Ok(json!({ "about": args.resident, "verdict": args.verdict.name() }))
            })?;
            let mut column = realm.col_mut::<Social>();
            let state = column[ctx.idx].make();
            state.opined.retain(|&(_, day)| day == today);
            state.opined.push((args.resident, today));
            Ok(out)
        },
    );
}
