//! Where the jail, the police stations and the courthouse of each city
//! are.
//!
//! An arrest moves the suspect to the jail of the city, and a search sends
//! what it takes to a police station. Those sites can be in another zone.
//! So every zone keeps a list of the law sites of the world. A zone tells
//! all zones about its own law sites when the world is made, and again
//! when they change.

use serde::{Deserialize, Serialize};
use unsup_kernel::{
    Builder, CityId, Dest, Message, Pos, Realm, Rect, Resource, Scope, SiteId, Stage, TileMap,
    ZoneId,
};
use unsup_life::names::{TICKS_PER_HOUR, site, tile};

/// The kinds of site that the module needs to find.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LawKind {
    /// The jail of a city.
    Jail,
    /// A police station.
    Station,
    /// The courthouse of a city.
    Courthouse,
}

impl LawKind {
    fn of(kind: &str) -> Option<LawKind> {
        match kind {
            site::JAIL => Some(LawKind::Jail),
            site::POLICE_STATION => Some(LawKind::Station),
            site::COURTHOUSE => Some(LawKind::Courthouse),
            _ => None,
        }
    }
}

/// One law site.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LawSite {
    /// The city of the site.
    pub city: CityId,
    /// What the site is.
    pub kind: LawKind,
    /// The site.
    pub site: SiteId,
    /// The zone of the site.
    pub zone: ZoneId,
    /// A tile inside the site where a resident can stand.
    pub pos: Pos,
}

/// A zone tells all zones which law sites it has now.
#[derive(Debug, Serialize, Deserialize)]
struct LawSitesOf {
    zone: ZoneId,
    sites: Vec<LawSite>,
}

impl Message for LawSitesOf {
    const KIND: &'static str = "crime.law_sites";
}

/// Every law site of the world, sorted by city, kind and site. Each zone
/// has a copy.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LawSites {
    list: Vec<LawSite>,
}

impl Resource for LawSites {
    const NAME: &'static str = "crime.law_sites";
}

impl LawSites {
    /// Returns the jail of a city: the one with the lowest site id.
    #[must_use]
    pub fn jail(&self, city: CityId) -> Option<LawSite> {
        self.list.iter().find(|s| s.city == city && s.kind == LawKind::Jail).copied()
    }

    /// Returns the police station of a city that is nearest to a position.
    /// The lower site id wins a tie.
    #[must_use]
    pub fn station(&self, city: CityId, pos: Pos) -> Option<LawSite> {
        self.list
            .iter()
            .filter(|s| s.city == city && s.kind == LawKind::Station)
            .min_by_key(|s| (s.pos.dist(pos), s.site))
            .copied()
    }

    /// Returns the courthouse of a city: the one with the lowest site id.
    #[must_use]
    pub fn courthouse(&self, city: CityId) -> Option<LawSite> {
        self.list.iter().find(|s| s.city == city && s.kind == LawKind::Courthouse).copied()
    }

    /// Returns the law sites of one zone, in the order of the list.
    fn of_zone(&self, zone: ZoneId) -> Vec<LawSite> {
        self.list.iter().filter(|s| s.zone == zone).copied().collect()
    }

    /// Replaces the law sites of one zone.
    fn replace(&mut self, zone: ZoneId, sites: &[LawSite]) {
        self.list.retain(|s| s.zone != zone);
        self.list.extend_from_slice(sites);
        self.list.sort_unstable();
    }
}

/// Returns the walkable tile of a rectangle that is nearest to its centre.
/// Ties go to the tile that comes first row by row.
fn standing_tile(tiles: &TileMap, rect: Rect) -> Option<Pos> {
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

/// Returns the law sites of this zone as they are now, sorted.
fn survey(realm: &Realm) -> Option<(ZoneId, Vec<LawSite>)> {
    let info = realm.zone_info()?;
    let sites = realm.sites();
    let tiles = realm.tiles();
    let mut found: Vec<LawSite> = sites
        .iter()
        .filter_map(|place| {
            let kind = LawKind::of(&place.kind)?;
            let pos = standing_tile(&tiles, place.rect)?;
            Some(LawSite { city: info.city, kind, site: place.id, zone: info.id, pos })
        })
        .collect();
    found.sort_unstable();
    Some((info.id, found))
}

/// Tells every zone the law sites of this zone when they are not what the
/// list says.
fn announce(realm: &Realm) {
    let Some((zone, found)) = survey(realm) else {
        return;
    };
    if realm.res::<LawSites>().of_zone(zone) == found {
        return;
    }
    realm.res_mut::<LawSites>().replace(zone, &found);
    realm.send(Dest::AllZones, &LawSitesOf { zone, sites: found });
}

/// Registers the list of law sites.
pub(crate) fn register(b: &mut Builder) {
    if b.scope() != Scope::Zone {
        return;
    }
    b.resource::<LawSites>();
    // After the life module made the sites (order 0).
    b.on_worldgen(1_000, |realm, _| announce(realm));
    // Another module can add or change a site later.
    b.every(TICKS_PER_HOUR, Stage::Post, 0, "crime.law_sites", announce);
    b.on_message::<LawSitesOf>(|realm, _from, news| {
        realm.res_mut::<LawSites>().replace(news.zone, &news.sites);
    });
}
