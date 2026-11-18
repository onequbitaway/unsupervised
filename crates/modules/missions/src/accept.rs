//! `mission_accept` and `mission_abandon`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{ActionError, Builder, EventMeta, GlobalCtx, OrgId, Realm, ResidentId, Value};

use crate::model::{Missions, Taker};
use crate::params::{self, Params};
use crate::remote::{Call, Origin, remote_verb};
use crate::{lifecycle, spec, view};

/// Arguments of `mission_accept`.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
pub(crate) struct AcceptArgs {
    /// Id of the mission.
    pub(crate) mission: u64,
    /// For a team mission: the organization that you accept for. You must
    /// be a member. Do not give it for a solo mission.
    #[serde(default)]
    pub(crate) org: Option<OrgId>,
}

/// Arguments of the verbs that name only a mission.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
pub(crate) struct MissionArgs {
    /// Id of the mission.
    pub(crate) mission: u64,
}

/// The global part of `mission_accept`.
#[derive(Serialize, Deserialize)]
struct Accept {
    mission: u64,
    org: Option<OrgId>,
}

impl Call for Accept {
    const KIND: &'static str = "missions.accept";
}

/// The error for a mission id that the engine does not have.
pub(crate) fn no_mission(id: u64) -> ActionError {
    ActionError::not_found(format!("No open mission has the id {id}."))
}

fn accept(realm: &Realm, p: &Params, origin: &Origin, call: &Accept) -> Result<Value, ActionError> {
    let me = origin.resident;
    let now = realm.frame();
    let mut state = realm.res_mut::<Missions>();
    let enrolled = state.enrolled_count(me);
    let already = state.key_of(me, call.mission).is_some();
    let Some(mission) = state.open.get_mut(&call.mission) else {
        return Err(no_mission(call.mission));
    };
    if mission.poster.resident() == Some(me) {
        return Err(ActionError::not_allowed("You cannot accept your own mission."));
    }
    let key = match (mission.def.team, call.org) {
        (false, None) => me.0,
        (false, Some(_)) => {
            return Err(ActionError::invalid_args("This is a solo mission. Do not give `org`."));
        }
        (true, None) => {
            return Err(ActionError::invalid_args(
                "This is a team mission. Give `org`: the organization that you accept for.",
            ));
        }
        (true, Some(org)) => {
            if let Some(only) = mission.def.org
                && only != org
            {
                return Err(ActionError::not_allowed(format!(
                    "Only organization {only} can take this mission."
                )));
            }
            org.0
        }
    };
    if now >= mission.deadline {
        return Err(ActionError::conflict("The mission ends in this frame."));
    }
    if already {
        return Err(ActionError::conflict("You work on this mission already."));
    }
    let limit = params::count(realm, p.accepted_max);
    if enrolled >= limit {
        return Err(ActionError::conflict(format!(
            "You work on too many missions. The limit is {limit}."
        )));
    }
    let team = mission.def.team;
    let full = mission.is_full();
    let objectives = mission.def.objectives.len();
    let members = match mission.takers.get_mut(&key) {
        Some(taker) if taker.done.is_some() => {
            return Err(ActionError::conflict(if team {
                "Your organization completed this mission already."
            } else {
                "You completed this mission already."
            }));
        }
        Some(taker) => {
            let limit = params::count(realm, p.team_members_max);
            if taker.members.len() >= limit {
                return Err(ActionError::conflict(format!(
                    "The team is full. The limit is {limit} members."
                )));
            }
            taker.members.insert(me, 0);
            taker.members.len()
        }
        None => {
            if full {
                return Err(ActionError::conflict("The mission has no free place."));
            }
            let taker = Taker {
                assignee: me,
                accepted: now,
                progress: vec![0; objectives],
                members: [(me, 0)].into(),
                done: None,
            };
            mission.takers.insert(key, taker);
            1
        }
    };
    let deadline = mission.deadline;
    state.enrol(me, call.mission, key);
    let mut data = Map::new();
    data.insert("mission".into(), json!(call.mission));
    data.insert("resident".into(), json!(me));
    if team {
        data.insert("org".into(), json!(key));
        data.insert("members".into(), json!(members));
    }
    realm.emit(
        EventMeta::public("missions.mission_accepted", params::drama(realm, p.drama_accepted))
            .actor(me),
        &Value::Object(data.clone()),
    );
    data.remove("resident");
    data.insert("status".into(), json!("accepted"));
    data.insert("deadline".into(), json!(deadline));
    if let Some(mission) = state.open.get(&call.mission) {
        data.insert("objectives".into(), view::objectives(mission));
    }
    Ok(Value::Object(data))
}

fn abandon(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    args: &MissionArgs,
) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Missions>();
    if !state.is_open(args.mission) {
        return Err(no_mission(args.mission));
    }
    let key = state.key_of(resident, args.mission);
    let team = state.open.get(&args.mission).is_some_and(|mission| mission.def.team);
    if !lifecycle::leave(&mut state, args.mission, resident) {
        return Err(ActionError::conflict("You do not work on this mission."));
    }
    let mut data = Map::new();
    data.insert("mission".into(), json!(args.mission));
    data.insert("resident".into(), json!(resident));
    if let (true, Some(org)) = (team, key) {
        data.insert("org".into(), json!(org));
    }
    realm.emit(
        EventMeta::public("missions.mission_abandoned", params::drama(realm, p.drama_accepted))
            .actor(resident),
        &Value::Object(data),
    );
    Ok(json!({ "mission": args.mission, "status": "abandoned" }))
}

/// Registers the verbs.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<AcceptArgs, Accept>(
        b,
        spec(
            "mission_accept",
            "Take a mission. The engine then counts your events and pays you by itself when the objectives are met. For a team mission, give your organization.",
        ),
        p,
        |realm, ctx, args| {
            // Only the zone knows the memberships of the resident.
            if let Some(org) = args.org
                && !realm.memberships(ctx.idx).iter().any(|m| m.org == org)
            {
                return Err(ActionError::not_allowed(format!(
                    "You are not a member of organization {org}."
                )));
            }
            Ok((Accept { mission: args.mission, org: args.org }, 0))
        },
        move |realm, origin, call| accept(realm, &p, origin, &call),
    );
    b.global_verb::<MissionArgs>(
        spec(
            "mission_abandon",
            "Give up a mission that you work on. Your place is free again, and a solo taker loses its progress.",
        ),
        move |realm, ctx: &GlobalCtx, args| abandon(realm, &p, ctx.resident, args),
    );
}
