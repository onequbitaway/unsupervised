//! The checks that turn the definition that a user wrote into a definition
//! of the engine. Every field is checked and every size is bounded.
//!
//! The checks read only parameters, so they run in a zone (for
//! `mission_post`, before any cash moves) and in the global realm (for a
//! sponsor mission).

use std::collections::BTreeMap;

use unsup_kernel::{ActionError, Realm, Value};

use crate::model::{Definition, Objective, Scalar};
use crate::params::{self, Params};
use crate::spec::{ActorRule, MissionSpec, Mode, ObjectiveSpec};
use crate::text;

/// The events that an objective can count, as prefixes of the event kind:
/// the events of every game module. The global realm gets each event with
/// one of these prefixes from every zone.
///
/// The prefixes do not overlap, so the engine sees each event one time.
/// Kernel events and the speech events of the society module are left out
/// on purpose.
pub(crate) const WATCHED: [&str; 12] = [
    "life.",
    "economy.",
    "finance.",
    "crime.",
    "politics.",
    "missions.",
    "society.org_",
    "society.contract_",
    "society.married",
    "society.divorced",
    "society.household_",
    "society.opinion_",
];

fn bad(message: impl Into<String>) -> ActionError {
    ActionError::invalid_args(message)
}

/// Checks that a number is in a range of two parameters or constants.
fn in_range(value: i64, min: i64, max: i64, what: &str, unit: &str) -> Result<(), ActionError> {
    if value < min || value > max {
        return Err(bad(format!("`{what}` must be from {min} to {max}{unit}.")));
    }
    Ok(())
}

/// Checks that text has no line break.
fn one_line(text: &str, what: &str) -> Result<(), ActionError> {
    if text.contains('\n') {
        return Err(bad(format!("The {what} has a line break.")));
    }
    Ok(())
}

fn fields(
    realm: &Realm,
    p: &Params,
    fields: &BTreeMap<String, Value>,
) -> Result<Vec<(String, Scalar)>, ActionError> {
    let max = params::count(realm, p.match_max);
    if fields.len() > max {
        return Err(bad(format!("An objective compares at most {max} data fields.")));
    }
    let name_max = params::count(realm, p.field_name_max);
    let text_max = params::count(realm, p.field_text_max);
    fields
        .iter()
        .map(|(name, value)| {
            if !text::is_ident(name, name_max) {
                return Err(bad(format!("`{name}` is not the name of a data field.")));
            }
            let scalar = match value {
                Value::Bool(v) => Scalar::Bool(*v),
                Value::Number(v) => Scalar::Int(v.as_i64().ok_or_else(|| {
                    bad(format!("The value of `{name}` must be a whole number."))
                })?),
                Value::String(v) if !v.is_empty() && text::is_token(v, text_max) => {
                    Scalar::Text(v.clone())
                }
                _ => {
                    return Err(bad(format!(
                        "The value of `{name}` must be a whole number, a boolean or a short name without spaces."
                    )));
                }
            };
            Ok((name.clone(), scalar))
        })
        .collect()
}

fn objective(
    realm: &Realm,
    p: &Params,
    spec: &ObjectiveSpec,
    team: bool,
) -> Result<Objective, ActionError> {
    let kind_max = params::count(realm, p.event_kind_max);
    if !text::is_event_kind(&spec.event, kind_max)
        || !WATCHED.iter().any(|prefix| spec.event.starts_with(prefix))
    {
        return Err(bad(format!(
            "`{}` is not an event that a mission can count. Use a kind that starts with one of: {}.",
            spec.event,
            WATCHED.join(", ")
        )));
    }
    in_range(i64::from(spec.count), 1, realm.param(p.count_max), "count", "")?;
    if spec.actor == ActorRule::Team && !team {
        return Err(bad("Only a team mission can count the events of team members."));
    }
    Ok(Objective {
        event: spec.event.clone(),
        fields: fields(realm, p, &spec.fields)?,
        count: spec.count,
        team: spec.actor == ActorRule::Team,
    })
}

/// Checks the definition of a mission and returns it in the form of the
/// engine.
pub(crate) fn definition(
    realm: &Realm,
    p: &Params,
    spec: &MissionSpec,
) -> Result<Definition, ActionError> {
    text::check_text(&spec.title, realm.param(p.title_max), true, "title")?;
    one_line(&spec.title, "title")?;
    text::check_text(&spec.text, realm.param(p.text_max), false, "text")?;
    let max = params::count(realm, p.objectives_max);
    if spec.objectives.is_empty() || spec.objectives.len() > max {
        return Err(bad(format!("A mission has from 1 to {max} objectives.")));
    }
    let team = spec.mode == Mode::Team;
    if spec.org.is_some() && !team {
        return Err(bad("Only a team mission can name an organization."));
    }
    let objectives = spec
        .objectives
        .iter()
        .map(|spec| objective(realm, p, spec, team))
        .collect::<Result<Vec<_>, _>>()?;
    in_range(
        spec.reward,
        realm.param(p.reward_min),
        realm.param(p.reward_max),
        "reward",
        " cents",
    )?;
    in_range(i64::from(spec.takers), 1, realm.param(p.takers_max), "takers", "")?;
    in_range(
        i64::from(spec.deadline_in),
        realm.param(p.deadline_min),
        realm.param(p.deadline_max),
        "deadline_in",
        " game minutes",
    )?;
    if spec.reward.checked_mul(i64::from(spec.takers)).is_none() {
        return Err(bad("The reward for all takers is too large."));
    }
    Ok(Definition {
        title: spec.title.clone(),
        text: spec.text.clone(),
        objectives,
        team,
        org: spec.org,
        reward: spec.reward,
        takers: spec.takers,
        deadline_in: spec.deadline_in,
    })
}

/// Checks the name that a sponsor gave.
pub(crate) fn sponsor_name(realm: &Realm, p: &Params, name: &str) -> Result<(), ActionError> {
    text::check_text(name, realm.param(p.sponsor_name_max), true, "name of the sponsor")?;
    one_line(name, "name of the sponsor")
}
