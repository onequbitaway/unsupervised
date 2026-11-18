//! The verb `search`: an officer searches a resident next to it and takes
//! hot value and contraband.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{Begin, Builder, Dest, EventMeta, ResidentId};

use super::{keep_result, kept_result, need_power};
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::msg::{Report, Source};
use crate::names::{event, notice, perm};
use crate::powers;
use crate::record::{Caught, Record};
use crate::seize;
use crate::verbs::{in_range, spec};

/// Arguments of `search`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SearchArgs {
    /// The resident to search. It must stand on your tile or on a tile
    /// next to it.
    pub target: ResidentId,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<SearchArgs>(
        spec(
            "search",
            "Police: search a resident next to you. Stolen cash, stolen goods and contraband that it holds go to the police station, and the find is evidence.",
        ),
        move |realm, ctx, args| {
            need_power(realm, ctx.idx, &[perm::SEARCH])?;
            let target = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.search_range))?;
            let found = seize::search(realm, &h, target);
            let frame = realm.frame();
            let until = frame.saturating_add(h.frames(realm, h.p.caught_ticks));
            let max = h.count(realm, h.p.known_max);
            // The loot of a case in the hands of a resident is evidence
            // against it, and grounds for this officer to arrest it.
            let points = realm.param(h.p.evidence_search);
            for &case in &found.cases {
                realm.send(Dest::Global, &Report::Evidence { case, source: Source::Search, points });
                realm.col_mut::<Record>()[target]
                    .add_caught(Caught { case, officer: ctx.resident, until }, max);
            }
            // Contraband in a pocket is a crime of its own.
            let mut smuggling = None;
            if !found.contraband.is_empty() {
                let case = commit::new_case(realm);
                let pos = realm.pos(target);
                let deed = Deed {
                    kind: CrimeKind::Smuggling,
                    perp: target,
                    pos,
                    site: realm.sites().at(pos).map(|site| site.id),
                    victim: None,
                    target: None,
                    owner: None,
                    value: found.contraband_value,
                    failed: false,
                    force: false,
                };
                commit::file(realm, &h, case, &deed, Finder::Officer(ctx.resident));
                smuggling = Some(case);
            }
            let mut result = found.json(realm);
            if let Some(map) = result.as_object_mut() {
                map.insert("target".into(), json!(args.target));
                map.insert("found".into(), json!(!found.is_empty()));
                if let Some(case) = smuggling {
                    map.insert("smuggling_case".into(), json!(case));
                }
            }
            realm.notify(args.target, notice::SEARCHED, Some(ctx.resident), result.clone());
            // The event names the officer and not the resident: a find is
            // not a verdict.
            realm.emit(
                EventMeta::public(event::SEARCH, h.drama(realm, h.p.drama_search))
                    .actor(ctx.resident)
                    .at(realm.pos(ctx.idx)),
                &json!({
                    "officer": ctx.resident,
                    "city": powers::city_of(realm),
                    "found": !found.is_empty(),
                }),
            );
            realm.attr_add(ctx.idx, h.investigation, realm.param(h.p.search_xp).max(0));
            keep_result(realm, ctx, &result);
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.search_ticks) })
        },
        |realm, ctx, _args| Ok(kept_result(realm, ctx)),
    );
}
