//! The configuration of the sponsor missions: the `[sponsors]` table.

use std::time::Duration;

use serde::Deserialize;

/// How the server takes missions from human sponsors (`POST
/// /v1/missions`). The values come from the `[sponsors]` table of the
/// server configuration. `data/defaults.toml` has the defaults.
///
/// A sponsor mission creates new money. The missions module limits the
/// sum for all sponsors in one game day (`missions.sponsor_daily_limit`).
/// These values are the limits of the server for one owner.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SponsorConfig {
    /// False: nobody can publish a sponsor mission. The route answers
    /// `403` with the code `missions_disabled`. Only the administrator of
    /// the server changes the configuration, so this is the switch of the
    /// administrator.
    pub enabled: bool,
    /// Missions that one owner can publish in 24 hours. 0: no limit for
    /// one owner; the limit of the module for all sponsors stays.
    pub missions_per_day: u32,
    /// Greatest sum of new money for one mission, in cents: the reward
    /// times the takers. 0: no limit of the server.
    pub max_mission_cents: u64,
    /// Real milliseconds that the route waits for the answer of the
    /// simulation. The answer comes in the frame after the request.
    pub answer_timeout_ms: u64,
}

impl SponsorConfig {
    /// Returns the time that the route waits for the answer of the
    /// simulation.
    #[must_use]
    pub fn answer_timeout(&self) -> Duration {
        Duration::from_millis(self.answer_timeout_ms)
    }
}
