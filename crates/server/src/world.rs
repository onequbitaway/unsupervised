//! The shape of a new world and the function that builds it.

use unsup_kernel::{CityConfig, Pos, World, WorldConfig, WorldError};

use crate::plugins::plugins;

/// Shape of the world that a new server creates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorldShape {
    /// Seed of all randomness.
    pub seed: u64,
    /// Number of cities.
    pub cities: u32,
    /// Districts on one side of a city. A city has `districts * districts` zones.
    pub districts: u32,
}

impl Default for WorldShape {
    fn default() -> WorldShape {
        crate::ServerConfig::default().world
    }
}

impl WorldShape {
    /// Returns the kernel configuration for this shape. Cities stand in a
    /// row with a gap between them.
    #[must_use]
    pub fn to_config(&self) -> WorldConfig {
        let mut config = WorldConfig::with_seed(self.seed);
        let side = i64::from(self.districts) * i64::from(config.zone_size);
        let pitch = side + i64::from(config.zone_size);
        config.cities = (0..self.cities)
            .map(|i| {
                let x = i32::try_from(i64::from(i) * pitch).unwrap_or(i32::MAX);
                CityConfig::new(city_name(i), Pos::new(x, 0), self.districts)
            })
            .collect();
        config
    }
}

/// Returns the name of a city by number. The names are fixed so that the
/// same seed always gives the same world.
fn city_name(index: u32) -> String {
    const NAMES: [&str; 12] = [
        "Latent City",
        "Tokentown",
        "Gradient Falls",
        "Port Epoch",
        "New Tensor",
        "Softmax Springs",
        "Fort Inference",
        "Attention Heights",
        "Overfit Hollow",
        "Kernel Bay",
        "Mount Entropy",
        "Bias Harbor",
    ];
    let base = NAMES[index as usize % NAMES.len()];
    let round = index as usize / NAMES.len();
    if round == 0 { base.to_string() } else { format!("{base} {}", round + 1) }
}

/// Builds a new world with all game modules.
///
/// # Panics
///
/// Panics when the configuration is not valid. [`try_build_world`]
/// returns the error instead.
#[must_use]
pub fn build_world(config: WorldConfig) -> World {
    let plugins = plugins();
    let refs: Vec<&dyn unsup_kernel::Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    World::new(&refs, config)
}

/// Builds a new world with all game modules.
///
/// # Errors
///
/// Fails when the configuration is not valid, for example with no city.
pub fn try_build_world(config: WorldConfig) -> Result<World, WorldError> {
    let plugins = plugins();
    let refs: Vec<&dyn unsup_kernel::Plugin> = plugins.iter().map(AsRef::as_ref).collect();
    World::try_new(&refs, config)
}
