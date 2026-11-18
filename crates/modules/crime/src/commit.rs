//! The common end of every crime: the case, the trace, the witnesses, the
//! city watch and the public event.
//!
//! The perpetrator of a case is secret. The public event has no actor, and
//! its data does not name the perpetrator. A witness learns the name only
//! when it recognizes the perpetrator.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Dest, EventMeta, Holder, Idx, Pos, Realm, ResidentId, SiteId};

use crate::calc;
use crate::ids::Handles;
use crate::kinds::CrimeKind;
use crate::msg::{CaseFile, Order, Report};
use crate::names::{event, notice, perm};
use crate::powers;
use crate::record::{Caught, Known, Record, Role};
use crate::scene::{Scenes, Trace};

/// What happened, as the verb of the crime describes it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Deed {
    pub(crate) kind: CrimeKind,
    /// The resident that did it.
    pub(crate) perp: Idx,
    pub(crate) pos: Pos,
    pub(crate) site: Option<SiteId>,
    /// The holder that lost value. A restitution goes to it.
    pub(crate) victim: Option<Holder>,
    /// The resident that the crime touched in person. It gets a notice
    /// also when it sleeps.
    pub(crate) target: Option<ResidentId>,
    /// The resident that owns the site of the crime. It gets a notice
    /// wherever it is.
    pub(crate) owner: Option<ResidentId>,
    /// Cents that the victim lost.
    pub(crate) value: i64,
    /// True when the crime failed.
    pub(crate) failed: bool,
    /// True when the crime was done by force, face to face.
    pub(crate) force: bool,
}

/// What the perpetrator learns about the case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Filed {
    /// The case.
    pub(crate) case: u64,
    /// Residents that noticed the crime.
    pub(crate) witnesses: u32,
}

/// Who found a crime that no bystander can see, at the moment at which the
/// case is filed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Finder {
    /// Nobody yet. The city watch gets its chance.
    Nobody,
    /// An officer found it, for example with a search.
    Officer(ResidentId),
    /// The city watch found it.
    Watch,
}

/// Returns the id of a new case.
pub(crate) fn new_case(realm: &Realm) -> u64 {
    realm.alloc_id()
}

/// Returns the chance in basis points that a resident recognizes the
/// perpetrator.
fn identify_chance(realm: &Realm, h: &Handles, deed: &Deed, seer: &Seer, stealth: i64) -> u32 {
    let p = &h.p;
    if seer.on_patrol {
        let raw = realm.param(p.patrol_detect) - stealth * realm.param(p.patrol_stealth);
        return calc::plain_chance(raw);
    }
    let mut raw = realm.param(p.identify_base)
        - stealth * realm.param(p.identify_stealth)
        - i64::from(seer.distance) * realm.param(p.identify_distance);
    if calc::is_night(realm, h) {
        raw -= realm.param(p.identify_night);
    }
    if deed.failed {
        raw += realm.param(p.identify_fail);
    }
    if deed.force && seer.is_target {
        raw += realm.param(p.identify_force);
    }
    if seer.asleep {
        raw -= realm.param(p.identify_asleep);
    }
    calc::plain_chance(raw)
}

/// A resident that can notice the crime.
struct Seer {
    idx: Idx,
    id: ResidentId,
    distance: u32,
    asleep: bool,
    is_target: bool,
    /// The resident holds `law.arrest` or `law.search` in this city.
    police: bool,
    /// The resident holds `law.arrest` in this city.
    can_arrest: bool,
    on_patrol: bool,
}

/// Returns the residents that can notice the crime: the nearest first,
/// then the lower id. A resident that sleeps notices only a crime against
/// itself. The list has at most `crime.witness_max` residents, and always
/// the resident that the crime touched.
fn seers(realm: &Realm, h: &Handles, deed: &Deed) -> Vec<Seer> {
    let city = powers::city_of(realm);
    let perp = realm.id_of(deed.perp);
    let radius = h.tiles(realm, h.p.witness_radius);
    let patrol_radius = h.tiles(realm, h.p.patrol_radius);
    let mut found: Vec<Seer> = realm
        .nearby(deed.pos, radius.max(patrol_radius))
        .into_iter()
        .filter_map(|idx| {
            let id = realm.id_of(idx);
            if id == perp {
                return None;
            }
            let distance = realm.pos(idx).dist(deed.pos);
            let asleep = realm.has_condition(idx, h.asleep);
            let is_target = deed.target == Some(id);
            let can_arrest = powers::has_power(realm, idx, perm::ARREST, city);
            let police = can_arrest || powers::has_power(realm, idx, perm::SEARCH, city);
            let on_patrol = police
                && !asleep
                && distance <= patrol_radius
                && realm.activity(idx).is_some_and(|activity| activity.verb == "patrol");
            let sees = is_target || on_patrol || (!asleep && distance <= radius);
            sees.then_some(Seer {
                idx,
                id,
                distance,
                asleep,
                is_target,
                police,
                can_arrest,
                on_patrol,
            })
        })
        .collect();
    found.sort_by_key(|seer| (!seer.is_target, seer.distance, seer.id));
    found.truncate(h.count(realm, h.p.witness_max).max(1));
    found
}

/// Lets the residents around a crime notice it. Returns how many noticed
/// it and whether an officer recognized the perpetrator.
fn witnesses(realm: &Realm, h: &Handles, case: u64, deed: &Deed, stealth: i64) -> (u32, bool) {
    let perp = realm.id_of(deed.perp);
    let frame = realm.frame();
    let known_max = h.count(realm, h.p.known_max);
    let caught_until = frame.saturating_add(h.frames(realm, h.p.caught_ticks));
    let mut caught = false;
    let mut count = 0u32;
    for seer in seers(realm, h, deed) {
        let chance = identify_chance(realm, h, deed, &seer, stealth);
        let identified = calc::roll(realm, chance);
        let role = if seer.is_target || deed.owner == Some(seer.id) {
            Role::Victim
        } else {
            Role::Witness
        };
        let suspect = identified.then_some(perp);
        realm.col_mut::<Record>()[seer.idx]
            .add_known(Known { case, kind: deed.kind, role, suspect, reported: false }, known_max);
        // An officer that saw who did it is strong evidence. With
        // `law.arrest`, that officer can arrest without a warrant.
        caught |= identified && seer.police;
        let can_arrest = identified && seer.can_arrest;
        if can_arrest {
            realm.col_mut::<Record>()[deed.perp]
                .add_caught(Caught { case, officer: seer.id, until: caught_until }, known_max);
        }
        let mut data = Map::new();
        data.insert("case".into(), json!(case));
        data.insert("kind".into(), json!(deed.kind));
        data.insert("role".into(), json!(role.name()));
        data.insert("pos".into(), json!(deed.pos));
        if let Some(site) = deed.site {
            data.insert("site".into(), json!(site));
        }
        if role == Role::Victim {
            data.insert("value".into(), json!(deed.value));
        }
        if deed.failed {
            data.insert("attempt".into(), json!(true));
        }
        if let Some(suspect) = suspect {
            data.insert("suspect".into(), json!(suspect));
        }
        if can_arrest {
            data.insert("can_arrest".into(), json!(true));
        }
        let kind = if seer.on_patrol { notice::PATROL } else { notice::WITNESSED };
        realm.notify(seer.id, kind, None, Value::Object(data));
        count += 1;
    }
    (count, caught)
}

/// Emits the public event of a crime. The event has no actor.
fn announce(realm: &Realm, h: &Handles, case: u64, deed: &Deed) {
    let city = powers::city_of(realm);
    let param = if deed.failed {
        h.p.drama_attempt
    } else if deed.value >= realm.param(h.p.heist_value) {
        h.p.drama_heist
    } else {
        h.kind(deed.kind).drama
    };
    let drama = h.drama(realm, param);
    let (kind, data) = match deed.kind {
        CrimeKind::Theft | CrimeKind::Burglary | CrimeKind::Robbery => (
            event::THEFT,
            json!({
                "case": case, "kind": deed.kind, "city": city, "site": deed.site,
                "value": deed.value, "attempt": deed.failed,
            }),
        ),
        CrimeKind::Assault | CrimeKind::Homicide => (
            event::ASSAULT,
            json!({
                "case": case, "kind": deed.kind, "city": city, "victim": deed.target,
                "fatal": deed.kind == CrimeKind::Homicide, "attempt": deed.failed,
            }),
        ),
        CrimeKind::Vandalism => (
            event::VANDALISM,
            json!({
                "case": case, "city": city, "site": deed.site, "damage": deed.value,
                "attempt": deed.failed,
            }),
        ),
        CrimeKind::Smuggling => {
            (event::CONTRABAND, json!({ "case": case, "city": city, "value": deed.value }))
        }
        // A bribe is secret.
        CrimeKind::Bribery => return,
    };
    realm.emit(EventMeta::public(kind, drama).at(deed.pos), &data);
}

/// Files a crime: it leaves a trace, lets the residents around it notice
/// it, gives the city watch its chance, tells the case registry and emits
/// the public event.
///
/// With a finder that is an officer, the case starts with the evidence of
/// a perpetrator that was caught in the act, and that officer can arrest
/// the perpetrator.
pub(crate) fn file(realm: &Realm, h: &Handles, case: u64, deed: &Deed, finder: Finder) -> Filed {
    let Some(zone) = realm.zone_info() else {
        return Filed { case, witnesses: 0 };
    };
    let perp = realm.id_of(deed.perp);
    let frame = realm.frame();
    let stealth = calc::level(realm, deed.perp, h.stealth);
    let hidden = deed.kind.is_hidden();

    let trace = Trace { case, kind: deed.kind, pos: deed.pos, frame, stealth, read_by: Vec::new() };
    realm.res_mut::<Scenes>().add(trace, h.count(realm, h.p.traces_max));

    let (witnesses, mut caught) = if deed.kind.has_witnesses() {
        witnesses(realm, h, case, deed, stealth)
    } else {
        (0, false)
    };
    if let Finder::Officer(officer) = finder {
        caught = true;
        let until = frame.saturating_add(h.frames(realm, h.p.caught_ticks));
        let max = h.count(realm, h.p.known_max);
        realm.col_mut::<Record>()[deed.perp].add_caught(Caught { case, officer, until }, max);
    }
    // The owner of the site learns of the crime wherever it is.
    if let (Some(owner), Some(site)) = (deed.owner, deed.site)
        && owner != perp
    {
        let order = Order::Heard { case, kind: deed.kind, site, value: deed.value };
        realm.send(Dest::Resident(owner), &order);
    }
    // The city watch: a weak chance to see every crime that is not secret.
    let watch = match finder {
        Finder::Watch => true,
        Finder::Officer(_) => false,
        Finder::Nobody => {
            !hidden
                && !caught
                && calc::roll(realm, calc::plain_chance(realm.param(h.p.watch_detect)))
        }
    };

    realm.send(
        Dest::Global,
        &Report::Opened(CaseFile {
            id: case,
            frame,
            kind: deed.kind,
            city: zone.city,
            zone: zone.id,
            pos: deed.pos,
            site: deed.site,
            perp,
            victim: deed.victim,
            value: deed.value,
            attempt: deed.failed,
            witnesses,
            caught,
            watch,
        }),
    );
    if !hidden {
        announce(realm, h, case, deed);
    }
    Filed { case, witnesses }
}
