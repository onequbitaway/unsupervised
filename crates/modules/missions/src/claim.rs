//! `mission_claim`: ask for the money of a mission.
//!
//! The engine pays a taker by itself, in the frame of the event that meets
//! the objectives. So a claim of a taker never has to move money: it tells
//! the taker what the mission paid, or what is still missing. A claim of
//! the poster takes the unpaid reward back before the deadline, when nobody
//! works on the mission.

use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Builder, GlobalCtx, Realm, Value};

use crate::accept::{MissionArgs, no_mission};
use crate::model::Missions;
use crate::params::Params;
use crate::{engine, spec, view};

fn claim(
    realm: &Realm,
    p: &Params,
    ctx: &GlobalCtx,
    args: &MissionArgs,
) -> Result<Value, ActionError> {
    let me = ctx.resident;
    let mut state = realm.res_mut::<Missions>();
    let Some(mission) = state.open.get(&args.mission) else {
        return Err(no_mission(args.mission));
    };
    if mission.poster.resident() == Some(me) {
        let active = mission.active();
        if active > 0 {
            return Err(ActionError::conflict(format!(
                "{active} takers work on the mission. The unpaid reward comes back at the deadline."
            ))
            .with_details(json!({ "active": active, "deadline": mission.deadline })));
        }
        let refunded = engine::close(realm, p, &mut state, args.mission, "withdrawn");
        return Ok(json!({ "mission": args.mission, "status": "withdrawn", "refunded": refunded }));
    }
    if let Some(key) = state.key_of(me, args.mission)
        && let Some(taker) = mission.takers.get(&key)
    {
        return Err(ActionError::conflict(
            "The objectives are not met yet. The engine pays by itself when they are.",
        )
        .with_details(json!({ "progress": view::progress(mission, taker) })));
    }
    match view::paid_to(mission, me) {
        Some((paid, frame)) => {
            Ok(json!({ "mission": args.mission, "status": "paid", "paid": paid, "frame": frame }))
        }
        None => Err(ActionError::not_allowed(
            "You did not accept this mission and you did not publish it.",
        )),
    }
}

/// Registers the verb.
pub(crate) fn build(b: &mut Builder, p: Params) {
    b.global_verb::<MissionArgs>(
        spec(
            "mission_claim",
            "Ask for the money of a mission. A taker learns what the mission paid or what is missing. The poster takes the unpaid reward back when nobody works on the mission.",
        ),
        move |realm, ctx, args| claim(realm, &p, ctx, args),
    );
}
