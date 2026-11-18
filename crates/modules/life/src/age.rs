//! Age and death of old age.
//!
//! One pass for each zone every game day sets the age of every resident
//! from the calendar. From the age `life.old_age_from`, each day has a
//! chance of death that grows with every year.
//!
//! A resident has one chance in each calendar day (`Body::rolled_day`). A
//! resident that arrives from another zone gets its age and the chance of
//! the day at once, so a resident cannot keep away from the pass of every
//! zone.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Builder, EventMeta, Idx, Realm, Rng, Scope, Stage};

use crate::body::Body;
use crate::death;
use crate::handles::Handles;
use crate::names::TICKS_PER_DAY;

/// One in this many is one basis point.
const BASIS_POINTS: u64 = 10_000;

/// The numbers of the age rules.
struct Rules {
    /// Frames in one year.
    year: u64,
    days_per_year: u64,
    /// Age of a new resident.
    start: i64,
    /// First age with a chance of death.
    old: i64,
    /// Chance of death in the first old year, in basis points.
    chance: i64,
    /// Basis points that the chance grows with each further year.
    rise: i64,
    frame: u64,
    /// The calendar day, from 1.
    today: u32,
}

impl Rules {
    fn read(realm: &Realm, h: &Handles) -> Rules {
        let days_per_year = u64::from(realm.layout().days_per_year().max(1));
        let frame = realm.frame();
        Rules {
            year: days_per_year * u64::from(TICKS_PER_DAY),
            days_per_year,
            start: realm.param(h.p.spawn_age),
            old: realm.param(h.p.old_age_from),
            chance: realm.param(h.p.old_age_death_bp),
            rise: realm.param(h.p.old_age_death_rise_bp),
            frame,
            today: u32::try_from(frame / u64::from(TICKS_PER_DAY)).unwrap_or(u32::MAX - 1) + 1,
        }
    }

    /// Returns the age of a resident that was born in the frame `born`.
    fn age(&self, born: u64) -> i64 {
        let lived = self.frame.saturating_sub(born) / self.year;
        self.start.saturating_add(i64::try_from(lived).unwrap_or(i64::MAX))
    }

    /// Gives a resident of an old age its chance of this calendar day to
    /// die. Returns true when it dies. A second call on the same day does
    /// nothing.
    fn roll(&self, body: &mut Body, rng: &mut Rng, age: i64) -> bool {
        if age < self.old || body.rolled_day >= self.today {
            return false;
        }
        body.rolled_day = self.today;
        // The chance of one year, spread over the days of the year.
        let per_year = self.chance.saturating_add(self.rise.saturating_mul(age - self.old));
        let per_year = u64::try_from(per_year.max(0)).unwrap_or(0);
        rng.chance(per_year, BASIS_POINTS * self.days_per_year)
    }
}

/// Tells the world about birthdays and lets the dead die.
fn conclude(realm: &Realm, h: &Handles, birthdays: &[(Idx, i64)], dead: &[Idx]) {
    let drama = h.drama(realm, h.p.drama_birthday);
    for &(idx, age) in birthdays {
        let resident = realm.id_of(idx);
        realm.emit(
            EventMeta::public("life.birthday_reached", drama).actor(resident).at(realm.pos(idx)),
            &json!({ "resident": resident, "age": age }),
        );
    }
    for &idx in dead {
        death::die(realm, idx, "old_age");
    }
}

/// The daily pass over every resident of the zone.
fn pass(realm: &Realm, h: &Handles) {
    if realm.is_empty() {
        return;
    }
    let rules = Rules::read(realm, h);
    let mut birthdays: Vec<(Idx, i64)> = Vec::new();
    let mut dead: Vec<Idx> = Vec::new();
    {
        let mut ages = realm.attrs_mut(h.age);
        let mut bodies = realm.col_mut::<Body>();
        let mut rng = realm.rng();
        for (i, slot) in ages.iter_mut().enumerate() {
            let idx = Idx(i as u32);
            let age = rules.age(realm.identity(idx).born);
            if age > *slot {
                *slot = age;
                birthdays.push((idx, age));
            }
            if rules.roll(&mut bodies[idx], &mut rng, age) {
                dead.push(idx);
            }
        }
    }
    conclude(realm, h, &birthdays, &dead);
}

/// A resident arrived from another zone. The daily pass of the zone that
/// it left may not have seen it, and the pass of this zone may not see it
/// either. So it gets its age and its chance of the day now.
fn arrived(realm: &Realm, h: &Handles, idx: Idx) {
    let rules = Rules::read(realm, h);
    let age = rules.age(realm.identity(idx).born);
    let birthday = age > realm.attr(idx, h.age);
    if birthday {
        realm.attr_set(idx, h.age, age);
    }
    let dead = {
        let mut bodies = realm.col_mut::<Body>();
        let mut rng = realm.rng();
        rules.roll(&mut bodies[idx], &mut rng, age)
    };
    if birthday || dead {
        let birthdays: &[(Idx, i64)] = if birthday { &[(idx, age)] } else { &[] };
        let dead: &[Idx] = if dead { &[idx] } else { &[] };
        conclude(realm, h, birthdays, dead);
    }
}

/// Registers the daily pass and the hook for residents that change zone.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.every(TICKS_PER_DAY, Stage::Pre, 10, "life.age", move |realm| pass(realm, &h));
        b.on_arrive(move |realm, idx| arrived(realm, &h, idx));
    }
}
