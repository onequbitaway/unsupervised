//! What a new resident gets: full needs, its start age and a bed in a
//! shelter.

use unsup_kernel::{Builder, Idx, Realm, Scope};

use crate::body::{Bed, Body};
use crate::care::remember_bed;
use crate::handles::Handles;
use crate::names::{attr, site};
use crate::needs;
use crate::shelters::Shelters;

/// Sets up a new resident.
fn welcome(realm: &Realm, h: &Handles, idx: Idx) {
    for need in [h.food, h.energy, h.health, h.mood] {
        realm.attr_set(idx, need, attr::NEED_MAX);
    }
    realm.attr_set(idx, h.age, realm.param(h.p.spawn_age));
    needs::start_clock(realm, idx);

    // The bed: the shelter where the resident appeared, else the nearest
    // shelter of the district, else the nearest shelter of the city.
    let pos = realm.pos(idx);
    let shelter = {
        let sites = realm.sites();
        sites
            .of_kind(site::SHELTER)
            .min_by_key(|place| (place.rect.dist_to(pos), place.id))
            .cloned()
    };
    if let Some(shelter) = shelter {
        remember_bed(realm, idx, &shelter, false);
        return;
    }
    let far = realm.zone_info().and_then(|zone| realm.res::<Shelters>().nearest(zone.city, pos));
    let kind = site::ALL.iter().position(|kind| *kind == site::SHELTER);
    if let (Some(far), Some(kind)) = (far, kind) {
        realm.col_mut::<Body>()[idx].bed = Some(Bed {
            site: far.site,
            zone: far.zone,
            pos: far.pos,
            kind: kind as u8,
            good: false,
        });
    }
}

/// Registers the spawn hook.
pub(crate) fn register(b: &mut Builder, h: Handles) {
    if b.scope() == Scope::Zone {
        b.on_spawn(0, move |realm, idx| welcome(realm, &h, idx));
    }
}
