//! The publication of the leaderboards, and the counters that events feed.

use unsup_kernel::serde_json::json;
use unsup_kernel::{Event, EventMeta, OwnerId, Realm, ResidentId, TICKS_PER_DAY, Value};

use super::score;
use super::state::{Board, Boards, Standing};
use crate::params::{self, Params};
use crate::season::Seasons;

/// The event that the board `law` counts: an arrest. The actor of the event
/// is the resident that made the arrest. The crime module emits it.
pub(crate) const ARREST_EVENT: &str = "crime.arrest_made";

/// Builds the standing of one board now.
pub(crate) fn standing(realm: &Realm, p: &Params, boards: &Boards, board: Board) -> Standing {
    let scores = score::scores(realm, boards, board);
    score::standing(
        realm,
        boards,
        &scores,
        params::count(realm, p.board_top),
        params::count(realm, p.board_groups),
    )
}

/// Returns the data of a publication: the standing with the name of the
/// board as `key` and the number of the season.
pub(crate) fn published(board: Board, season: u32, standing: &Standing) -> Value {
    let mut data = standing.to_value(board);
    data["key"] = json!(board.name());
    data["season"] = json!(season);
    data
}

/// Runs in every tick of the global realm, after the zone reports of the
/// frame: publishes every board each `missions.board_period` frames as the
/// event `missions.leaderboard_published`.
pub(crate) fn tick(realm: &Realm, p: &Params) {
    if !realm.frame().is_multiple_of(params::frames(realm, p.board_period)) {
        return;
    }
    let season = realm.res::<Seasons>().current(realm, p);
    let boards = realm.res::<Boards>();
    for board in Board::ALL {
        let standing = standing(realm, p, &boards, board);
        realm.emit(
            EventMeta::public(
                "missions.leaderboard_published",
                params::drama(realm, p.drama_routine),
            ),
            &published(board, season, &standing),
        );
    }
}

/// Reads `crime.arrest_made`: counts one arrest for the resident that made
/// it.
pub(crate) fn on_arrest(realm: &Realm, event: &Event) {
    let officer = event.actor.or_else(|| event.data["officer"].as_u64().map(ResidentId));
    if let Some(officer) = officer {
        let mut boards = realm.res_mut::<Boards>();
        let record = boards.records.entry(officer).or_default();
        if record.gone == 0 {
            record.arrests += 1;
        }
    }
}

/// Reads `kernel.despawned`: the resident left the world.
pub(crate) fn on_despawned(realm: &Realm, event: &Event) {
    if let Some(gone) = event.actor {
        realm.res_mut::<Boards>().mark_gone(gone, realm.frame());
    }
}

/// Gives a resident the points for a completed mission and returns them.
///
/// A mission of a sponsor gives `missions.points_sponsor`. A mission of a
/// resident gives `missions.points_resident`, with two limits: no points
/// when the poster has the owner of the resident, and at most
/// `missions.points_per_day` such points in one game day.
pub(crate) fn credit_points(
    realm: &Realm,
    p: &Params,
    resident: ResidentId,
    poster_owner: Option<OwnerId>,
) -> i64 {
    let owner = realm.directory().get(resident).map(|entry| entry.owner);
    let mut boards = realm.res_mut::<Boards>();
    let record = boards.records.entry(resident).or_default();
    let points = match poster_owner {
        None => realm.param(p.points_sponsor).max(0),
        // An owner cannot give points to its own residents.
        Some(poster) if Some(poster) == owner => 0,
        Some(_) => {
            let day = realm.frame() / TICKS_PER_DAY;
            if record.points_day != day {
                record.points_day = day;
                record.points_today = 0;
            }
            let room = (realm.param(p.points_per_day) - record.points_today).max(0);
            let points = realm.param(p.points_resident).clamp(0, room);
            record.points_today += points;
            points
        }
    };
    record.points = record.points.saturating_add(points);
    points
}
