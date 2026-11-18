//! The numbers of the showcase world, from `data/showcase.toml`.

use serde::Deserialize;

/// The numbers of the showcase world.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
    /// How residents walk.
    pub walk: Walk,
    /// What the driver asks residents to do.
    pub driver: Driver,
    /// Drama scores of the events.
    pub drama: Drama,
}

/// How residents walk.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Walk {
    /// Chance in 1,000 per tick that an idle resident starts a new walk.
    pub start_per_mille: u32,
    /// Chance in 100 that a new walk leads to a neighbour district.
    pub leave_zone_percent: u32,
    /// Longest path of one walk, in tiles.
    pub max_path: u32,
}

/// What the driver asks residents to do.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Driver {
    /// One resident in this many gets a request in each frame.
    pub one_in: u64,
    /// Weight of `work`.
    pub work: u64,
    /// Weight of `eat`.
    pub eat: u64,
    /// Weight of `rest`.
    pub rest: u64,
    /// Weight of `sleep`.
    pub sleep: u64,
    /// Weight of `train`.
    pub train: u64,
    /// Weight of `say`.
    pub say: u64,
    /// Weight of `rob`.
    pub rob: u64,
    /// Weight of `marry`.
    pub marry: u64,
    /// Weight of `open_business`.
    pub open_business: u64,
    /// Weight of `die`.
    pub die: u64,
    /// Shortest duration of a busy action, in ticks.
    pub min_ticks: u64,
    /// Longest duration of a busy action, in ticks.
    pub max_ticks: u64,
    /// Frames between two elections.
    pub election_every: u64,
}

/// Drama scores of the events.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drama {
    /// `society.said`.
    pub said: u8,
    /// `economy.shift_completed`.
    pub shift_completed: u8,
    /// `economy.business_opened`.
    pub business_opened: u8,
    /// `society.married`.
    pub married: u8,
    /// `crime.theft_committed`.
    pub theft_committed: u8,
    /// `life.resident_died`.
    pub resident_died: u8,
    /// `politics.election_decided`.
    pub election_decided: u8,
}

impl Numbers {
    /// Reads the numbers that are built into the example.
    ///
    /// # Errors
    ///
    /// Fails when `data/showcase.toml` does not parse.
    pub fn built_in() -> Result<Numbers, toml::de::Error> {
        toml::from_str(include_str!("../../data/showcase.toml"))
    }
}
