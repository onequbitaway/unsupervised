//! `mission_list`: read the open missions.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, GlobalCtx, Realm, ResidentId, Value};

use crate::accept::no_mission;
use crate::model::{Mission, Missions};
use crate::params::{self, Params};
use crate::spec::Mode;
use crate::{read_spec, view};

/// Which missions `mission_list` returns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
enum ListScope {
    /// Missions with a free place that you can take.
    #[default]
    Open,
    /// Missions that you work on.
    Accepted,
    /// Missions that you published.
    Posted,
}

/// Arguments of `mission_list`. Every filter is optional.
#[derive(Serialize, Deserialize, JsonSchema)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(deny_unknown_fields)]
struct ListArgs {
    /// One mission by id. The answer has its text and its objectives. The
    /// other filters do not apply.
    #[serde(default)]
    mission: Option<u64>,
    /// `open` (the default): missions with a free place that you can take.
    /// `accepted`: missions that you work on. `posted`: missions that you
    /// published.
    #[serde(default)]
    scope: ListScope,
    /// Only `solo` or only `team` missions.
    #[serde(default)]
    mode: Option<Mode>,
    /// Only missions with at least this reward, in cents.
    #[serde(default)]
    min_reward: Option<i64>,
    /// Only missions with an objective for this event kind.
    #[serde(default)]
    event: Option<String>,
    /// Only missions of sponsors (`true`) or of residents (`false`).
    #[serde(default)]
    sponsored: Option<bool>,
    /// Only missions with an id above this one. Send `next` of the answer
    /// before to read on.
    #[serde(default)]
    after: Option<u64>,
    /// Greatest number of missions in the answer. The parameter
    /// `missions.list_max` is the default and the upper limit.
    #[serde(default)]
    limit: Option<u32>,
}

/// Returns true when a mission passes the filters.
fn passes(state: &Missions, mission: &Mission, me: ResidentId, args: &ListArgs) -> bool {
    let in_scope = match args.scope {
        ListScope::Open => {
            mission.poster.resident() != Some(me)
                && state.key_of(me, mission.id).is_none()
                // A team with a place for one more member is open too, but
                // only its organization knows that. A list shows places for
                // new takers.
                && !mission.is_full()
        }
        ListScope::Accepted => state.key_of(me, mission.id).is_some(),
        ListScope::Posted => mission.poster.resident() == Some(me),
    };
    in_scope
        && args.mode.is_none_or(|mode| (mode == Mode::Team) == mission.def.team)
        && args.min_reward.is_none_or(|min| mission.def.reward >= min)
        && args.sponsored.is_none_or(|yes| yes == mission.poster.resident().is_none())
        && args
            .event
            .as_ref()
            .is_none_or(|kind| mission.def.objectives.iter().any(|o| o.event == *kind))
}

fn list(realm: &Realm, p: &Params, ctx: &GlobalCtx, args: &ListArgs) -> Result<Value, ActionError> {
    let state = realm.res::<Missions>();
    if let Some(id) = args.mission {
        let mission = state.open.get(&id).ok_or_else(|| no_mission(id))?;
        return Ok(json!({ "mission": view::full(&state, mission, Some(ctx.resident)) }));
    }
    let max = params::count(realm, p.list_max).max(1);
    let limit = args.limit.map_or(max, |limit| (limit as usize).clamp(1, max));
    let first = args.after.map_or(0, |after| after.saturating_add(1));
    let mut found = state
        .open
        .range(first..)
        .map(|(_, mission)| mission)
        .filter(|mission| passes(&state, mission, ctx.resident, args));
    let missions: Vec<Value> = found.by_ref().take(limit).map(view::brief).collect();
    let mut out = json!({ "missions": missions });
    // The cursor is there only when more missions pass the filters.
    if found.next().is_some()
        && let Some(last) = out["missions"].as_array().and_then(|list| list.last())
    {
        let next = last["mission"].clone();
        out["next"] = next;
    }
    Ok(out)
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<ListArgs>(
        read_spec(
            "mission_list",
            "Read the open missions, with filters. With `mission`, read one mission with its text and objectives.",
        ),
        move |realm, ctx, args| list(realm, &p, ctx, args),
    );
}
