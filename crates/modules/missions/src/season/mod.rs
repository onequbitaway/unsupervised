//! Seasons: a number, a length, final standings, a hall of fame and a soft
//! reset.
//!
//! At the end of a season the global realm publishes the final standing of
//! every board (`missions.season_ended`), stores it in the hall of fame,
//! clears the counters of the boards, orders every zone to apply the soft
//! reset (`reset.rs`) and starts the next season.

mod reset;

use serde::{Deserialize, Serialize};
use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{Builder, Dest, EventMeta, Realm, Resource, Scope, TICKS_PER_DAY, Value};

use self::reset::ResetOrder;
pub use self::reset::SeasonMark;
use crate::board::{self, Board, Boards, Standing};
use crate::params::{self, Params};

/// The final standings of a season that ended.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SeasonRecord {
    /// Number of the season.
    pub season: u32,
    /// The frame in which the season started.
    pub started: u64,
    /// The frame in which the season ended.
    pub ended: u64,
    /// The final standing of each board.
    pub boards: Vec<(Board, Standing)>,
}

impl SeasonRecord {
    /// Returns the final standing of a board.
    #[must_use]
    pub fn standing(&self, board: Board) -> Option<&Standing> {
        self.boards.iter().find(|(b, _)| *b == board).map(|(_, standing)| standing)
    }

    /// Returns the record as JSON: the boards by name.
    fn to_value(&self) -> Value {
        let boards: Map<String, Value> = self
            .boards
            .iter()
            .map(|(board, standing)| (board.name().to_string(), standing.to_value(*board)))
            .collect();
        json!({
            "season": self.season,
            "started": self.started,
            "ended": self.ended,
            "boards": boards,
        })
    }
}

/// The seasons. A resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Seasons {
    /// Number of the season that runs. 0 before the first tick.
    pub(crate) number: u32,
    /// The frame in which the season started.
    pub(crate) started: u64,
    /// The hall of fame: the seasons that ended, oldest first.
    pub(crate) hall: Vec<SeasonRecord>,
}

impl Resource for Seasons {
    const NAME: &'static str = "missions.seasons";
}

impl Seasons {
    /// Returns the number of the season that runs.
    #[must_use]
    pub fn number(&self) -> u32 {
        self.number
    }

    /// Returns the frame in which the season that runs started.
    #[must_use]
    pub fn started(&self) -> u64 {
        self.started
    }

    /// Returns the hall of fame: the seasons that ended, oldest first.
    #[must_use]
    pub fn hall_of_fame(&self) -> &[SeasonRecord] {
        &self.hall
    }

    /// Returns the number of the season that runs, also before the first
    /// tick of the world, when the first season has no number yet.
    pub(crate) fn current(&self, realm: &Realm, p: &Params) -> u32 {
        if self.number == 0 { first_number(realm, p) } else { self.number }
    }

    /// Returns the frame in which the season that runs ends.
    pub(crate) fn ends(&self, realm: &Realm, p: &Params) -> u64 {
        self.started.saturating_add(params::frames(realm, p.season_days) * TICKS_PER_DAY)
    }
}

/// Returns the number of the first season of a new world.
fn first_number(realm: &Realm, p: &Params) -> u32 {
    u32::try_from(realm.param(p.season_first).max(1)).unwrap_or(1)
}

/// Tells the world that a season started.
fn announce(realm: &Realm, p: &Params, seasons: &Seasons) {
    realm.emit(
        EventMeta::public("missions.season_started", params::drama(realm, p.drama_season_started)),
        &json!({
            "season": seasons.number,
            "started": seasons.started,
            "ends": seasons.ends(realm, p),
        }),
    );
}

/// Ends the season that runs and starts the next one.
fn end(realm: &Realm, p: &Params, seasons: &mut Seasons) {
    let now = realm.frame();
    let mut boards = realm.res_mut::<Boards>();
    let record = SeasonRecord {
        season: seasons.number,
        started: seasons.started,
        ended: now,
        boards: Board::ALL.map(|b| (b, board::standing(realm, p, &boards, b))).into(),
    };
    let order = ResetOrder {
        season: seasons.number,
        standing_keep_bp: realm.param(p.reset_standing_keep_bp),
        notoriety_keep_bp: realm.param(p.reset_notoriety_keep_bp),
        wealth_floor: realm.param(p.reset_wealth_floor),
        wealth_cut_bp: realm.param(p.reset_wealth_cut_bp),
    };
    let mut data = record.to_value();
    data["reset"] = order.to_value();
    data["next"] = json!(seasons.number + 1);
    realm.emit(
        EventMeta::public("missions.season_ended", params::drama(realm, p.drama_season_ended)),
        &data,
    );
    // The same data as published state, one series for each season.
    data["key"] = json!(seasons.number);
    realm.emit(
        EventMeta::public("missions.hall_of_fame_published", params::drama(realm, p.drama_routine)),
        &data,
    );

    seasons.hall.push(record);
    let keep = params::count(realm, p.hall_of_fame_max).max(1);
    if seasons.hall.len() > keep {
        let extra = seasons.hall.len() - keep;
        seasons.hall.drain(..extra);
    }
    boards.start_season();
    realm.send(Dest::AllZones, &order);
    seasons.number += 1;
    seasons.started = now;
    announce(realm, p, seasons);
}

/// Runs in every tick of the global realm: starts the first season, and
/// ends a season whose length is over.
pub(crate) fn tick(realm: &Realm, p: &Params) {
    let now = realm.frame();
    let mut seasons = realm.res_mut::<Seasons>();
    if seasons.number == 0 {
        // The first tick of a new world is frame 1: the season started with
        // the world.
        seasons.number = first_number(realm, p);
        seasons.started = now.saturating_sub(1);
        announce(realm, p, &seasons);
    }
    if now >= seasons.ends(realm, p) {
        end(realm, p, &mut seasons);
    }
}

/// Registers the state of the seasons and the soft reset. The season tick
/// itself runs in the system of the mission engine; see `lib.rs`.
pub(crate) fn build(b: &mut Builder, p: Params) {
    if b.scope() == Scope::Global {
        b.resource::<Seasons>();
    }
    reset::build(b, p);
}
