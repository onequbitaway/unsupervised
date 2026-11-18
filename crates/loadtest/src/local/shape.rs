//! The size of a world for a number of residents.

use serde::{Deserialize, Serialize};
use unsup_kernel::WorldConfig;
use unsup_server::WorldShape;

use crate::defaults::defaults;

/// Cities and districts of a world.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Shape {
    /// Number of cities.
    pub cities: u32,
    /// Districts on one side of a city. A city has `districts * districts`
    /// zones.
    pub districts: u32,
}

impl Shape {
    /// Returns the number of zones.
    #[must_use]
    pub fn zones(&self) -> u64 {
        u64::from(self.cities) * u64::from(self.districts) * u64::from(self.districts)
    }

    /// Chooses the size of a world for `residents`, with about
    /// `world.residents_per_zone` residents for each zone. A number that
    /// the caller gives is kept; the other is chosen to fit.
    ///
    /// Without a given number, cities are as large as the data file allows
    /// while the number of zones stays near the wanted number: residents
    /// then cross district borders, as they do in the game.
    #[must_use]
    pub fn choose(residents: u32, cities: Option<u32>, districts: Option<u32>) -> Shape {
        let d = &defaults().world;
        let wanted = u64::from(residents.div_ceil(d.residents_per_zone.max(1)).max(1));
        let zones_of = |cities: u64, districts: u64| cities * districts * districts;
        // The number of cities that brings a city size nearest to the
        // wanted number of zones.
        let cities_for = |districts: u64| {
            let per_city = districts * districts;
            ((wanted + per_city / 2) / per_city).max(1)
        };
        let small = |n: u64| u32::try_from(n).unwrap_or(u32::MAX);
        match (cities, districts) {
            (Some(cities), Some(districts)) => {
                Shape { cities: cities.max(1), districts: districts.max(1) }
            }
            (None, Some(districts)) => {
                let districts = districts.max(1);
                Shape { cities: small(cities_for(u64::from(districts))), districts }
            }
            (Some(cities), None) => {
                let cities = cities.max(1);
                let best = (1..=u64::from(d.max_districts.max(1)))
                    .min_by_key(|&districts| {
                        zones_of(u64::from(cities), districts).abs_diff(wanted)
                    })
                    .unwrap_or(1);
                Shape { cities, districts: small(best) }
            }
            (None, None) => {
                let tolerance = u64::from(d.shape_tolerance_percent);
                let districts = (1..=u64::from(d.max_districts.max(1)))
                    .rev()
                    .find(|&districts| {
                        let zones = zones_of(cities_for(districts), districts);
                        zones.abs_diff(wanted) * 100 <= tolerance * wanted
                    })
                    .unwrap_or(1);
                Shape { cities: small(cities_for(districts)), districts: small(districts) }
            }
        }
    }

    /// Returns the size in words, for example `1 city of 2 x 2 districts`
    /// or `8 cities of 8 x 8 districts`.
    #[must_use]
    pub fn describe(&self) -> String {
        let cities = if self.cities == 1 { "city" } else { "cities" };
        format!("{} {cities} of {} x {} districts", self.cities, self.districts, self.districts)
    }

    /// Writes a number of zones, for example `1 zone` or `512 zones`.
    #[must_use]
    pub fn zones_text(zones: u64) -> String {
        format!("{zones} {}", if zones == 1 { "zone" } else { "zones" })
    }

    /// Returns the kernel configuration of a world of this size, as the
    /// server makes it.
    #[must_use]
    pub fn to_config(&self, seed: u64) -> WorldConfig {
        WorldShape { seed, cities: self.cities, districts: self.districts }.to_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_size_gives_about_two_thousand_residents_for_each_zone() {
        // The data file asks for 2,000 residents for each zone, cities of at
        // most 8 by 8 districts, and a tolerance of 20 %.
        assert_eq!(Shape::choose(1, None, None), Shape { cities: 1, districts: 1 });
        assert_eq!(Shape::choose(2_000, None, None), Shape { cities: 1, districts: 1 });
        // 5 zones wanted: one city of 2 by 2 is within 20 %.
        assert_eq!(Shape::choose(10_000, None, None), Shape { cities: 1, districts: 2 });
        // 50 zones wanted: 7 by 7 = 49.
        assert_eq!(Shape::choose(100_000, None, None), Shape { cities: 1, districts: 7 });
        // 500 zones wanted: 8 cities of 8 by 8 = 512.
        assert_eq!(Shape::choose(1_000_000, None, None), Shape { cities: 8, districts: 8 });
        assert_eq!(Shape { cities: 8, districts: 8 }.zones(), 512);
    }

    #[test]
    fn a_given_number_is_kept() {
        assert_eq!(Shape::choose(100_000, Some(3), Some(2)), Shape { cities: 3, districts: 2 });
        // 50 zones wanted, cities of 2 by 2: 12 or 13 cities; 50 / 4 rounds to 13.
        assert_eq!(Shape::choose(100_000, None, Some(2)), Shape { cities: 13, districts: 2 });
        // 50 zones wanted in 2 cities: 5 by 5 gives exactly 50.
        assert_eq!(Shape::choose(100_000, Some(2), None), Shape { cities: 2, districts: 5 });
        assert_eq!(Shape::choose(100_000, Some(0), Some(0)), Shape { cities: 1, districts: 1 });
    }

    #[test]
    fn the_size_is_written_in_words() {
        assert_eq!(Shape { cities: 1, districts: 2 }.describe(), "1 city of 2 x 2 districts");
        assert_eq!(Shape { cities: 8, districts: 8 }.describe(), "8 cities of 8 x 8 districts");
        assert_eq!(Shape::zones_text(1), "1 zone");
        assert_eq!(Shape::zones_text(512), "512 zones");
    }

    #[test]
    fn the_configuration_is_the_one_of_the_server() {
        let config = Shape { cities: 2, districts: 3 }.to_config(9);
        assert_eq!(config.seed, 9);
        assert_eq!(config.cities.len(), 2);
        assert!(config.cities.iter().all(|city| city.districts == 3));
        assert_eq!(config.validate(), Ok(()));
    }
}
