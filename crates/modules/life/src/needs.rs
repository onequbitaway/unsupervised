//! The needs of the body: food, energy, health and mood.
//!
//! One pass for each zone every game hour changes the needs of the
//! residents of the zone. Nothing here runs every tick. See
//! `docs/design/modules/life.md` for the rules.
//!
//! Every resident has its own clock (`Body::needs_at`): the tick up to which
//! it paid for its needs. The pass charges a resident one hour only when a
//! whole hour passed on that clock, and a resident that arrives from another
//! zone pays the hour that it owes at once. So every resident pays one hour
//! for each hour that it lives. A resident that changes zone often pays no
//! more, and a resident cannot keep away from the pass of every zone.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, Idx, Realm, Scope, Stage};

use crate::body::Body;
use crate::death;
use crate::handles::Handles;
use crate::names::{TICKS_PER_HOUR, attr};
use crate::reflex;

/// Kind of the notice that a resident gets when a need falls below the low
/// mark.
pub const NEED_LOW: &str = "life.need_low";

/// Kind of the notice that a resident gets when food or energy reaches 0.
pub const NEED_EMPTY: &str = "life.need_empty";

/// Ticks in one game hour, as a signed number.
const HOUR: i64 = TICKS_PER_HOUR as i64;

/// The numbers of one pass.
struct Rates {
    food_decay: i64,
    energy_decay: i64,
    mood_decay: i64,
    low: i64,
    low_mood: i64,
    starve: i64,
    exhaust: i64,
    regen: i64,
    reflex_eat: i64,
    reflex_doze: i64,
}

impl Rates {
    fn read(realm: &Realm, h: &Handles) -> Rates {
        let reflexes = realm.param(h.p.reflexes) != 0;
        let when = |id| if reflexes { realm.param(id) } else { i64::MIN };
        Rates {
            food_decay: realm.param(h.p.food_decay),
            energy_decay: realm.param(h.p.energy_decay),
            mood_decay: realm.param(h.p.mood_decay),
            low: realm.param(h.p.need_low),
            low_mood: realm.param(h.p.low_need_mood),
            starve: realm.param(h.p.starve_health),
            exhaust: realm.param(h.p.exhaust_health),
            regen: realm.param(h.p.health_regen),
            reflex_eat: when(h.p.reflex_eat_below),
            reflex_doze: when(h.p.reflex_doze_below),
        }
    }

    /// Returns the needs of a resident one hour later.
    fn hour(&self, before: Needs, asleep: bool) -> Needs {
        let max = attr::NEED_MAX;
        let food = (before.food - self.food_decay).max(0);
        let energy =
            if asleep { before.energy } else { (before.energy - self.energy_decay).max(0) };
        let mut mood = before.mood - self.mood_decay;
        let mut health = before.health;
        if food < self.low {
            mood -= self.low_mood;
        }
        if !asleep && energy < self.low {
            mood -= self.low_mood;
        }
        if food == 0 {
            health -= self.starve;
        }
        if !asleep && energy == 0 {
            health -= self.exhaust;
        }
        if food >= self.low && energy >= self.low {
            health += self.regen;
        }
        Needs { food, energy, health: health.clamp(0, max), mood: mood.clamp(0, max) }
    }
}

/// The four needs of one resident.
#[derive(Clone, Copy)]
struct Needs {
    food: i64,
    energy: i64,
    health: i64,
    mood: i64,
}

/// A notice for a resident whose need crossed a mark.
struct Alert {
    idx: Idx,
    kind: &'static str,
    need: &'static str,
    value: i64,
}

/// What follows from the hours that a pass charged: notices, reflexes and
/// deaths. They happen after the pass, when no column is borrowed.
#[derive(Default)]
struct Effects {
    dead: Vec<(Idx, &'static str)>,
    alerts: Vec<Alert>,
    hungry: Vec<Idx>,
    tired: Vec<Idx>,
}

impl Effects {
    /// Notes what one hour did to a resident.
    fn note(&mut self, r: &Rates, idx: Idx, before: Needs, after: Needs, asleep: bool) {
        let mut alert = |kind, need, value| self.alerts.push(Alert { idx, kind, need, value });
        if after.food == 0 && before.food > 0 {
            alert(NEED_EMPTY, "food", after.food);
        } else if after.food < r.low && before.food >= r.low {
            alert(NEED_LOW, "food", after.food);
        }
        if !asleep && after.energy == 0 && before.energy > 0 {
            alert(NEED_EMPTY, "energy", after.energy);
        } else if !asleep && after.energy < r.low && before.energy >= r.low {
            alert(NEED_LOW, "energy", after.energy);
        }
        if after.health < r.low && before.health >= r.low {
            alert(NEED_LOW, "health", after.health);
        }
        if after.health == 0 {
            let cause = if after.food == 0 { "starvation" } else { "exhaustion" };
            self.dead.push((idx, cause));
            return;
        }
        if after.food < r.reflex_eat {
            self.hungry.push(idx);
        }
        if !asleep && after.energy < r.reflex_doze {
            self.tired.push(idx);
        }
    }

    /// Runs the reflexes, sends the notices and lets the dead die.
    fn apply(self, realm: &Realm, h: &Handles) {
        reflex::run(realm, h, &self.hungry, &self.tired);
        for alert in self.alerts {
            realm.notify(
                realm.id_of(alert.idx),
                alert.kind,
                None,
                json!({ "need": alert.need, "value": alert.value }),
            );
        }
        for (idx, cause) in self.dead {
            death::die(realm, idx, cause);
        }
    }
}

/// Returns the tick of the zone as a signed number.
fn now(realm: &Realm) -> i64 {
    i64::try_from(realm.tick().0).unwrap_or(i64::MAX)
}

/// Charges one hour on the clock of a resident. Returns false when less
/// than one hour passed since the last hour that the resident paid.
fn take_hour(body: &mut Body, tick: i64) -> bool {
    if tick.saturating_sub(body.needs_at) < HOUR {
        return false;
    }
    // A resident never owes two hours: it meets a pass or arrives in a zone
    // at least once in each hour. A larger debt can only come from state
    // that an older version wrote. It is not charged.
    body.needs_at = body.needs_at.saturating_add(HOUR).max(tick - (HOUR - 1));
    true
}

/// Starts the clock of a new resident. The first hourly pass of its zone is
/// its first hour.
pub(crate) fn start_clock(realm: &Realm, idx: Idx) {
    realm.col_mut::<Body>()[idx].needs_at = now(realm) - (HOUR - 1);
}

/// The hourly pass over every resident of the zone.
fn pass(realm: &Realm, h: &Handles) {
    let count = realm.len();
    if count == 0 {
        return;
    }
    let r = Rates::read(realm, h);
    let tick = now(realm);
    let mut effects = Effects::default();
    {
        let mut bodies = realm.col_mut::<Body>();
        let mut foods = realm.attrs_mut(h.food);
        let mut energies = realm.attrs_mut(h.energy);
        let mut healths = realm.attrs_mut(h.health);
        let mut moods = realm.attrs_mut(h.mood);
        for i in 0..count {
            let idx = Idx(i as u32);
            let before =
                Needs { food: foods[i], energy: energies[i], health: healths[i], mood: moods[i] };
            if before.health <= 0 {
                // Another module took the last health point.
                effects.dead.push((idx, "injury"));
                continue;
            }
            if !take_hour(&mut bodies[idx], tick) {
                // The resident paid this hour in another zone.
                continue;
            }
            if realm.has_condition(idx, h.hospitalized) {
                // The hospital looks after its patients.
                continue;
            }
            let asleep = realm.has_condition(idx, h.asleep);
            let after = r.hour(before, asleep);
            foods[i] = after.food;
            energies[i] = after.energy;
            healths[i] = after.health;
            moods[i] = after.mood;
            effects.note(&r, idx, before, after, asleep);
        }
    }
    effects.apply(realm, h);
}

/// A resident leaves for another zone. The two zones count their ticks
/// apart, so the clock changes to the ticks that are not paid yet.
fn departing(realm: &Realm, idx: Idx) {
    let tick = now(realm);
    let mut bodies = realm.col_mut::<Body>();
    let body = &mut bodies[idx];
    body.needs_at = tick.saturating_sub(body.needs_at);
}

/// A resident arrived from another zone. Its clock changes back to a tick
/// of this zone. When it owes an hour, it pays now: the pass of this zone
/// can be far away, and the next zone can be near.
fn arrived(realm: &Realm, h: &Handles, idx: Idx) {
    let tick = now(realm);
    let due = {
        let mut bodies = realm.col_mut::<Body>();
        let body = &mut bodies[idx];
        body.needs_at = tick.saturating_sub(body.needs_at);
        take_hour(body, tick)
    };
    if !due || realm.has_condition(idx, h.hospitalized) {
        return;
    }
    let before = Needs {
        food: realm.attr(idx, h.food),
        energy: realm.attr(idx, h.energy),
        health: realm.attr(idx, h.health),
        mood: realm.attr(idx, h.mood),
    };
    if before.health <= 0 {
        // The next pass of this zone lets the resident die.
        return;
    }
    let r = Rates::read(realm, h);
    let asleep = realm.has_condition(idx, h.asleep);
    let after = r.hour(before, asleep);
    realm.attr_set(idx, h.food, after.food);
    realm.attr_set(idx, h.energy, after.energy);
    realm.attr_set(idx, h.health, after.health);
    realm.attr_set(idx, h.mood, after.mood);
    let mut effects = Effects::default();
    effects.note(&r, idx, before, after, asleep);
    effects.apply(realm, h);
}

/// Registers the hourly pass and the hooks that keep the clock of a
/// resident right when it changes zone.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.every(TICKS_PER_HOUR, Stage::Pre, 0, "life.needs", move |realm| pass(realm, &h));
        b.on_depart(departing);
        b.on_arrive(move |realm, idx| arrived(realm, &h, idx));
    }
}
