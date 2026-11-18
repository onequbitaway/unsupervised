//! The verb `patrol`: an officer watches the streets around it.
//!
//! While the action runs, the officer sees each crime within
//! `crime.patrol_radius` tiles with a high chance (see `commit`). At the
//! end, the result lists the wanted residents near the officer.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, ResidentId};
use unsup_life::names::TICKS_PER_HOUR;

use super::need_power;
use crate::ids::Handles;
use crate::names::perm;
use crate::verbs::spec;

/// Arguments of `patrol`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct PatrolArgs {
    /// Length of the patrol in ticks. One tick is one game minute. Without
    /// it, the patrol has the default length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<PatrolArgs>(
        spec(
            "patrol",
            "Police: watch the place around you for some time. You see most crimes near you while you patrol, and at the end you learn which wanted residents are near.",
        ),
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::ARREST, perm::SEARCH])?;
            let most = h.ticks(realm, h.p.patrol_max_ticks);
            let ticks = args.ticks.unwrap_or_else(|| h.ticks(realm, h.p.patrol_ticks).min(most));
            if !(1..=most).contains(&ticks) {
                return Err(ActionError::invalid_args(format!("`ticks` must be from 1 to {most}.")));
            }
            Ok(Begin::Busy { ticks })
        },
        move |realm, ctx, _args| {
            let ticks = i64::try_from(realm.tick().since(ctx.started)).unwrap_or(0);
            let xp = ticks.saturating_mul(realm.param(h.p.patrol_xp).max(0))
                / i64::from(TICKS_PER_HOUR);
            realm.attr_add(ctx.idx, h.investigation, xp);
            let radius = h.tiles(realm, h.p.patrol_radius);
            let mut wanted: Vec<ResidentId> = realm
                .nearby(realm.pos(ctx.idx), radius)
                .into_iter()
                .filter(|&idx| idx != ctx.idx && realm.has_condition(idx, h.wanted))
                .map(|idx| realm.id_of(idx))
                .collect();
            let most = h.count(realm, h.p.patrol_list_max);
            let more = wanted.len().saturating_sub(most);
            wanted.truncate(most);
            Ok(json!({ "ticks": ticks, "wanted": wanted, "more_wanted": more }))
        },
    );
}
