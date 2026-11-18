//! The verbs of the court: `charge`, `rule_case` and `jury_vote`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, CityId, Dest, Realm, ResidentId, Value};

use super::registry::{Cases, Status};
use super::trial::{self, Ruling};
use crate::ids::Handles;
use crate::msg::Order;
use crate::names::perm;
use crate::police::need_power;
use crate::powers;
use crate::remote::{Call, Origin, remote_verb};
use crate::verbs::spec;

/// Arguments of `charge`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ChargeArgs {
    /// The suspect. It must be under arrest in the city where you are.
    pub target: ResidentId,
    /// The case to charge the suspect with.
    pub case: u64,
}

/// The global part of `charge`.
#[derive(Serialize, Deserialize)]
struct Charge {
    target: ResidentId,
    case: u64,
}

impl Call for Charge {
    const KIND: &'static str = "crime.charge";
}

/// The global part of `charge`: the registry checks the custody and the
/// evidence, and starts the trial.
fn charge(
    realm: &Realm,
    h: &Handles,
    origin: &Origin,
    call: &Charge,
) -> Result<Value, ActionError> {
    let mut cases = realm.res_mut::<Cases>();
    if cases.holds.get(&call.target).is_none_or(|hold| hold.city != origin.city) {
        return Err(ActionError::not_found(format!(
            "Resident {} is not under arrest in this city.",
            call.target
        )));
    }
    let Some(case) = cases.cases.get(&call.case).filter(|case| case.city == origin.city) else {
        return Err(ActionError::not_found("No case of this city has this number."));
    };
    if !matches!(case.status, Status::Open | Status::Wanted | Status::Held) {
        return Err(ActionError::conflict(format!(
            "This case is {}. It cannot be charged again.",
            case.status.name()
        )));
    }
    // The answer is the same for a suspect without evidence and for a
    // resident that did not do it, so `charge` does not tell who did it.
    let needed = realm.param_in(h.p.charge_evidence, case.city);
    let against = if case.perp == call.target { case.evidence.total() } else { 0 };
    if against < needed {
        return Err(ActionError::insufficient(
            "The evidence against this resident in this case is below the charge threshold.",
        )
        .with_details(json!({ "needed": needed })));
    }
    trial::open(realm, h, &mut cases, call.case, Some(origin.resident));
    realm.send(Dest::Resident(origin.resident), &Order::Lesson { xp: realm.param(h.p.court_xp) });
    let decided_in = cases
        .cases
        .get(&call.case)
        .and_then(|case| case.trial.as_ref())
        .map_or(0, |trial| trial.deadline.saturating_sub(realm.frame()));
    Ok(json!({ "case": call.case, "target": call.target, "decided_in": decided_in }))
}

/// Arguments of `rule_case`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RuleCaseArgs {
    /// The case on trial.
    pub case: u64,
    /// True to convict, false to acquit.
    pub guilty: bool,
    /// Cents of fine for the city treasury. Without it, the fine of the
    /// kind of crime. At most `crime.<kind>.fine_max`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fine: Option<i64>,
    /// Ticks in jail. Without it, the jail time of the kind of crime. At
    /// most `crime.<kind>.jail_max_ticks`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jail_ticks: Option<i64>,
    /// Cents for the victim. Without it, the loss of the victim. At most
    /// that loss.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restitution: Option<i64>,
}

/// The global part of `rule_case`.
#[derive(Serialize, Deserialize)]
struct Rule {
    case: u64,
    guilty: bool,
    fine: Option<i64>,
    jail_ticks: Option<i64>,
    restitution: Option<i64>,
    /// The cities in which the zone found the judge to hold `law.judge`.
    cities: Vec<CityId>,
}

impl Call for Rule {
    const KIND: &'static str = "crime.rule_case";
}

/// Checks one number of a sentence against its limit.
fn within(name: &str, value: i64, most: i64) -> Result<i64, ActionError> {
    if (0..=most).contains(&value) {
        return Ok(value);
    }
    Err(ActionError::invalid_args(format!("`{name}` must be from 0 to {most}."))
        .with_details(json!({ "max": most })))
}

/// The global part of `rule_case`: the registry checks the trial and the
/// limits of the sentence, and ends the trial.
fn rule(realm: &Realm, h: &Handles, origin: &Origin, call: &Rule) -> Result<Value, ActionError> {
    let mut cases = realm.res_mut::<Cases>();
    let Some(case) = cases.cases.get(&call.case) else {
        return Err(ActionError::not_found("No case has this number."));
    };
    if case.status != Status::Trial {
        return Err(ActionError::conflict(format!(
            "This case is {}. It is not on trial.",
            case.status.name()
        )));
    }
    if !call.cities.contains(&case.city) {
        return Err(ActionError::not_allowed(
            "You need `law.judge` from the government of the city of this case.",
        ));
    }
    if case.perp == origin.resident {
        return Err(ActionError::not_allowed("You cannot judge your own case."));
    }
    let limits = h.kind(case.kind);
    let (fine, jail, restitution) = trial::engine_sentence(realm, h, &cases, call.case);
    let loss = if case.victim.is_some() { case.value.max(0) } else { 0 };
    let ruling = if call.guilty {
        Ruling {
            judge: origin.resident,
            guilty: true,
            fine: within(
                "fine",
                call.fine.unwrap_or(fine),
                realm.param_in(limits.fine_max, case.city).max(0),
            )?,
            jail_ticks: within(
                "jail_ticks",
                call.jail_ticks.unwrap_or(jail),
                realm.param_in(limits.jail_max, case.city).max(0),
            )?,
            restitution: within("restitution", call.restitution.unwrap_or(restitution), loss)?,
        }
    } else {
        Ruling { judge: origin.resident, guilty: false, fine: 0, jail_ticks: 0, restitution: 0 }
    };
    trial::conclude(realm, h, &mut cases, call.case, Some(ruling));
    realm.send(Dest::Resident(origin.resident), &Order::Lesson { xp: realm.param(h.p.court_xp) });
    Ok(json!({
        "case": call.case, "guilty": ruling.guilty, "fine": ruling.fine,
        "jail_ticks": ruling.jail_ticks, "restitution": ruling.restitution,
    }))
}

/// Arguments of `jury_vote`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct JuryVoteArgs {
    /// The case of the trial. The notice `crime.jury_duty` has the number.
    pub case: u64,
    /// True to vote for guilty, false for not guilty.
    pub guilty: bool,
}

/// The verb `jury_vote`: a juror votes. The trial ends when every juror
/// voted.
fn jury_vote(
    realm: &Realm,
    h: &Handles,
    juror: ResidentId,
    args: &JuryVoteArgs,
) -> Result<Value, ActionError> {
    let mut cases = realm.res_mut::<Cases>();
    let Some(case) = cases.cases.get_mut(&args.case) else {
        return Err(ActionError::not_found("No case has this number."));
    };
    let kind = case.kind;
    let on_trial = case.status == Status::Trial;
    let Some(trial) = case.trial.as_mut().filter(|_| on_trial) else {
        return Err(ActionError::conflict("This case is not on trial."));
    };
    let Some(seat) = trial.jury.iter_mut().find(|seat| seat.resident == juror) else {
        return Err(ActionError::not_allowed("You are not a juror of this trial."));
    };
    if seat.vote.is_some() {
        return Err(ActionError::conflict("You voted in this trial already."));
    }
    seat.vote = Some(args.guilty);
    let (cast, jurors) = (trial.votes_cast(), trial.jurors());
    let deadline = trial.deadline;
    realm.send(Dest::Resident(juror), &Order::Jury { case: args.case, kind, deadline, on: false });
    realm.send(Dest::Resident(juror), &Order::Lesson { xp: realm.param(h.p.court_xp) });
    if cast == jurors {
        trial::conclude(realm, h, &mut cases, args.case, None);
    }
    Ok(json!({ "case": args.case, "guilty": args.guilty, "votes": cast, "jurors": jurors }))
}

/// Registers the three verbs.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    remote_verb::<ChargeArgs, Charge>(
        b,
        h,
        spec(
            "charge",
            "Police: charge a suspect that is under arrest in this city with a case. A trial starts. The case needs evidence against the suspect.",
        ),
        |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::ARREST])?;
            Ok((Charge { target: args.target, case: args.case }, 0))
        },
        move |realm, origin, call| charge(realm, &h, origin, &call),
    );
    remote_verb::<RuleCaseArgs, Rule>(
        b,
        h,
        spec(
            "rule_case",
            "Judge: end a trial of your city. Convict with a fine, jail time and restitution inside the limits of the kind of crime, or acquit.",
        ),
        |realm, ctx, args| {
            let cities = powers::cities_with_power(realm, ctx.idx, perm::JUDGE);
            if cities.is_empty() {
                return Err(ActionError::not_allowed(
                    "You need `law.judge` from the government of a city.",
                )
                .with_details(json!({ "permissions": [perm::JUDGE] })));
            }
            for (name, value) in [
                ("fine", args.fine),
                ("jail_ticks", args.jail_ticks),
                ("restitution", args.restitution),
            ] {
                if value.is_some_and(|value| value < 0) {
                    return Err(ActionError::invalid_args(format!(
                        "`{name}` must not be negative."
                    )));
                }
            }
            let call = Rule {
                case: args.case,
                guilty: args.guilty,
                fine: args.fine,
                jail_ticks: args.jail_ticks,
                restitution: args.restitution,
                cities,
            };
            Ok((call, 0))
        },
        move |realm, origin, call| rule(realm, &h, origin, &call),
    );
    b.global_verb::<JuryVoteArgs>(
        spec("jury_vote", "Juror: vote in a trial that you were called for. The trial ends when every juror voted."),
        move |realm, ctx, args| jury_vote(realm, &h, ctx.resident, args),
    );
}
