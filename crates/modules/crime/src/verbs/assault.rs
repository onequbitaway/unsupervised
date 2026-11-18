//! The verb `assault`: an attack on a resident next to you.
//!
//! Violence is abstract: the game reports an assault and a loss of health.
//! A victim whose health reaches 0 dies through the life module, with the
//! cause `violence`.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{ActionError, Begin, Builder, Holder, ResidentId};

use super::rob::{CAUSE_VIOLENCE, force_chance};
use super::{hurt, in_range, learn, not_guarded, outcome, spec};
use crate::calc;
use crate::commit::{self, Deed, Finder};
use crate::ids::Handles;
use crate::kinds::CrimeKind;

/// The parameter of the life module with the fee of the hospital for one
/// health point. The loss of the victim of an assault is that fee for the
/// health that it lost.
const HEAL_FEE: &str = "life.heal_fee_per_point";

/// Arguments of `assault`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AssaultArgs {
    /// The resident to attack. It must stand on your tile or on a tile
    /// next to it.
    pub target: ResidentId,
}

/// Registers the verb.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<AssaultArgs>(
        spec(
            "assault",
            "Attack a resident next to you. Combat decides. The victim loses health and can die; when you fail, you are hurt. It is a crime.",
        ),
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.assault_range))?;
            not_guarded(realm, &h, victim)?;
            Ok(Begin::Busy { ticks: h.ticks(realm, h.p.assault_ticks) })
        },
        move |realm, ctx, args| {
            let victim = in_range(realm, ctx.idx, args.target, h.tiles(realm, h.p.assault_range))
                .map_err(|_| ActionError::not_here("The target walked away before you could act."))?;
            not_guarded(realm, &h, victim)?;
            let base = realm.param(h.p.assault_base);
            let success = calc::roll(realm, force_chance(realm, &h, base, ctx.idx, victim));
            let case = commit::new_case(realm);
            let pos = realm.pos(victim);
            let mut damage = 0;
            let mut fatal = false;
            let mut own_injury = 0;
            if success {
                let blow = realm.param(h.p.assault_damage)
                    + calc::level(realm, ctx.idx, h.combat) * realm.param(h.p.assault_level_damage);
                let (lost, left) = hurt(realm, &h, victim, blow, 0);
                damage = lost;
                fatal = left <= 0;
                realm.interrupt(victim, "You were attacked.");
            } else {
                // The victim fought back. A failed attack never kills.
                own_injury = hurt(realm, &h, ctx.idx, realm.param(h.p.assault_fail_injury), 1).0;
            }
            let fee = realm.defs().param_id(HEAL_FEE).map_or(0, |id| realm.param(id));
            let deed = Deed {
                kind: if fatal { CrimeKind::Homicide } else { CrimeKind::Assault },
                perp: ctx.idx,
                pos,
                site: realm.sites().at(pos).map(|site| site.id),
                victim: Some(Holder::Resident(args.target)),
                target: Some(args.target),
                owner: None,
                value: damage.saturating_mul(fee.max(0)),
                failed: !success,
                force: true,
            };
            let filed = commit::file(realm, &h, case, &deed, Finder::Nobody);
            if fatal {
                unsup_life::die(realm, victim, CAUSE_VIOLENCE);
            }
            learn(realm, &h, ctx.idx, h.combat, success);
            let mut out = outcome(success, filed);
            out.insert("target".into(), json!(args.target));
            out.insert("damage".into(), json!(damage));
            out.insert("fatal".into(), json!(fatal));
            out.insert("hurt".into(), json!(own_injury));
            Ok(Value::Object(out))
        },
    );
}
