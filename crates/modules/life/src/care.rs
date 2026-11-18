//! Sleep, rest and hospital care: the verbs `sleep`, `rest` and `heal`.
//!
//! Each is a busy action that gives points for every hour that it lasts.
//! The points are added when the action ends. An action that is stopped
//! early (cancel, interrupt) gives the points of the time that passed.

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::{self, JsonSchema};
use unsup_kernel::serde_json::{Value, json};
use unsup_kernel::{
    ActionCtx, ActionError, Begin, Builder, Event, EventMeta, Holder, Idx, Money, Realm,
    ResidentId, Scope, Site, SiteId, Tick, VerbSpec,
};

use crate::body::{Bed, Body};
use crate::handles::Handles;
use crate::math::div_ceil;
use crate::names::{TICKS_PER_HOUR, attr, site};
use crate::sites::{site_here, standing_tile};

/// The blocking conditions in which a resident can still look after its
/// body: eat, sleep, rest, use an item, write a will, look around.
pub(crate) const BODY_CARE: &[&str] = &["hospitalized", "jailed", "detained"];

/// How well a resident sleeps at a place.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SleepQuality {
    /// In a home or an apartment that the resident owns or rents.
    Good,
    /// In a shelter.
    Fair,
    /// In any other place.
    Poor,
}

impl SleepQuality {
    /// Returns the name of the quality: `good`, `fair` or `poor`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SleepQuality::Good => "good",
            SleepQuality::Fair => "fair",
            SleepQuality::Poor => "poor",
        }
    }
}

/// Returns how well a resident sleeps in a site.
#[must_use]
pub fn sleep_quality_in(place: &Site, resident: ResidentId) -> SleepQuality {
    let lives_there = place.owner == Some(Holder::Resident(resident)) || place.is_tenant(resident);
    match place.kind.as_str() {
        site::HOME | site::APARTMENT if lives_there => SleepQuality::Good,
        site::SHELTER => SleepQuality::Fair,
        _ => SleepQuality::Poor,
    }
}

/// Returns how well a resident sleeps where it stands.
#[must_use]
pub fn sleep_quality(realm: &Realm, idx: Idx) -> SleepQuality {
    let resident = realm.id_of(idx);
    realm
        .sites()
        .at(realm.pos(idx))
        .map_or(SleepQuality::Poor, |place| sleep_quality_in(place, resident))
}

/// The kind of a timed care action.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum CareKind {
    /// The verb `sleep`.
    #[default]
    Sleep,
    /// The verb `rest`.
    Rest,
    /// The verb `heal`.
    Heal,
}

impl CareKind {
    fn verb(self) -> &'static str {
        match self {
            CareKind::Sleep => "sleep",
            CareKind::Rest => "rest",
            CareKind::Heal => "heal",
        }
    }
}

/// A sleep, a rest or a hospital stay in progress. It is part of [`Body`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Care {
    /// Id of the action.
    #[serde(rename = "a")]
    pub(crate) action: u64,
    #[serde(rename = "k")]
    pub(crate) kind: CareKind,
    /// Tick of the zone at which the action started. While the resident
    /// moves between two zones, it is the number of ticks that passed.
    #[serde(rename = "s")]
    pub(crate) since: u64,
    /// Energy points for each hour.
    #[serde(rename = "e", default)]
    pub(crate) energy: i64,
    /// Mood points for each hour.
    #[serde(rename = "m", default)]
    pub(crate) mood: i64,
    /// Health points for each hour.
    #[serde(rename = "h", default)]
    pub(crate) health: i64,
    /// The greatest number of health points that the action gives.
    #[serde(rename = "c", default)]
    pub(crate) health_cap: i64,
    /// The site of a hospital stay.
    #[serde(rename = "p", default, skip_serializing_if = "Option::is_none")]
    pub(crate) place: Option<SiteId>,
    /// How well the resident sleeps.
    #[serde(rename = "q", default, skip_serializing_if = "Option::is_none")]
    pub(crate) quality: Option<SleepQuality>,
}

/// Arguments of `sleep`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct SleepArgs {
    /// How many ticks to sleep. One tick is one game minute. Without it,
    /// you sleep until your energy is full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks: Option<u32>,
}

/// Arguments of `rest`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RestArgs {
    /// How many ticks to rest. One tick is one game minute.
    pub ticks: u32,
}

/// Arguments of `heal`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HealArgs {
    /// How many health points to buy. Without it, the hospital heals you
    /// fully.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
}

/// Returns `rate` points for each hour, for a number of ticks.
fn points(rate: i64, ticks: u64) -> i64 {
    rate.saturating_mul(i64::try_from(ticks).unwrap_or(i64::MAX)) / i64::from(TICKS_PER_HOUR)
}

/// Returns the ticks that `points` points take at `rate` points for each
/// hour, rounded up.
fn ticks_for(points: i64, rate: i64) -> u32 {
    let ticks = div_ceil(points.max(0).saturating_mul(i64::from(TICKS_PER_HOUR)), rate);
    u32::try_from(ticks).unwrap_or(u32::MAX)
}

/// Stores the care action of a resident.
fn start(realm: &Realm, ctx: &ActionCtx, care: Care) {
    realm.col_mut::<Body>()[ctx.idx].care =
        Some(Box::new(Care { action: ctx.action_id, since: ctx.started.0, ..care }));
}

/// Ends the care action of a resident: adds the points of the time that
/// passed and clears the condition. Returns the result of the action, or
/// `None` when the resident has no care action with this id.
fn settle(realm: &Realm, h: &Handles, idx: Idx, action: u64) -> Option<Value> {
    let care = {
        let mut bodies = realm.col_mut::<Body>();
        let body = &mut bodies[idx];
        if body.care.as_ref().is_none_or(|care| care.action != action) {
            return None;
        }
        body.care.take()?
    };
    let resident = realm.id_of(idx);
    let ticks = realm.tick().0.saturating_sub(care.since);
    let energy = realm.attr_add(idx, h.energy, points(care.energy, ticks));
    let mood = realm.attr_add(idx, h.mood, points(care.mood, ticks));
    let pos = realm.pos(idx);
    // A hospital heals only a patient that is still inside it. Something
    // else can move a resident during the stay.
    let in_place =
        care.place.is_none_or(|place| realm.sites().at(pos).is_some_and(|site| site.id == place));
    let gained = if in_place { points(care.health, ticks).min(care.health_cap) } else { 0 };
    let health = realm.attr_add(idx, h.health, gained);
    Some(match care.kind {
        CareKind::Sleep => {
            realm.clear_condition(idx, h.asleep);
            let quality = care.quality.unwrap_or(SleepQuality::Poor).name();
            if ticks > 0 {
                let drama = h.drama(realm, h.p.drama_slept);
                realm.emit(
                    EventMeta::public("life.slept", drama).actor(resident).at(pos),
                    &json!({ "resident": resident, "quality": quality, "ticks": ticks }),
                );
            }
            json!({ "ticks": ticks, "quality": quality, "energy": energy, "mood": mood })
        }
        CareKind::Rest => json!({ "ticks": ticks, "energy": energy, "mood": mood }),
        CareKind::Heal => {
            realm.clear_condition(idx, h.hospitalized);
            if gained > 0 {
                let drama = h.drama(realm, h.p.drama_healed);
                realm.emit(
                    EventMeta::public("life.patient_healed", drama).actor(resident).at(pos),
                    &json!({ "resident": resident, "site": care.place, "health": gained }),
                );
            }
            json!({ "ticks": ticks, "healed": gained, "health": health })
        }
    })
}

/// `finish` of the three verbs.
fn finish(realm: &Realm, h: &Handles, ctx: &ActionCtx) -> Result<Value, ActionError> {
    settle(realm, h, ctx.idx, ctx.action_id)
        .ok_or_else(|| ActionError::conflict("The action was stopped."))
}

fn sleep_begin(
    realm: &Realm,
    h: &Handles,
    ctx: &ActionCtx,
    args: &SleepArgs,
) -> Result<Begin, ActionError> {
    let energy = realm.attr(ctx.idx, h.energy);
    if energy >= attr::NEED_MAX {
        return Err(ActionError::conflict("You are not tired: your energy is full."));
    }
    let place = site_here(realm, ctx.idx);
    let quality =
        place.as_ref().map_or(SleepQuality::Poor, |place| sleep_quality_in(place, ctx.resident));
    let (energy_rate, mood_rate) = match quality {
        SleepQuality::Good => (h.p.sleep_energy_good, h.p.sleep_mood_good),
        SleepQuality::Fair => (h.p.sleep_energy_fair, h.p.sleep_mood_fair),
        SleepQuality::Poor => (h.p.sleep_energy_poor, h.p.sleep_mood_poor),
    };
    let (energy_rate, mood_rate) = (realm.param(energy_rate), realm.param(mood_rate));
    let least = u32::try_from(realm.param(h.p.sleep_min_ticks)).unwrap_or(1);
    let most = u32::try_from(realm.param(h.p.sleep_max_ticks)).unwrap_or(1).max(least);
    let ticks = match args.ticks {
        Some(ticks) if (least..=most).contains(&ticks) => ticks,
        Some(_) => {
            return Err(ActionError::invalid_args(format!(
                "`ticks` must be from {least} to {most}."
            )));
        }
        None => ticks_for(attr::NEED_MAX - energy, energy_rate).clamp(least, most),
    };
    // The resident remembers the place when it is better than the street.
    if let Some(place) = place.filter(|_| quality != SleepQuality::Poor) {
        remember_bed(realm, ctx.idx, &place, quality == SleepQuality::Good);
    }
    start(
        realm,
        ctx,
        Care {
            kind: CareKind::Sleep,
            energy: energy_rate,
            mood: mood_rate,
            quality: Some(quality),
            ..Care::default()
        },
    );
    realm.set_condition(ctx.idx, h.asleep, Some(Tick(ctx.started.0 + u64::from(ticks))));
    Ok(Begin::Busy { ticks })
}

/// Stores a site as the place where a resident sleeps.
pub(crate) fn remember_bed(realm: &Realm, idx: Idx, place: &Site, good: bool) {
    let Some(zone) = realm.zone_info() else {
        return;
    };
    let Some(kind) = site::ALL.iter().position(|kind| *kind == place.kind) else {
        return;
    };
    let pos = standing_tile(&realm.tiles(), place.rect).unwrap_or(place.rect.centre());
    realm.col_mut::<Body>()[idx].bed =
        Some(Bed { site: place.id, zone: zone.id, pos, kind: kind as u8, good });
}

fn rest_begin(
    realm: &Realm,
    h: &Handles,
    ctx: &ActionCtx,
    args: &RestArgs,
) -> Result<Begin, ActionError> {
    let most = u32::try_from(realm.param(h.p.rest_max_ticks)).unwrap_or(1);
    if args.ticks == 0 || args.ticks > most {
        return Err(ActionError::invalid_args(format!("`ticks` must be from 1 to {most}.")));
    }
    let in_park = site_here(realm, ctx.idx).is_some_and(|place| place.kind == site::PARK);
    let mood = if in_park { h.p.rest_mood_park } else { h.p.rest_mood };
    start(
        realm,
        ctx,
        Care {
            kind: CareKind::Rest,
            energy: realm.param(h.p.rest_energy),
            mood: realm.param(mood),
            ..Care::default()
        },
    );
    Ok(Begin::Busy { ticks: args.ticks })
}

fn heal_begin(
    realm: &Realm,
    h: &Handles,
    ctx: &ActionCtx,
    args: &HealArgs,
) -> Result<Begin, ActionError> {
    let Some(hospital) = site_here(realm, ctx.idx).filter(|place| place.kind == site::HOSPITAL)
    else {
        return Err(ActionError::not_here("You must be in a hospital."));
    };
    let missing = attr::NEED_MAX - realm.attr(ctx.idx, h.health);
    if missing <= 0 {
        return Err(ActionError::conflict("You are healthy: your health is full."));
    }
    let wanted = match args.points {
        Some(0) => return Err(ActionError::invalid_args("`points` must be greater than zero.")),
        Some(points) => i64::from(points).min(missing),
        None => missing,
    };
    let fee = Money(wanted.saturating_mul(realm.param(h.p.heal_fee)));
    let rate = realm.param(h.p.heal_rate);
    realm.ledger_mut().transfer_money(
        Holder::Resident(ctx.resident),
        Holder::Site(hospital.id),
        fee,
        "hospital_fee",
    )?;
    let ticks = ticks_for(wanted, rate).max(1);
    start(
        realm,
        ctx,
        Care {
            kind: CareKind::Heal,
            health: rate,
            health_cap: wanted,
            place: Some(hospital.id),
            ..Care::default()
        },
    );
    realm.set_condition(ctx.idx, h.hospitalized, Some(Tick(ctx.started.0 + u64::from(ticks))));
    Ok(Begin::Busy { ticks })
}

/// The kernel stopped an action (cancel, interrupt). A stopped sleep, rest
/// or hospital stay gives the points of the time that passed.
fn interrupted(realm: &Realm, h: &Handles, event: &Event) {
    let verb = event.data["verb"].as_str();
    if ![CareKind::Sleep, CareKind::Rest, CareKind::Heal].iter().any(|k| Some(k.verb()) == verb) {
        return;
    }
    let Some(idx) = event.actor.and_then(|id| realm.idx_of(id)) else {
        return;
    };
    if let Some(action) = event.data["action_id"].as_u64() {
        settle(realm, h, idx, action);
    }
}

/// Changes the start tick of a care action to the ticks that passed, and
/// back. The two zones of a migration count their ticks apart.
fn rebase(realm: &Realm, idx: Idx) {
    let tick = realm.tick().0;
    if let Some(care) = realm.col_mut::<Body>()[idx].care.as_deref_mut() {
        care.since = tick.saturating_sub(care.since);
    }
}

/// Registers the verbs `sleep`, `rest` and `heal`.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.on_event("kernel.action_interrupted", move |realm, event| interrupted(realm, &h, event));
        b.on_depart(rebase);
        b.on_arrive(rebase);
    }
    b.verb::<SleepArgs>(
        VerbSpec {
            verb: "sleep",
            module: "life",
            summary: "Sleep to get energy back. Sleep is good in a home that you own or rent, fair in a shelter and poor elsewhere.",
            free: false,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx, args| sleep_begin(realm, &h, ctx, args),
        move |realm, ctx, _args| finish(realm, &h, ctx),
    );
    b.verb::<RestArgs>(
        VerbSpec {
            verb: "rest",
            module: "life",
            summary: "Rest to get a little energy and mood back. A park gives more mood.",
            free: false,
            allowed_in: BODY_CARE,
        },
        move |realm, ctx, args| rest_begin(realm, &h, ctx, args),
        move |realm, ctx, _args| finish(realm, &h, ctx),
    );
    b.verb::<HealArgs>(
        VerbSpec {
            verb: "heal",
            module: "life",
            summary: "Get health back in a hospital. You pay the fee first and stay until you are healed.",
            free: false,
            allowed_in: &[],
        },
        move |realm, ctx, args| heal_begin(realm, &h, ctx, args),
        move |realm, ctx, _args| finish(realm, &h, ctx),
    );
}
