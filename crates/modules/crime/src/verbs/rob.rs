//! The verb `rob`: cash and goods from a resident next to you, by force.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{ActionError, Begin, Builder, Holder, Idx, Realm, ResidentId};

use super::{hurt, in_range, learn, not_guarded, outcome, spec};
use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::loot;

/// Cause of a death by violence, as the life module reports it.
pub(crate) const CAUSE_VIOLENCE: &str = "violence";

/// Arguments of `rob`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RobArgs {
    /// The resident to rob. It must stand on your tile or on a tile next
    /// to it.
    pub target: ResidentId,
}

/// Returns the chance in basis points that an attack on `victim` works:
/// `base` plus the difference of the combat levels, plus a bonus against
/// a victim that sleeps.
pub(crate) fn force_chance(
    realm: &Realm,
    h: &Handles,
    base: i64,
    attacker: Idx,
    victim: Idx,
) -> u32 {
    let edge = calc::level(realm, attacker, h.combat) - calc::level(realm, victim, h.combat);
    let mut raw = base + edge * realm.param(h.p.combat_level);
    if realm.has_condition(victim, h.asleep) {
        raw += realm.param(h.p.force_asleep);
    }
    calc::clamp_chance(realm, h, raw)
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<RobArgs>(
        spec(
            "rob",
            "Take cash and goods from a resident next to you by force. Combat decides. The victim is hurt; when you fail, you are hurt. It is a crime.",
        ),
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.rob_range))?;
            not_guarded(realm, &h, victim)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.rob_ticks) })
        },
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.rob_range))
                .map_err(|_| ActionError::not_here("The target walked away before you could act."))?;
            not_guarded(realm, &h, victim)?;
            let base = realm.param(h.p.rob_base);
            let success = calc::roll(realm, force_chance(realm, &h, base, ctx.idx, victim));
            let case = commit::new_case(realm);
            let from = Holder::Resident(args.target);
            let pos = realm.pos(victim);
            let mut haul = loot::Haul::default();
            let mut injury = 0;
            let mut fatal = false;
            let mut own_injury = 0;
            if success {
                let purse = realm.ledger().money(from).cents();
                let cash = calc::percent(purse, realm.param(h.p.rob_cash_pct));
                let goods = loot::pick(realm, from, realm.param(h.p.rob_units_max));
                haul = loot::take(realm, &h, case, from, ctx.idx, cash, &goods);
                let (lost, left) = hurt(realm, &h, victim, realm.param(h.p.rob_injury), 0);
                injury = lost;
                fatal = left <= 0;
                realm.interrupt(victim, "You were robbed.");
            } else {
                // The victim fought back. A failed attack never kills.
                own_injury = hurt(realm, &h, ctx.idx, realm.param(h.p.rob_fail_injury), 1).0;
            }
            let deed = Deed {
                kind: if fatal { CrimeKind::Homicide } else { CrimeKind::Robbery },
                perp: ctx.idx,
                pos,
                site: realm.sites().at(pos).map(|site| site.id),
                victim: Some(from),
                target: Some(args.target),
                owner: None,
                value: haul.value,
                failed: !success,
                force: true,
            };
            let filed = commit::file(realm, &h, case, &deed, Finder::Nobody);
            if fatal {
                // The loot is with the robber. The rest is the estate.
                unsup_life::die(realm, victim, CAUSE_VIOLENCE);
            }
            learn(realm, &h, ctx.idx, h.combat, success);
            let mut out = outcome(success, filed);
            out.insert("target".into(), json!(args.target));
            out.insert("cash".into(), json!(haul.cash));
            out.insert("goods".into(), haul.goods_json(realm));
            out.insert("value".into(), json!(haul.value));
            out.insert("injury".into(), json!(injury));
            out.insert("fatal".into(), json!(fatal));
            out.insert("hurt".into(), json!(own_injury));
            Ok(Value::Object(out))
        },
    );
}
