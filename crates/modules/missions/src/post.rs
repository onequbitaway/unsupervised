//! `mission_post`: a resident publishes a mission and locks the reward.

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, Holder, Realm, Value};

use crate::model::{Definition, Mission, Missions, Poster};
use crate::params::{self, Params};
use crate::pay::{self, ESCROW};
use crate::remote::{Call, Origin, remote_verb};
use crate::spec::MissionSpec;
use crate::{check, engine, spec};

/// The global part of `mission_post`.
#[derive(Serialize, Deserialize)]
struct Post {
    def: Definition,
}

impl Call for Post {
    const KIND: &'static str = "missions.post";
}

/// The error for a world with too many open missions.
pub(crate) fn world_full(realm: &Realm, p: &Params) -> ActionError {
    ActionError::conflict(format!(
        "The world has {} open missions already. Try again later.",
        params::count(realm, p.open_max)
    ))
}

fn post(realm: &Realm, p: &Params, origin: &Origin, call: Post) -> Result<Value, ActionError> {
    let mut state = realm.res_mut::<Missions>();
    if state.open_count() >= params::count(realm, p.open_max) {
        return Err(world_full(realm, p));
    }
    let limit = params::count(realm, p.posted_max);
    if state.posted_by(origin.resident) >= limit {
        return Err(ActionError::conflict(format!(
            "You have too many open missions. The limit is {limit}."
        )));
    }
    let total = call.def.total();
    if realm.ledger().money(origin.escrow()).cents() < total {
        return Err(ActionError::internal("The reward did not arrive with the request."));
    }
    let id = realm.alloc_id();
    let locked = pay::transfer(realm, origin.escrow(), Holder::Escrow(id), total, ESCROW);
    let now = realm.frame();
    let deadline = now + u64::from(call.def.deadline_in);
    let mission = Mission {
        id,
        poster: Poster::Resident { id: origin.resident, owner: origin.owner },
        def: call.def,
        posted: now,
        deadline,
        city: Some(origin.city),
        takers: Default::default(),
    };
    engine::open(realm, p, &mut state, mission, None);
    Ok(json!({ "mission": id, "locked": locked, "deadline": deadline }))
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    remote_verb::<MissionSpec, Post>(
        b,
        spec(
            "mission_post",
            "Publish a mission: objectives that the engine checks from events, a reward for each taker and a deadline. The reward for all takers leaves your cash and is locked at once.",
        ),
        p,
        move |realm, ctx, args| {
            let def = check::definition(realm, &p, args)?;
            let total = def.total();
            let cash = realm.ledger().money(Holder::Resident(ctx.resident)).cents();
            if cash < total {
                return Err(ActionError::insufficient(format!(
                    "The mission locks {total} cents: the reward for each of its takers. You have {cash}."
                )));
            }
            Ok((Post { def }, total))
        },
        move |realm, origin, call| post(realm, &p, origin, call),
    );
}
