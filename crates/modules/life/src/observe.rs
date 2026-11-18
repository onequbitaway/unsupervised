//! The observation sections of the life module: `home` and `skills`.

use unsup_kernel::serde_json::{Map, Value, json};
use unsup_kernel::{Builder, Idx, ObserveOpts, Realm, Scope};

use crate::body::Body;
use crate::care::{SleepQuality, sleep_quality_in};
use crate::handles::Handles;
use crate::names::{SKILLS, site};
use crate::skills::{skill_level, xp_for_level};

/// Order of the sections among those of other modules.
const SECTION_ORDER: i32 = 0;

/// The section `home`: the place where the resident sleeps, and how well it
/// sleeps there. It is absent when the resident has no such place.
fn home(realm: &Realm, idx: Idx) -> Option<Value> {
    let bed = realm.col::<Body>()[idx].bed()?;
    let resident = realm.id_of(idx);
    let here = realm.zone_info().map(|zone| zone.id);
    let mut quality = if bed.good { SleepQuality::Good } else { SleepQuality::Fair };
    let mut name = None;
    if here == Some(bed.zone) {
        // The site is in this zone: say what is true now.
        let sites = realm.sites();
        let place = sites.get(bed.site)?;
        quality = sleep_quality_in(place, resident);
        name = Some(place.name.clone());
    }
    let mut out = Map::new();
    out.insert("site".into(), json!(bed.site));
    out.insert("kind".into(), json!(site::ALL.get(usize::from(bed.kind))));
    if let Some(name) = name {
        out.insert("name".into(), json!(name));
    }
    out.insert("zone".into(), json!(bed.zone));
    out.insert("pos".into(), json!(bed.pos));
    out.insert("sleep".into(), json!(quality.name()));
    Some(Value::Object(out))
}

/// The section `skills`: the level of each skill above level 0. With full
/// detail, also the experience and the experience of the next level. It is
/// absent when every skill is at level 0.
fn skills(realm: &Realm, h: &Handles, idx: Idx, opts: &ObserveOpts) -> Option<Value> {
    let mut out = Map::new();
    for (name, attr) in SKILLS.iter().zip(h.skills) {
        let xp = realm.attr(idx, attr);
        let level = skill_level(xp);
        if level == 0 {
            continue;
        }
        let entry = if opts.is_full() {
            json!({ "level": level, "xp": xp, "next": xp_for_level(level + 1) })
        } else {
            json!(level)
        };
        out.insert((*name).to_string(), entry);
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

/// Registers the two sections.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.observe("home", SECTION_ORDER, |realm, idx, _opts| home(realm, idx));
        b.observe("skills", SECTION_ORDER, move |realm, idx, opts| skills(realm, &h, idx, opts));
    }
}
