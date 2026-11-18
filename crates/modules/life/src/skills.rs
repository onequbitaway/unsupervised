//! Skills: the level formula and the verb `train`.
//!
//! A skill is an attribute `skill.<name>` that counts experience points.
//! Other modules add experience when a resident does the related work. The
//! life module teaches at a school.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::json;
use unsup_kernel::{ActionError, Begin, Builder, EventMeta, Holder, Idx, Money, Realm, VerbSpec};

use crate::handles::Handles;
use crate::math::div_ceil;
use crate::names::{SKILLS, TICKS_PER_HOUR, attr, site};
use crate::sites::site_here;

/// Experience points of level 1. Level `n` needs `n * n` times as many:
/// `level = floor(sqrt(experience / 100))`. The number is part of the game
/// design (`docs/design/game-design.md`, section 3).
pub const XP_PER_LEVEL_SQUARED: i64 = 100;

/// Returns the level of a skill with `xp` experience points:
/// `floor(sqrt(xp / 100))`.
#[must_use]
pub fn skill_level(xp: i64) -> i64 {
    (xp.max(0) / XP_PER_LEVEL_SQUARED).isqrt()
}

/// Returns the experience points that a level needs: `level * level * 100`.
#[must_use]
pub fn xp_for_level(level: i64) -> i64 {
    level.max(0).saturating_mul(level.max(0)).saturating_mul(XP_PER_LEVEL_SQUARED)
}

/// Returns the name of the attribute of a skill: `skill.<name>`.
#[must_use]
pub fn skill_attr(skill: &str) -> String {
    format!("{}{skill}", attr::SKILL_PREFIX)
}

/// Returns the experience that a resident learns from `base` points: all of
/// it at full mood, and the percent `life.learn_min_pct` of it at no mood.
pub(crate) fn learned(realm: &Realm, h: &Handles, idx: Idx, base: i64) -> i64 {
    let least = realm.param(h.p.learn_min_pct).clamp(0, 100);
    let mood = realm.attr(idx, h.mood).clamp(0, attr::NEED_MAX);
    let percent_times_max = least * attr::NEED_MAX + (100 - least) * mood;
    base.saturating_mul(percent_times_max) / (100 * attr::NEED_MAX)
}

/// Arguments of `train`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct TrainArgs {
    /// The skill to learn: labor, farming, mining, crafting, cooking,
    /// trading, management, medicine, law, investigation, stealth, combat or
    /// rhetoric.
    pub skill: String,
    /// Length of the lesson in ticks. One tick is one game minute. Without
    /// it, the lesson has the shortest length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
}

/// Returns the length of a lesson, or an error when the request is outside
/// the limits.
fn lesson_ticks(realm: &Realm, h: &Handles, args: &TrainArgs) -> Result<u32, ActionError> {
    let least = h.ticks(realm, h.p.train_min_ticks);
    let most = h.ticks(realm, h.p.train_max_ticks).max(least);
    match args.ticks {
        None => Ok(least),
        Some(ticks) if (least..=most).contains(&ticks) => Ok(ticks),
        Some(_) => {
            Err(ActionError::invalid_args(format!("`ticks` must be from {least} to {most}.")))
        }
    }
}

/// Registers the verb `train`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    b.verb::<TrainArgs>(
        VerbSpec {
            verb: "train",
            module: "life",
            summary: "Take a lesson in a skill at a school. You pay the fee first.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| {
            if !SKILLS.contains(&args.skill.as_str()) {
                return Err(ActionError::invalid_args(format!(
                    "No skill is named `{}`.",
                    args.skill
                ))
                .with_details(json!({ "skills": SKILLS })));
            }
            let ticks = lesson_ticks(realm, &h, args)?;
            let Some(school) = site_here(realm, ctx.idx).filter(|place| place.kind == site::SCHOOL)
            else {
                return Err(ActionError::not_here("You must be in a school."));
            };
            let fee = div_ceil(
                i64::from(ticks).saturating_mul(realm.param(h.p.train_fee)),
                i64::from(TICKS_PER_HOUR),
            );
            realm.ledger_mut().transfer_money(
                Holder::Resident(ctx.resident),
                Holder::Site(school.id),
                Money(fee),
                "tuition",
            )?;
            Ok(Begin::Busy { ticks })
        },
        move |realm, ctx, args| {
            let Some(slot) = SKILLS.iter().position(|skill| *skill == args.skill) else {
                return Err(ActionError::invalid_args("No skill has this name."));
            };
            // Something else can move a resident during the lesson.
            if site_here(realm, ctx.idx).is_none_or(|place| place.kind != site::SCHOOL) {
                return Err(ActionError::conflict("You left the school before the lesson ended."));
            }
            let skill = h.skills[slot];
            let ticks = realm.tick().since(ctx.started);
            let base = i64::try_from(ticks).unwrap_or(0).saturating_mul(realm.param(h.p.train_xp))
                / i64::from(TICKS_PER_HOUR);
            let xp = learned(realm, &h, ctx.idx, base);
            let before = skill_level(realm.attr(ctx.idx, skill));
            let total = realm.attr_add(ctx.idx, skill, xp);
            let level = skill_level(total);
            let pos = realm.pos(ctx.idx);
            realm.emit(
                EventMeta::public("life.skill_trained", h.drama(realm, h.p.drama_trained))
                    .actor(ctx.resident)
                    .at(pos),
                &json!({ "resident": ctx.resident, "skill": args.skill, "xp": xp, "level": level }),
            );
            if level > before {
                realm.emit(
                    EventMeta::public("life.level_gained", h.drama(realm, h.p.drama_level))
                        .actor(ctx.resident)
                        .at(pos),
                    &json!({ "resident": ctx.resident, "skill": args.skill, "level": level }),
                );
            }
            Ok(json!({ "skill": args.skill, "xp": xp, "total": total, "level": level }))
        },
    );
}
