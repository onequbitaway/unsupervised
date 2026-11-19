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
    /// The stories that the driver plays.
    pub plots: Plots,
    /// Drama scores of the events of the stories.
    pub plot_drama: PlotDrama,
}

/// The stories that the driver plays: how often, and how fast.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plots {
    /// Frames between two crimes with an investigation.
    pub crime_every: u64,
    /// Frames from the crime to the warrant.
    pub investigation_after: u64,
    /// Frames from the crime to the arrest.
    pub arrest_after: u64,
    /// Frames from the crime to the verdict.
    pub trial_after: u64,
    /// Frames between two feuds.
    pub feud_every: u64,
    /// Frames between the events of one feud.
    pub feud_step: u64,
    /// Frames between two bankruptcies.
    pub bankruptcy_every: u64,
    /// Frames between the events of one bankruptcy.
    pub bankruptcy_step: u64,
    /// Frames between two residents that get a job or a home.
    pub career_every: u64,
    /// Frames before an election in which the candidates register.
    pub campaign_frames: u64,
    /// Candidates of one election.
    pub candidates: u64,
    /// Frames between two publications of the statistics.
    pub stats_every: u64,
    /// Frames between two publications of the boards.
    pub boards_every: u64,
    /// Rows of one board.
    pub board_rows: u64,
}

/// Drama scores of the events of the stories.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlotDrama {
    /// `crime.warrant_issued`.
    pub investigation_opened: u8,
    /// `crime.arrest_made`.
    pub arrest_made: u8,
    /// `crime.verdict_given`.
    pub trial_ended: u8,
    /// `crime.theft_committed` with a large loss: a heist.
    pub heist_committed: u8,
    /// `society.opinion_given`.
    pub opinion_given: u8,
    /// `society.contract_breached`.
    pub contract_breached: u8,
    /// `finance.loan_defaulted`.
    pub loan_defaulted: u8,
    /// `economy.property_foreclosed`.
    pub property_foreclosed: u8,
    /// `economy.tenant_evicted`.
    pub tenant_evicted: u8,
    /// `economy.worker_hired`.
    pub worker_hired: u8,
    /// `economy.lease_started`.
    pub lease_started: u8,
    /// `politics.candidate_registered`.
    pub candidate_registered: u8,
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
