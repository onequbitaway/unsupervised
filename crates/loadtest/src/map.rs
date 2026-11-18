//! The map of a world: the tiles and the sites of each city.
//!
//! A walk target must be a tile on which a resident can stand, or a site.
//! The load tools read the map one time when a run starts. The commands
//! that run in one process read it from `World::view`. The swarm reads the
//! same views from `GET /v1/world` and `GET /v1/world/zones/{id}`.
//!
//! The life module makes the map so that a resident can reach each walkable
//! tile of a zone on foot (`docs/design/modules/life.md`, section 1.5), and
//! the edge of each zone is a road. Thus a walkable tile in the city of a
//! resident is a tile that the resident can reach.

use std::collections::BTreeMap;
use std::fmt;

use unsup_kernel::{Rng, World};
use unsup_protocol::{CityId, Pos, Rect, SiteId, ViewQuery, WorldView, ZoneId};

use crate::defaults::defaults;
use crate::random;

/// A map could not be built from the views of a world.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapError {
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cannot build the map of the world: {}", self.message)
    }
}

impl std::error::Error for MapError {}

fn invalid(message: impl Into<String>) -> MapError {
    MapError { message: message.into() }
}

/// A site on the map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapSite {
    /// Id of the site.
    pub id: SiteId,
    /// Kind of the site, for example `depot`.
    pub kind: String,
    /// Tiles that the site covers.
    pub rect: Rect,
    /// The zone of the site.
    pub zone: ZoneId,
}

/// The tiles of one zone.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ZoneTiles {
    bounds: Rect,
    /// Tile kinds, row by row.
    tiles: Vec<u8>,
}

impl ZoneTiles {
    fn get(&self, pos: Pos) -> Option<u8> {
        if !self.bounds.contains(pos) {
            return None;
        }
        let col = pos.x.abs_diff(self.bounds.x0) as usize;
        let row = pos.y.abs_diff(self.bounds.y0) as usize;
        self.tiles.get(row * self.bounds.width() as usize + col).copied()
    }
}

/// The map of one city.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CityMap {
    id: CityId,
    bounds: Rect,
    /// The zones of the city, row by row.
    zones: Vec<ZoneTiles>,
    /// Zones in one row of the city, when all zones have the same size.
    columns: usize,
    sites: Vec<MapSite>,
    /// Places in `sites`, for each kind.
    by_kind: BTreeMap<String, Vec<u32>>,
}

impl CityMap {
    /// Returns the id of the city.
    #[must_use]
    pub fn id(&self) -> CityId {
        self.id
    }

    /// Returns the tiles that the city covers.
    #[must_use]
    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    /// Returns every site of the city.
    #[must_use]
    pub fn sites(&self) -> &[MapSite] {
        &self.sites
    }

    fn zone_at(&self, pos: Pos) -> Option<&ZoneTiles> {
        // The zones of a city make a grid. The grid gives the zone at once;
        // the search is for a city whose zones do not have one size.
        let first = self.zones.first()?;
        let (width, height) = (first.bounds.width().max(1), first.bounds.height().max(1));
        if self.bounds.contains(pos) && self.columns > 0 {
            let col = (pos.x.abs_diff(self.bounds.x0) / width) as usize;
            let row = (pos.y.abs_diff(self.bounds.y0) / height) as usize;
            if let Some(zone) = self.zones.get(row * self.columns + col)
                && zone.bounds.contains(pos)
            {
                return Some(zone);
            }
        }
        self.zones.iter().find(|zone| zone.bounds.contains(pos))
    }

    /// Returns the kind of the tile at `pos`, or `None` outside the city.
    #[must_use]
    pub fn tile(&self, pos: Pos) -> Option<u8> {
        self.zone_at(pos).and_then(|zone| zone.get(pos))
    }

    /// Returns true when a resident can stand on the tile at `pos`. A tile
    /// outside the city is not walkable.
    #[must_use]
    pub fn walkable(&self, pos: Pos) -> bool {
        self.tile(pos).is_some_and(|kind| !defaults().world.blocked_tiles.contains(&kind))
    }

    /// Returns the site that covers `pos`.
    #[must_use]
    pub fn site_at(&self, pos: Pos) -> Option<&MapSite> {
        self.sites.iter().find(|site| site.rect.contains(pos))
    }

    /// Returns true when the city has a site of the kind.
    #[must_use]
    pub fn has_kind(&self, kind: &str) -> bool {
        self.by_kind.contains_key(kind)
    }

    /// Returns the sites of the given kinds that are nearest to `from`, the
    /// nearest first, at most `count`. Two sites at the same distance come
    /// in id order.
    #[must_use]
    pub fn nearest(&self, kinds: &[String], from: Pos, count: usize) -> Vec<&MapSite> {
        let mut best: Vec<(u32, SiteId, &MapSite)> = Vec::with_capacity(count + 1);
        let places = kinds.iter().filter_map(|kind| self.by_kind.get(kind)).flatten();
        for &place in places {
            let site = &self.sites[place as usize];
            let key = (site.rect.dist_to(from), site.id);
            if best.len() == count && best.last().is_some_and(|last| (last.0, last.1) <= key) {
                continue;
            }
            let at = best.partition_point(|entry| (entry.0, entry.1) < key);
            best.insert(at, (key.0, key.1, site));
            best.truncate(count);
        }
        best.into_iter().map(|(_, _, site)| site).collect()
    }

    /// Returns a random walkable tile within `radius` tiles of `from` on
    /// each axis, inside the city. Returns `None` when `attempts` tries
    /// found no such tile.
    pub fn walkable_near(
        &self,
        rng: &mut Rng,
        from: Pos,
        radius: u32,
        attempts: u32,
    ) -> Option<Pos> {
        let b = self.bounds;
        if b.is_empty() {
            return None;
        }
        let radius = i64::from(radius);
        let span = |at: i32, first: i32, past: i32| {
            let (first, last) = (i64::from(first), i64::from(past) - 1);
            (
                (i64::from(at) - radius).clamp(first, last),
                (i64::from(at) + radius).clamp(first, last),
            )
        };
        let (x_lo, x_hi) = span(from.x, b.x0, b.x1);
        let (y_lo, y_hi) = span(from.y, b.y0, b.y1);
        for _ in 0..attempts.max(1) {
            let x = i32::try_from(random::int_between(rng, x_lo, x_hi)).ok()?;
            let y = i32::try_from(random::int_between(rng, y_lo, y_hi)).ok()?;
            let pos = Pos::new(x, y);
            if self.walkable(pos) {
                return Some(pos);
            }
        }
        None
    }

    /// Returns a random walkable tile of the city. Returns `None` when
    /// `attempts` tries found none.
    pub fn walkable_anywhere(&self, rng: &mut Rng, attempts: u32) -> Option<Pos> {
        let b = self.bounds;
        if b.is_empty() {
            return None;
        }
        for _ in 0..attempts.max(1) {
            let x = i32::try_from(rng.range(i64::from(b.x0), i64::from(b.x1))).ok()?;
            let y = i32::try_from(rng.range(i64::from(b.y0), i64::from(b.y1))).ok()?;
            let pos = Pos::new(x, y);
            if self.walkable(pos) {
                return Some(pos);
            }
        }
        None
    }
}

/// The map of a world: one [`CityMap`] for each city.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorldMap {
    /// The cities, in ascending id order.
    cities: Vec<CityMap>,
}

impl WorldMap {
    /// Builds the map from the layout of the world and the view of each
    /// zone, as `World::view` and the routes `GET /v1/world` and
    /// `GET /v1/world/zones/{id}` give them.
    ///
    /// # Errors
    ///
    /// Fails when a view is of the wrong kind, when a zone of the layout
    /// has no view, or when the tiles of a zone do not fit its size.
    pub fn from_views(layout: &WorldView, zones: Vec<WorldView>) -> Result<WorldMap, MapError> {
        let WorldView::Layout { cities, .. } = layout else {
            return Err(invalid("the first view is not the layout of the world"));
        };
        let mut by_zone = BTreeMap::new();
        for view in zones {
            let WorldView::Zone { zone, width, height, tiles, sites, .. } = view else {
                return Err(invalid("a view of a zone is the layout of the world"));
            };
            let expected = width as usize * height as usize;
            if tiles.len() != expected
                || zone.bounds.width() != width
                || zone.bounds.height() != height
            {
                return Err(invalid(format!(
                    "zone {} has {} tiles for {width} columns and {height} rows",
                    zone.id,
                    tiles.len()
                )));
            }
            by_zone.insert(zone.id, (zone.bounds, tiles, sites));
        }
        let mut maps = Vec::with_capacity(cities.len());
        for city in cities {
            let mut map = CityMap {
                id: city.id,
                bounds: city.bounds,
                zones: Vec::with_capacity(city.zones.len()),
                columns: 0,
                sites: Vec::new(),
                by_kind: BTreeMap::new(),
            };
            for id in &city.zones {
                let (bounds, tiles, sites) = by_zone
                    .remove(id)
                    .ok_or_else(|| invalid(format!("zone {id} of city {} has no view", city.id)))?;
                for site in sites {
                    let place = u32::try_from(map.sites.len())
                        .map_err(|_| invalid("a city has too many sites"))?;
                    map.by_kind.entry(site.kind.clone()).or_default().push(place);
                    map.sites.push(MapSite {
                        id: site.id,
                        kind: site.kind,
                        rect: site.rect,
                        zone: *id,
                    });
                }
                map.zones.push(ZoneTiles { bounds, tiles });
            }
            let width = map.zones.first().map_or(0, |zone| zone.bounds.width());
            map.columns = city.bounds.width().checked_div(width).unwrap_or(0) as usize;
            maps.push(map);
        }
        maps.sort_by_key(|city| city.id);
        Ok(WorldMap { cities: maps })
    }

    /// Reads the map of a world in this process through `World::view`.
    ///
    /// # Errors
    ///
    /// Fails when the world does not host one of its zones, or like
    /// [`WorldMap::from_views`].
    pub fn of_world(world: &World) -> Result<WorldMap, MapError> {
        let failed = |what: String, error| invalid(format!("cannot read {what}: {error}"));
        let layout = world
            .view(&ViewQuery::Layout)
            .map_err(|error| failed("the layout".to_string(), error))?;
        let mut zones = Vec::with_capacity(world.layout().zones().len());
        for zone in world.layout().zones() {
            let view = world
                .view(&ViewQuery::Zone { zone: zone.id })
                .map_err(|error| failed(format!("zone {}", zone.id), error))?;
            zones.push(view);
        }
        WorldMap::from_views(&layout, zones)
    }

    /// Returns the map of one city.
    #[must_use]
    pub fn city(&self, id: CityId) -> Option<&CityMap> {
        self.cities.binary_search_by_key(&id, |city| city.id).ok().map(|at| &self.cities[at])
    }

    /// Returns the map of each city, in ascending id order.
    #[must_use]
    pub fn cities(&self) -> &[CityMap] {
        &self.cities
    }

    /// Returns the city that covers `pos`.
    #[must_use]
    pub fn city_at(&self, pos: Pos) -> Option<&CityMap> {
        self.cities.iter().find(|city| city.bounds.contains(pos))
    }

    /// Returns true when a city of the world has a site of the kind.
    #[must_use]
    pub fn has_kind(&self, kind: &str) -> bool {
        self.cities.iter().any(|city| city.has_kind(kind))
    }
}
