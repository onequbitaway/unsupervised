//! Helpers for sites, and the index that finds a site of another district.
//!
//! A zone knows only its own sites. The global realm keeps one small entry
//! for every site of the world: its zone and a tile where a resident can
//! stand. `move_to` asks it when the site is not in the zone of the
//! resident.

use serde::{Deserialize, Serialize};
use unsup_kernel::{
    Builder, CityId, Dest, Idx, Message, Pos, Realm, Rect, ResidentId, Resource, Scope, Site,
    SiteId, TileMap, ZoneId,
};

use crate::names::tile;
use crate::walk;

/// Returns the site that covers the position of a resident, if any.
pub(crate) fn site_here(realm: &Realm, idx: Idx) -> Option<Site> {
    realm.sites().at(realm.pos(idx)).cloned()
}

/// Returns the walkable tile of a rectangle that is nearest to its centre:
/// the place where a resident stands when it walks "to the site". Ties go
/// to the tile that comes first row by row.
pub(crate) fn standing_tile(tiles: &TileMap, rect: Rect) -> Option<Pos> {
    let centre = rect.centre();
    let walkable = |pos: Pos| tiles.get(pos).is_some_and(tile::is_walkable);
    if walkable(centre) {
        return Some(centre);
    }
    let mut best: Option<(u32, Pos)> = None;
    for y in rect.y0..rect.y1 {
        for x in rect.x0..rect.x1 {
            let pos = Pos::new(x, y);
            let distance = pos.dist(centre);
            if walkable(pos) && best.is_none_or(|(least, _)| distance < least) {
                best = Some((distance, pos));
            }
        }
    }
    best.map(|(_, pos)| pos)
}

/// Returns the door tiles of a building: the walkable tiles of its outer
/// ring. Open land has no walls, so every tile of its ring counts; the
/// result is then empty, to keep it short.
pub(crate) fn doors(tiles: &TileMap, rect: Rect) -> Vec<Pos> {
    let mut ring = Vec::new();
    for x in rect.x0..rect.x1 {
        ring.push(Pos::new(x, rect.y0));
        ring.push(Pos::new(x, rect.y1 - 1));
    }
    for y in rect.y0 + 1..rect.y1 - 1 {
        ring.push(Pos::new(rect.x0, y));
        ring.push(Pos::new(rect.x1 - 1, y));
    }
    let walls = ring.iter().filter(|pos| tiles.get(**pos) == Some(tile::WALL)).count();
    if walls == 0 {
        return Vec::new();
    }
    ring.retain(|pos| tiles.get(*pos).is_some_and(tile::is_walkable));
    ring.sort_by_key(|pos| (pos.y, pos.x));
    ring.dedup();
    ring
}

/// Where a site is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Place {
    /// The zone of the site.
    pub zone: ZoneId,
    /// The city of the site.
    pub city: CityId,
    /// A tile of the site where a resident can stand.
    pub tile: Pos,
}

/// The place of every site of the world. It is a resource of the global
/// realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SiteIndex {
    /// Sorted by site id.
    places: Vec<(SiteId, Place)>,
}

impl SiteIndex {
    /// Returns the place of a site.
    #[must_use]
    pub fn get(&self, site: SiteId) -> Option<Place> {
        let slot = self.places.binary_search_by_key(&site, |(id, _)| *id).ok()?;
        Some(self.places[slot].1)
    }

    /// Returns the number of sites.
    #[must_use]
    pub fn len(&self) -> usize {
        self.places.len()
    }

    /// Returns true when the index has no site.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.places.is_empty()
    }
}

impl Resource for SiteIndex {
    const NAME: &'static str = "life.site_index";
}

/// A zone tells the global realm where its sites are.
#[derive(Debug, Serialize, Deserialize)]
struct SitesBuilt {
    city: CityId,
    sites: Vec<(SiteId, Pos)>,
}

impl Message for SitesBuilt {
    const KIND: &'static str = "life.sites_built";
}

/// A zone asks the global realm where a site is.
#[derive(Debug, Serialize, Deserialize)]
struct WhereIs {
    resident: ResidentId,
    action: u64,
    site: SiteId,
}

impl Message for WhereIs {
    const KIND: &'static str = "life.where_is";
}

/// The answer of the global realm: the place of a site, or nothing when no
/// site has the id.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct SiteAt {
    pub(crate) action: u64,
    pub(crate) site: SiteId,
    pub(crate) place: Option<Place>,
}

impl Message for SiteAt {
    const KIND: &'static str = "life.site_at";
}

/// Tells the global realm where the new sites of this zone are.
pub(crate) fn announce(realm: &Realm, ids: &[SiteId]) {
    let Some(info) = realm.zone_info() else {
        return;
    };
    let sites = realm.sites();
    let tiles = realm.tiles();
    let places: Vec<(SiteId, Pos)> = ids
        .iter()
        .filter_map(|id| sites.get(*id))
        .filter_map(|site| standing_tile(&tiles, site.rect).map(|tile| (site.id, tile)))
        .collect();
    if !places.is_empty() {
        realm.send(Dest::Global, &SitesBuilt { city: info.city, sites: places });
    }
}

/// Asks the global realm where a site is. The answer comes to the zone of
/// the resident as [`SiteAt`].
pub(crate) fn ask(realm: &Realm, resident: ResidentId, action: u64, site: SiteId) {
    realm.send(Dest::Global, &WhereIs { resident, action, site });
}

/// Registers the index and its messages.
pub(crate) fn register(b: &mut Builder) {
    match b.scope() {
        Scope::Global => {
            b.resource::<SiteIndex>();
            b.on_message::<SitesBuilt>(|realm, from, message| {
                let Some(zone) = from.zone() else {
                    return;
                };
                let mut index = realm.res_mut::<SiteIndex>();
                for (site, tile) in message.sites {
                    index.places.push((site, Place { zone, city: message.city, tile }));
                }
                index.places.sort_by_key(|(id, _)| *id);
                index.places.dedup_by_key(|(id, _)| *id);
            });
            b.on_message::<WhereIs>(|realm, _from, message| {
                let place = realm.res::<SiteIndex>().get(message.site);
                realm.send(
                    Dest::Resident(message.resident),
                    &SiteAt { action: message.action, site: message.site, place },
                );
            });
        }
        Scope::Zone => {
            b.on_resident_message::<SiteAt>(walk::site_found);
        }
    }
}
