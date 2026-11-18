//! The verb `pickpocket`: cash from a resident next to you, in secret.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{ActionError, Begin, Builder, Holder, Realm, ResidentId};

use super::{in_range, learn, not_guarded, outcome, spec};
use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::loot;

/// Arguments of `pickpocket`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PickpocketArgs {
    /// The resident to steal from. It must stand on your tile or on a tile
    /// next to it.
    pub target: ResidentId,
}

/// Returns the chance in basis points that the theft works.
fn chance(realm: &Realm, h: &Handles, thief: unsup_kernel::Idx, victim: unsup_kernel::Idx) -> u32 {
    let p = &h.p;
    let mut raw = realm.param(p.pickpocket_base)
        + calc::level(realm, thief, h.stealth) * realm.param(p.stealth_level)
        - calc::level(realm, victim, h.investigation) * realm.param(p.pickpocket_alert);
    if calc::is_night(realm, h) {
        raw += realm.param(p.night_bonus);
    }
    if realm.has_condition(victim, h.asleep) {
        raw += realm.param(p.pickpocket_asleep);
    }
    calc::clamp_chance(realm, h, raw)
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<PickpocketArgs>(
        spec(
            "pickpocket",
            "Steal cash from a resident next to you. Stealth helps. It is a crime: the victim learns of it and witnesses can recognize you.",
        ),
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.pickpocket_range))?;
            not_guarded(realm, &h, victim)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.pickpocket_ticks) })
        },
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.pickpocket_range))
                .map_err(|_| ActionError::not_here("The target walked away before you could act."))?;
            not_guarded(realm, &h, victim)?;
            let success = calc::roll(realm, chance(realm, &h, ctx.idx, victim));
            let case = commit::new_case(realm);
            let from = Holder::Resident(args.target);
            let pos = realm.pos(victim);
            let mut cash = 0;
            if success {
                let purse = realm.ledger().money(from).cents();
                let want = calc::percent(purse, realm.param(h.p.pickpocket_pct))
                    .min(realm.param(h.p.pickpocket_max));
                cash = loot::take(realm, &h, case, from, ctx.idx, want, &[]).cash;
            }
            let deed = Deed {
                kind: CrimeKind::Theft,
                perp: ctx.idx,
                pos,
                site: realm.sites().at(pos).map(|site| site.id),
                victim: Some(from),
                target: Some(args.target),
                owner: None,
                value: cash,
                failed: !success,
                force: false,
            };
            let filed = commit::file(realm, &h, case, &deed, Finder::Nobody);
            learn(realm, &h, ctx.idx, h.stealth, success);
            let mut out = outcome(success, filed);
            out.insert("target".into(), json!(args.target));
            out.insert("cash".into(), json!(cash));
            Ok(Value::Object(out))
        },
    );
}
