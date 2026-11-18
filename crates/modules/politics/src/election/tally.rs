//! The count: who wins the seats, with a seeded draw for a tie, and what
//! happens when an election is decided.

use unsup_kernel::serde_json::json;
use unsup_kernel::{EventMeta, Realm, ResidentId, Value};

use super::{facts, facts_with, fund};
use crate::calendar;
use crate::offices;
use crate::params::{self, Params};
use crate::publish;
use crate::state::Politics;
use crate::state::election::{Cause, Election};

/// The result of a count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Count {
    /// The winners: most votes first, then the lower id.
    pub(crate) winners: Vec<ResidentId>,
    /// True when a draw decided between candidates with the same votes.
    pub(crate) tie: bool,
    /// Votes between the last winner and the best loser, when there is a
    /// winner and a loser.
    pub(crate) margin: Option<u32>,
}

/// Decides who wins `seats` seats.
///
/// `standings` has every candidate with its votes: most votes first, then
/// the lower id. A candidate needs `least` votes to win. When more
/// candidates have the votes of the last seat than seats are left, `draw`
/// puts those candidates in a random order and the first ones win.
pub(crate) fn count(
    standings: &[(ResidentId, u32)],
    seats: usize,
    least: u32,
    draw: impl FnOnce(&mut [ResidentId]),
) -> Count {
    let able: Vec<(ResidentId, u32)> =
        standings.iter().copied().filter(|&(_, votes)| votes >= least).collect();
    let mut tie = false;
    let winners: Vec<ResidentId> = if able.len() <= seats {
        able.iter().map(|&(id, _)| id).collect()
    } else {
        // `able` has more entries than seats, so both indexes exist when
        // there is a seat at all.
        let last = seats.checked_sub(1).and_then(|at| able.get(at)).map(|&(_, votes)| votes);
        let next = able.get(seats).map(|&(_, votes)| votes);
        match last {
            None => Vec::new(),
            Some(cutoff) if next == Some(cutoff) => {
                tie = true;
                let mut winners: Vec<ResidentId> =
                    able.iter().filter(|&&(_, votes)| votes > cutoff).map(|&(id, _)| id).collect();
                let mut tied: Vec<ResidentId> =
                    able.iter().filter(|&&(_, votes)| votes == cutoff).map(|&(id, _)| id).collect();
                draw(&mut tied);
                tied.truncate(seats - winners.len());
                // The winners of the draw are listed in id order: the draw
                // decides who wins, not the order.
                tied.sort_unstable();
                winners.extend(tied);
                winners
            }
            Some(_) => able.iter().take(seats).map(|&(id, _)| id).collect(),
        }
    };
    let votes_of = |id: ResidentId| standings.iter().find(|&&(c, _)| c == id).map(|&(_, v)| v);
    let last_winner = winners.iter().filter_map(|&id| votes_of(id)).min();
    let best_loser =
        standings.iter().filter(|(id, _)| !winners.contains(id)).map(|&(_, votes)| votes).max();
    let margin = match (last_winner, best_loser) {
        (Some(won), Some(lost)) => Some(won.saturating_sub(lost)),
        _ => None,
    };
    Count { winners, tie, margin }
}

/// Returns the drama of the result of an election.
fn drama_of(realm: &Realm, p: &Params, result: &Count) -> u8 {
    let close = u32::try_from(realm.param(p.close_margin).max(0)).unwrap_or(u32::MAX);
    let param = if result.winners.is_empty() {
        p.drama_election_empty
    } else if result.tie || result.margin.is_some_and(|margin| margin <= close) {
        p.drama_election_close
    } else {
        p.drama_election_decided
    };
    params::drama(realm, param)
}

/// Tells each candidate how the election ended for it.
fn tell_candidates(realm: &Realm, election: &Election, winners: &[ResidentId]) {
    for (&candidate, record) in &election.candidates {
        let won = winners.contains(&candidate);
        let kind = if won { "politics.election_won" } else { "politics.election_lost" };
        realm.notify(candidate, kind, None, facts_with(election, json!({ "votes": record.votes })));
    }
}

/// Counts the votes of an election whose polls are closed. The winners
/// take office at once and the residents that held the office before lose
/// it. An office that nobody won is empty.
pub(crate) fn decide(realm: &Realm, p: &Params, state: &mut Politics, id: u64) {
    let Some(mut election) = state.elections.remove(&id) else {
        return;
    };
    // A candidate that left the world cannot take office.
    election.candidates.retain(|&candidate, _| realm.directory().is_alive(candidate));
    let org = election.gov.org();
    let role = offices::office_role(election.office);
    let previous = offices::holders(realm, org, role);
    let standings = election.standings();
    let seats = usize::try_from(election.seats).unwrap_or(usize::MAX);
    let least = u32::try_from(realm.param(p.win_votes_min).max(0)).unwrap_or(u32::MAX);
    let mut result = count(&standings, seats, least, |tied| realm.rng().shuffle(tied));
    // The registry has the last word on who holds the office.
    result.winners = offices::install(realm, org, role, &result.winners);
    result.winners.sort_by_key(|winner| standings.iter().position(|&(id, _)| id == *winner));

    let treasury: i64 = election.candidates.values().map(|c| fund::close(realm, c, org)).sum();
    tell_candidates(realm, &election, &result.winners);
    let tally: Vec<Value> = standings
        .iter()
        .map(|&(candidate, votes)| {
            json!({ "candidate": candidate, "votes": votes, "won": result.winners.contains(&candidate) })
        })
        .collect();
    let mut event = EventMeta::public("politics.election_decided", drama_of(realm, p, &result));
    if let Some(&winner) = result.winners.first() {
        event = event.actor(winner);
    }
    let mut data = facts(&election);
    data.insert("seats".into(), json!(election.seats));
    data.insert("winner".into(), json!(result.winners.first()));
    data.insert("winners".into(), json!(result.winners));
    data.insert("previous".into(), json!(previous));
    data.insert("tally".into(), json!(tally));
    data.insert("turnout".into(), json!(election.voters.len()));
    data.insert("tie".into(), json!(result.tie));
    data.insert("margin".into(), json!(result.margin));
    data.insert("funds_to_treasury".into(), json!(treasury));
    realm.emit(event, &Value::Object(data));

    publish::election_decided(realm, p, &election, &result.winners, result.tie);
    state.dirty.elections.remove(&id);
    state.touch_government(election.gov);
    // The term of a regular election ends here. The next one follows.
    if election.cause == Cause::Regular && !state.has_regular(election.gov, election.vote_day) {
        let next = election.vote_day + calendar::term_days(realm, p, election.gov);
        state.next_vote.insert(org, next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[u64]) -> Vec<ResidentId> {
        list.iter().copied().map(ResidentId).collect()
    }

    fn standings(list: &[(u64, u32)]) -> Vec<(ResidentId, u32)> {
        list.iter().map(|&(id, votes)| (ResidentId(id), votes)).collect()
    }

    /// A draw that must not happen.
    fn no_draw(_: &mut [ResidentId]) {
        panic!("this count has no tie");
    }

    #[test]
    fn the_candidate_with_the_most_votes_wins() {
        let result = count(&standings(&[(4, 9), (2, 5), (7, 1)]), 1, 1, no_draw);
        assert_eq!(result, Count { winners: ids(&[4]), tie: false, margin: Some(4) });
    }

    #[test]
    fn several_seats_go_to_the_top_of_the_list() {
        let result = count(&standings(&[(4, 9), (2, 5), (7, 3), (1, 2)]), 3, 1, no_draw);
        assert_eq!(result, Count { winners: ids(&[4, 2, 7]), tie: false, margin: Some(1) });
    }

    #[test]
    fn fewer_candidates_than_seats_all_win() {
        let result = count(&standings(&[(4, 2), (2, 1)]), 5, 1, no_draw);
        assert_eq!(result, Count { winners: ids(&[4, 2]), tie: false, margin: None });
    }

    #[test]
    fn a_candidate_without_the_least_votes_cannot_win() {
        let result = count(&standings(&[(4, 2), (2, 0)]), 5, 1, no_draw);
        assert_eq!(result, Count { winners: ids(&[4]), tie: false, margin: Some(2) });
        let nobody = count(&standings(&[(4, 0), (2, 0)]), 1, 1, no_draw);
        assert_eq!(nobody, Count { winners: Vec::new(), tie: false, margin: None });
        // With a least number of zero, a candidate wins without a vote.
        let alone = count(&standings(&[(4, 0)]), 1, 0, no_draw);
        assert_eq!(alone.winners, ids(&[4]));
    }

    #[test]
    fn a_tie_for_the_last_seat_goes_to_the_draw() {
        // Candidate 4 is sure. Candidates 2, 7 and 9 have the votes of the
        // second seat. The draw puts 9 first.
        let result = count(&standings(&[(4, 9), (2, 5), (7, 5), (9, 5), (1, 2)]), 2, 1, |tied| {
            assert_eq!(tied, ids(&[2, 7, 9]));
            tied.reverse();
        });
        assert_eq!(result, Count { winners: ids(&[4, 9]), tie: true, margin: Some(0) });
    }

    #[test]
    fn a_tie_inside_the_seats_needs_no_draw() {
        let result = count(&standings(&[(4, 5), (2, 5), (7, 3)]), 2, 1, no_draw);
        assert_eq!(result, Count { winners: ids(&[4, 2]), tie: false, margin: Some(2) });
    }

    #[test]
    fn no_seat_means_no_winner() {
        let result = count(&standings(&[(4, 5), (2, 5)]), 0, 1, no_draw);
        assert_eq!(result, Count { winners: Vec::new(), tie: false, margin: None });
    }
}
