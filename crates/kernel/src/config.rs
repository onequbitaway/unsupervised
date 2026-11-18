//! World configuration.

use std::fmt;

use serde::{Deserialize, Serialize};
use unsup_protocol::{Pos, RealmId, Rect};

/// Greatest number of zones in one world. Realm ordinals take 13 bits of the
/// ids that `Realm::alloc_id` returns.
pub const MAX_ZONES: usize = 8_190;

/// Game minutes (ticks) in one day.
pub const TICKS_PER_DAY: u64 = 1_440;

/// The configuration of a world. It is fixed for the life of the world and is
/// part of every snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorldConfig {
    /// Seed of all randomness.
    pub seed: u64,
    /// The cities. The index in this list is the `CityId`.
    pub cities: Vec<CityConfig>,
    /// Width and height of one zone in tiles.
    pub zone_size: u32,
    /// Days in one game year.
    pub days_per_year: u32,
    /// Extra data files (TOML text, the format of `data/balance.toml`). They
    /// are applied after the data of all plugins and override it.
    #[serde(default)]
    pub data: Vec<String>,
    /// The realms that this process hosts. All realms when absent.
    #[serde(default)]
    pub hosted: Option<Vec<RealmId>>,
    /// Every realm audits its ledger every this many ticks. 0 turns the
    /// self-audit off; `World::audit` still works.
    #[serde(default = "default_audit_every")]
    pub audit_every: u32,
}

fn default_audit_every() -> u32 {
    1
}

/// One city of a world: a square of `districts * districts` zones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CityConfig {
    /// Name of the city.
    pub name: String,
    /// The tile at the north-west corner of the city.
    pub origin: Pos,
    /// Number of zones per side.
    pub districts: u32,
}

impl CityConfig {
    /// Makes a city.
    #[must_use]
    pub fn new(name: impl Into<String>, origin: Pos, districts: u32) -> CityConfig {
        CityConfig { name: name.into(), origin, districts }
    }
}

impl Default for WorldConfig {
    /// One city of 2 by 2 zones, 64 tiles per zone, 28 days per year, seed 0.
    fn default() -> WorldConfig {
        WorldConfig {
            seed: 0,
            cities: vec![CityConfig::new("Ashford", Pos::new(0, 0), 2)],
            zone_size: 64,
            days_per_year: 28,
            data: Vec::new(),
            hosted: None,
            audit_every: 1,
        }
    }
}

impl WorldConfig {
    /// Makes the default configuration with a seed.
    #[must_use]
    pub fn with_seed(seed: u64) -> WorldConfig {
        WorldConfig { seed, ..WorldConfig::default() }
    }

    /// Returns the tiles that city `index` covers, or `None` when the numbers
    /// do not fit.
    pub(crate) fn city_bounds(&self, city: &CityConfig) -> Option<Rect> {
        let side = city.districts.checked_mul(self.zone_size)?;
        let side = i32::try_from(side).ok()?;
        Some(Rect::new(
            city.origin.x,
            city.origin.y,
            city.origin.x.checked_add(side)?,
            city.origin.y.checked_add(side)?,
        ))
    }

    /// Checks the configuration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let fail = |message: String| Err(ConfigError { message });
        if self.cities.is_empty() {
            return fail("the world needs at least one city".into());
        }
        if self.cities.len() > usize::from(u16::MAX) {
            return fail("too many cities".into());
        }
        if !(8..=1_024).contains(&self.zone_size) {
            return fail(format!("zone_size is {}, it must be from 8 to 1024", self.zone_size));
        }
        if self.days_per_year == 0 {
            return fail("days_per_year must be greater than zero".into());
        }
        let mut zones = 0usize;
        let mut bounds: Vec<(&str, Rect)> = Vec::new();
        for city in &self.cities {
            if city.name.trim().is_empty() {
                return fail("a city has an empty name".into());
            }
            if city.districts == 0 {
                return fail(format!("city `{}` has no districts", city.name));
            }
            let Some(rect) = self.city_bounds(city) else {
                return fail(format!("city `{}` does not fit in the coordinate range", city.name));
            };
            if let Some((other, _)) = bounds.iter().find(|(_, r)| r.intersects(&rect)) {
                return fail(format!("cities `{}` and `{}` overlap", other, city.name));
            }
            if bounds.iter().any(|(name, _)| *name == city.name) {
                return fail(format!("two cities are named `{}`", city.name));
            }
            bounds.push((&city.name, rect));
            zones = zones.saturating_add((city.districts as usize).saturating_pow(2));
        }
        if zones > MAX_ZONES {
            return fail(format!("the world has {zones} zones, the limit is {MAX_ZONES}"));
        }
        Ok(())
    }
}

/// The configuration of a world is not valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError {
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid world configuration: {}", self.message)
    }
}

impl std::error::Error for ConfigError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_valid() {
        assert_eq!(WorldConfig::default().validate(), Ok(()));
        assert_eq!(WorldConfig::with_seed(9).seed, 9);
    }

    #[test]
    fn invalid_configurations_are_refused() {
        let base = WorldConfig::default();
        let message = |config: WorldConfig| config.validate().unwrap_err().message;

        assert!(
            message(WorldConfig { cities: vec![], ..base.clone() }).contains("at least one city")
        );
        assert!(message(WorldConfig { zone_size: 4, ..base.clone() }).contains("zone_size"));
        assert!(
            message(WorldConfig { days_per_year: 0, ..base.clone() }).contains("days_per_year")
        );
        let city =
            |name: &str, x: i32, districts: u32| CityConfig::new(name, Pos::new(x, 0), districts);
        assert!(
            message(WorldConfig { cities: vec![city("A", 0, 0)], ..base.clone() })
                .contains("no districts")
        );
        assert!(
            message(WorldConfig { cities: vec![city(" ", 0, 1)], ..base.clone() })
                .contains("empty name")
        );
        assert!(
            message(WorldConfig {
                cities: vec![city("A", 0, 2), city("B", 127, 1)],
                ..base.clone()
            })
            .contains("overlap")
        );
        assert!(
            message(WorldConfig {
                cities: vec![city("A", 0, 1), city("A", 500, 1)],
                ..base.clone()
            })
            .contains("named `A`")
        );
        assert!(
            message(WorldConfig { cities: vec![city("A", i32::MAX - 10, 1)], ..base.clone() })
                .contains("does not fit")
        );
        assert!(
            message(WorldConfig { cities: vec![city("A", 0, 91)], zone_size: 8, ..base.clone() })
                .contains("limit")
        );
        // Cities that touch but do not overlap are fine.
        let touching = WorldConfig { cities: vec![city("A", 0, 2), city("B", 128, 1)], ..base };
        assert_eq!(touching.validate(), Ok(()));
    }
}
