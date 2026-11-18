//! The layout of the world: cities and zones.

use unsup_protocol::{CityId, Clock, Pos, Rect, ZoneId};

use crate::config::WorldConfig;

/// One city: a square of zones.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CityInfo {
    /// Id of the city.
    pub id: CityId,
    /// Name of the city.
    pub name: String,
    /// Tiles that the city covers.
    pub bounds: Rect,
    /// Number of zones per side.
    pub districts: u32,
    /// Zones of the city, row by row from the north-west corner.
    pub zones: Vec<ZoneId>,
}

/// One zone: a square district of a city.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZoneInfo {
    /// Id of the zone.
    pub id: ZoneId,
    /// The city that the zone belongs to.
    pub city: CityId,
    /// Name of the zone: the city name with the column and the row.
    pub name: String,
    /// Tiles that the zone covers.
    pub bounds: Rect,
    /// Column of the zone in its city, from 0.
    pub col: u32,
    /// Row of the zone in its city, from 0.
    pub row: u32,
}

/// The cities and zones of a world. It never changes after the world is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    cities: Vec<CityInfo>,
    zones: Vec<ZoneInfo>,
    zone_size: u32,
    days_per_year: u32,
}

impl Layout {
    /// Builds the layout of a valid configuration. Zones are numbered city by
    /// city, row by row.
    pub(crate) fn new(config: &WorldConfig) -> Layout {
        let size = config.zone_size;
        let mut cities = Vec::with_capacity(config.cities.len());
        let mut zones = Vec::new();
        for (index, city) in config.cities.iter().enumerate() {
            let city_id = CityId(index as u16);
            let bounds = config.city_bounds(city).unwrap_or_default();
            let mut ids = Vec::with_capacity((city.districts * city.districts) as usize);
            for row in 0..city.districts {
                for col in 0..city.districts {
                    let id = ZoneId(zones.len() as u32);
                    ids.push(id);
                    zones.push(ZoneInfo {
                        id,
                        city: city_id,
                        name: format!("{} {}-{}", city.name, col, row),
                        bounds: Rect::sized(
                            city.origin.x + (col * size) as i32,
                            city.origin.y + (row * size) as i32,
                            size,
                            size,
                        ),
                        col,
                        row,
                    });
                }
            }
            cities.push(CityInfo {
                id: city_id,
                name: city.name.clone(),
                bounds,
                districts: city.districts,
                zones: ids,
            });
        }
        Layout { cities, zones, zone_size: size, days_per_year: config.days_per_year }
    }

    /// Returns every city, ordered by id.
    #[must_use]
    pub fn cities(&self) -> &[CityInfo] {
        &self.cities
    }

    /// Returns every zone, ordered by id.
    #[must_use]
    pub fn zones(&self) -> &[ZoneInfo] {
        &self.zones
    }

    /// Returns one city.
    #[must_use]
    pub fn city(&self, id: CityId) -> Option<&CityInfo> {
        self.cities.get(usize::from(id.0))
    }

    /// Returns one zone.
    #[must_use]
    pub fn zone(&self, id: ZoneId) -> Option<&ZoneInfo> {
        self.zones.get(id.0 as usize)
    }

    /// Returns the width and height of a zone in tiles.
    #[must_use]
    pub fn zone_size(&self) -> u32 {
        self.zone_size
    }

    /// Returns the number of days in a game year.
    #[must_use]
    pub fn days_per_year(&self) -> u32 {
        self.days_per_year
    }

    /// Returns the calendar time of a frame.
    #[must_use]
    pub fn clock_at(&self, frame: u64) -> Clock {
        Clock::at(frame, self.days_per_year)
    }

    /// Returns the city that covers `pos`.
    #[must_use]
    pub fn city_at(&self, pos: Pos) -> Option<CityId> {
        self.cities.iter().find(|c| c.bounds.contains(pos)).map(|c| c.id)
    }

    /// Returns the zone that covers `pos`, or `None` when `pos` is outside
    /// every zone.
    #[must_use]
    pub fn zone_at(&self, pos: Pos) -> Option<ZoneId> {
        let city = self.cities.iter().find(|c| c.bounds.contains(pos))?;
        let col = pos.x.abs_diff(city.bounds.x0) / self.zone_size;
        let row = pos.y.abs_diff(city.bounds.y0) / self.zone_size;
        city.zones.get((row * city.districts + col) as usize).copied()
    }

    /// Returns the zones that share an edge with `zone`, in the order north,
    /// east, south, west. A side without a zone is left out.
    #[must_use]
    pub fn neighbours(&self, zone: ZoneId) -> Vec<ZoneId> {
        let Some(info) = self.zone(zone) else {
            return Vec::new();
        };
        let b = info.bounds;
        let c = b.centre();
        [Pos::new(c.x, b.y0 - 1), Pos::new(b.x1, c.y), Pos::new(c.x, b.y1), Pos::new(b.x0 - 1, c.y)]
            .into_iter()
            .filter_map(|p| self.zone_at(p))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CityConfig;

    fn two_cities() -> Layout {
        let config = WorldConfig {
            cities: vec![
                CityConfig::new("Ashford", Pos::new(0, 0), 2),
                CityConfig::new("Brine", Pos::new(1_000, -64), 3),
            ],
            ..WorldConfig::default()
        };
        config.validate().unwrap();
        Layout::new(&config)
    }

    #[test]
    fn zones_are_numbered_city_by_city_row_by_row() {
        let layout = two_cities();
        assert_eq!(layout.cities().len(), 2);
        assert_eq!(layout.zones().len(), 4 + 9);
        assert_eq!(
            layout.city(CityId(0)).unwrap().zones,
            [ZoneId(0), ZoneId(1), ZoneId(2), ZoneId(3)]
        );
        let z3 = layout.zone(ZoneId(3)).unwrap();
        assert_eq!((z3.col, z3.row, z3.city), (1, 1, CityId(0)));
        assert_eq!(z3.bounds, Rect::new(64, 64, 128, 128));
        assert_eq!(z3.name, "Ashford 1-1");
        let z4 = layout.zone(ZoneId(4)).unwrap();
        assert_eq!(z4.bounds, Rect::new(1_000, -64, 1_064, 0));
        assert_eq!(layout.city(CityId(1)).unwrap().bounds, Rect::new(1_000, -64, 1_192, 128));
        assert_eq!(layout.zone(ZoneId(13)), None);
        assert_eq!(layout.city(CityId(2)), None);
    }

    #[test]
    fn zone_at_finds_the_zone_or_nothing() {
        let layout = two_cities();
        assert_eq!(layout.zone_at(Pos::new(0, 0)), Some(ZoneId(0)));
        assert_eq!(layout.zone_at(Pos::new(63, 63)), Some(ZoneId(0)));
        assert_eq!(layout.zone_at(Pos::new(64, 63)), Some(ZoneId(1)));
        assert_eq!(layout.zone_at(Pos::new(63, 64)), Some(ZoneId(2)));
        assert_eq!(layout.zone_at(Pos::new(127, 127)), Some(ZoneId(3)));
        assert_eq!(layout.zone_at(Pos::new(128, 127)), None);
        assert_eq!(layout.zone_at(Pos::new(-1, 0)), None);
        assert_eq!(layout.zone_at(Pos::new(1_000, -64)), Some(ZoneId(4)));
        assert_eq!(layout.zone_at(Pos::new(1_191, 127)), Some(ZoneId(12)));
        assert_eq!(layout.city_at(Pos::new(1_100, 0)), Some(CityId(1)));
        assert_eq!(layout.city_at(Pos::new(500, 0)), None);
        // Every zone finds itself from each of its corners.
        for zone in layout.zones() {
            let b = zone.bounds;
            for pos in [Pos::new(b.x0, b.y0), Pos::new(b.x1 - 1, b.y1 - 1), b.centre()] {
                assert_eq!(layout.zone_at(pos), Some(zone.id));
            }
        }
    }

    #[test]
    fn neighbours_share_an_edge() {
        let layout = two_cities();
        assert_eq!(layout.neighbours(ZoneId(0)), [ZoneId(1), ZoneId(2)]);
        assert_eq!(layout.neighbours(ZoneId(3)), [ZoneId(1), ZoneId(2)]);
        // The centre zone of the 3 by 3 city has four neighbours.
        assert_eq!(layout.neighbours(ZoneId(8)), [ZoneId(5), ZoneId(9), ZoneId(11), ZoneId(7)]);
        assert_eq!(layout.neighbours(ZoneId(99)), []);
    }

    #[test]
    fn clock_uses_the_days_per_year_of_the_config() {
        let layout = two_cities();
        assert_eq!(layout.days_per_year(), 28);
        assert_eq!(layout.zone_size(), 64);
        assert_eq!(layout.clock_at(1_440 * 28).year, 2);
    }
}
