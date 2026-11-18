//! Where new residents appear: the spawn sites of each city.
//!
//! A new resident appears on a tile of a site with the tag `spawn` in its
//! city. The world asks for the list of these sites at every spawn. A
//! scan of every site of every zone of the city for each spawn is too slow
//! for a large world, so the list is kept here and read again only for a
//! zone whose sites were borrowed for a change since the last look.
//!
//! The cache is derived from the sites. It is not simulation state: it is
//! not in a snapshot, and a world with or without it puts every resident at
//! the same place.

use unsup_protocol::{Rect, ZoneId};

use crate::layout::CityInfo;
use crate::realm::Realm;
use crate::sites::Site;

/// The spawn sites of one zone, as the cache last read them.
#[derive(Clone, Debug, Default)]
struct ZoneSpawn {
    /// `Realm::sites_epoch` of the zone at the last read. `None`: the zone
    /// was never read.
    epoch: Option<u64>,
    /// The tiles of each site with the tag `spawn`, in site id order.
    sites: Vec<Rect>,
}

/// The spawn sites of one city.
#[derive(Clone, Debug, Default)]
struct CitySpawn {
    /// One entry per zone of the city, in the order of `CityInfo::zones`.
    zones: Vec<ZoneSpawn>,
    /// The sites of all hosted zones: in zone order, then in site id
    /// order.
    sites: Vec<(ZoneId, Rect)>,
}

/// The spawn sites of every city of a world.
#[derive(Clone, Debug, Default)]
pub(crate) struct SpawnSites {
    cities: Vec<CitySpawn>,
}

impl SpawnSites {
    /// Returns the places where a new resident of the city can appear: the
    /// zone and the tiles of every site with the tag `spawn` in a hosted
    /// zone of the city, in zone order and then in site id order.
    ///
    /// `zones` holds one slot per zone of the world, `None` for a zone that
    /// another process hosts. The cost is one number per zone of the city,
    /// plus one scan of each zone whose sites changed.
    pub(crate) fn of_city(
        &mut self,
        city: &CityInfo,
        zones: &[Option<Realm>],
    ) -> &[(ZoneId, Rect)] {
        let index = usize::from(city.id.0);
        if self.cities.len() <= index {
            self.cities.resize_with(index + 1, CitySpawn::default);
        }
        let cache = &mut self.cities[index];
        cache.zones.resize_with(city.zones.len(), ZoneSpawn::default);

        let mut changed = false;
        for (seen, &zone) in cache.zones.iter_mut().zip(&city.zones) {
            let Some(realm) = zones.get(zone.0 as usize).and_then(Option::as_ref) else {
                continue;
            };
            let epoch = realm.sites_epoch();
            if seen.epoch == Some(epoch) {
                continue;
            }
            seen.epoch = Some(epoch);
            // Most changes of a site do not make or move a spawn site.
            let sites = realm.sites();
            let rects = || sites.with_tag(Site::SPAWN_TAG).map(|site| site.rect);
            if !rects().eq(seen.sites.iter().copied()) {
                seen.sites.clear();
                seen.sites.extend(rects());
                changed = true;
            }
        }
        if changed {
            cache.sites.clear();
            for (seen, &zone) in cache.zones.iter().zip(&city.zones) {
                cache.sites.extend(seen.sites.iter().map(|&rect| (zone, rect)));
            }
        }
        &cache.sites
    }
}
