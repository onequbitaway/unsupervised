//! The calendar system: it opens elections, moves them through their
//! phases and closes the polls.

use std::collections::BTreeMap;

use unsup_kernel::serde_json::json;
use unsup_kernel::{EventMeta, Realm};

use super::{facts_with, tally};
use crate::calendar::{self, Windows};
use crate::model::{Gov, Office};
use crate::offices;
use crate::params::{self, Params};
use crate::state::Politics;
use crate::state::election::{Cause, Election, Phase};

/// Defines the roles and makes the calendar when the state is new: at
/// world generation, and in the first frame of a world that was restored
/// from a snapshot without this module.
pub(crate) fn ensure_ready(realm: &Realm, p: &Params, state: &mut Politics) {
    if state.ready {
        return;
    }
    offices::define_roles(realm);
    let today = calendar::today(realm);
    for gov in offices::governments(realm) {
        state.next_vote.insert(gov.org(), calendar::first_vote_day(realm, p, gov, today));
        state.touch_government(gov);
        state.touch_bills(gov);
    }
    state.ready = true;
}

/// What a new election is for and when it happens.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opening {
    /// The government of the office.
    pub(crate) gov: Gov,
    /// The office.
    pub(crate) office: Office,
    /// Why the election is held.
    pub(crate) cause: Cause,
    /// The voting day, as a day of the world from 0.
    pub(crate) vote_day: u64,
    /// The windows of the election.
    pub(crate) windows: Windows,
}

/// Opens an election and tells the world. Returns its id.
pub(crate) fn open(realm: &Realm, p: &Params, state: &mut Politics, opening: Opening) -> u64 {
    let Opening { gov, office, cause, vote_day, windows } = opening;
    let id = realm.alloc_id();
    let frame = realm.frame();
    let mut election = Election {
        id,
        gov,
        office,
        cause,
        seats: offices::office_seats(realm, p, gov, office),
        vote_day,
        opens: windows.opens,
        campaign_from: windows.campaign_from,
        polls_open: windows.polls_open,
        polls_close: windows.polls_close,
        phase: Phase::Registration,
        candidates: BTreeMap::new(),
        voters: BTreeMap::new(),
        leader: None,
    };
    // A calendar that changed can open an election after its registration
    // window. The phase is then the one of the frame.
    election.phase = election.phase_at(frame).unwrap_or(Phase::Voting);
    realm.emit(
        EventMeta::public(
            "politics.election_called",
            params::drama(realm, p.drama_election_called),
        ),
        &facts_with(
            &election,
            json!({
                "seats": election.seats,
                "vote_day": calendar::date(realm, vote_day),
                "registration_until": calendar::moment(realm, election.campaign_from),
                "polls_open": calendar::moment(realm, election.polls_open),
                "polls_close": calendar::moment(realm, election.polls_close),
            }),
        ),
    );
    state.elections.insert(id, election);
    state.touch_election(id);
    state.touch_government(gov);
    id
}

/// Moves one election to the phase of the frame, or counts its votes when
/// the polls are closed.
fn advance(realm: &Realm, p: &Params, state: &mut Politics, id: u64, frame: u64) {
    let Some(election) = state.elections.get_mut(&id) else {
        return;
    };
    let Some(phase) = election.phase_at(frame) else {
        tally::decide(realm, p, state, id);
        return;
    };
    if phase == election.phase {
        return;
    }
    election.phase = phase;
    if phase == Phase::Voting {
        realm.emit(
            EventMeta::public("politics.polls_opened", params::drama(realm, p.drama_polls_opened)),
            &facts_with(
                election,
                json!({
                    "candidates": election.candidates.len(),
                    "seats": election.seats,
                    "polls_close": calendar::moment(realm, election.polls_close),
                }),
            ),
        );
    }
    state.touch_election(id);
}

/// Runs the calendar for one frame.
pub(crate) fn run(realm: &Realm, p: &Params) {
    let mut state = realm.res_mut::<Politics>();
    ensure_ready(realm, p, &mut state);
    let frame = realm.frame();
    for gov in offices::governments(realm) {
        let Some(&vote_day) = state.next_vote.get(&gov.org()) else {
            continue;
        };
        if state.has_regular(gov, vote_day) {
            continue;
        }
        let windows = calendar::windows(realm, p, gov, vote_day);
        if frame >= windows.opens {
            for office in Office::of(gov) {
                let cause = Cause::Regular;
                open(realm, p, &mut state, Opening { gov, office, cause, vote_day, windows });
            }
        }
    }
    let ids: Vec<u64> = state.elections.keys().copied().collect();
    for id in ids {
        advance(realm, p, &mut state, id, frame);
    }
}
