//! The shelters of the world: where a newcomer finds its first bed.
//!
//! A newcomer can appear in a station, in a zone that has no shelter. So
//! every zone keeps a list of all shelters of the world. A zone with a
//! shelter tells all zones about it when the world is made.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Builder, CityId, Dest, Message, Pos, Realm, Resource, Scope, SiteId, ZoneId};

use crate::names::site;
use crate::sites::standing_tile;

/// One shelter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Shelter {
    /// The city of the shelter.
    pub city: CityId,
    /// The site.
    pub site: SiteId,
    /// The zone of the shelter.
    pub zone: ZoneId,
    /// A tile inside the shelter where a resident can stand.
    pub pos: Pos,
}

impl Message for Shelter {
    const KIND: &'static str = "life.shelter";
}

/// Every shelter of the world, sorted by city and site. Each zone has a
/// copy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shelters {
    list: Vec<Shelter>,
}

impl Shelters {
    /// Returns the shelters of a city.
    pub fn of_city(&self, city: CityId) -> impl Iterator<Item = &Shelter> + '_ {
        self.list.iter().filter(move |shelter| shelter.city == city)
    }

    /// Returns the shelter of a city that is nearest to a position. The
    /// lower site id wins a tie.
    #[must_use]
    pub fn nearest(&self, city: CityId, pos: Pos) -> Option<Shelter> {
        self.of_city(city).min_by_key(|shelter| (shelter.pos.dist(pos), shelter.site)).copied()
    }

    fn add(&mut self, shelter: Shelter) {
        if let Err(slot) = self.list.binary_search(&shelter) {
            self.list.insert(slot, shelter);
        }
    }
}

impl Resource for Shelters {
    const NAME: &'static str = "life.shelters";
}

/// Tells every zone about the shelters of this zone. World generation
/// calls it.
pub(crate) fn announce(realm: &Realm) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let shelters: Vec<Shelter> = {
        let sites = realm.sites();
        let tiles = realm.tiles();
        sites
            .of_kind(site::SHELTER)
            .filter_map(|place| {
                let pos = standing_tile(&tiles, place.rect)?;
                Some(Shelter { city: info.city, site: place.id, zone: info.id, pos })
            })
            .collect()
    };
    for shelter in shelters {
        realm.res_mut::<Shelters>().add(shelter);
        realm.send(Dest::AllZones, &shelter);
    }
}

/// Registers the list of shelters.
pub(crate) fn register(b: &mut Builder) {
    if b.scope() == Scope::Zone {
        b.resource::<Shelters>();
        b.on_message::<Shelter>(|realm, _from, shelter| {
            realm.res_mut::<Shelters>().add(shelter);
        });
    }
}
