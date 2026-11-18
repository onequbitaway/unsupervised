//! The verb `investigate`: an officer reads the traces of a crime at its
//! scene.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, Dest, Idx, Realm, ResidentId};

use super::need_power;
use crate::calc;
use crate::ids::Handles;
use crate::msg::{Report, Source};
use crate::names::perm;
use crate::scene::{self, Scenes, Trace};
use crate::verbs::spec;

/// Arguments of `investigate`.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct InvestigateArgs {
    /// The case to look for. Without it, you read the newest trace near
    /// you that you did not read yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<u64>,
}

/// Returns the trace that the officer can read from where it stands.
fn trace_here(
    realm: &Realm,
    h: &Handles,
    idx: Idx,
    officer: ResidentId,
    case: Option<u64>,
) -> Result<Trace, ActionError> {
    let radius = h.tiles(realm, h.p.investigate_radius);
    let ttl = h.frames(realm, h.p.trace_ticks);
    let scenes = realm.res::<Scenes>();
    scenes.find(realm.pos(idx), radius, officer, case, realm.frame(), ttl).cloned().ok_or_else(
        || {
            ActionError::not_found(
                "No trace that you did not read is near you. Go to the place of the crime.",
            )
        },
    )
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<InvestigateArgs>(
        spec(
            "investigate",
            "Police: read the traces of a crime near you. A fresh trace gives more evidence than an old one. Your investigation skill helps and the stealth of the perpetrator hinders.",
        ),
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::SEARCH, perm::ARREST])?;
            trace_here(realm, &h, ctx.idx, ctx.resident, args.case)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.investigate_ticks) })
        },
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::SEARCH, perm::ARREST])?;
            let trace = trace_here(realm, &h, ctx.idx, ctx.resident, args.case)?;
            let p = &h.p;
            let raw = realm.param(p.investigate_base)
                + calc::level(realm, ctx.idx, h.investigation) * realm.param(p.investigate_level)
                - trace.stealth * realm.param(p.investigate_stealth);
            let found = calc::roll(realm, calc::clamp_chance(realm, &h, raw));
            let xp = realm.param(p.investigate_xp).max(0);
            if !found {
                realm.attr_add(ctx.idx, h.investigation, xp / 2);
                return Ok(json!({ "case": trace.case, "kind": trace.kind, "found": false }));
            }
            let age = realm.frame().saturating_sub(trace.frame);
            let ttl = h.frames(realm, p.trace_ticks);
            let points = scene::strength(realm.param(p.evidence_investigate), age, ttl).max(1);
            realm.res_mut::<Scenes>().mark_read(trace.case, ctx.resident);
            realm.send(
                Dest::Global,
                &Report::Evidence { case: trace.case, source: Source::Investigation, points },
            );
            realm.attr_add(ctx.idx, h.investigation, xp);
            Ok(json!({
                "case": trace.case, "kind": trace.kind, "found": true, "evidence": points,
            }))
        },
    );
}
