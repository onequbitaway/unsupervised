//! How a board is computed: the score of each resident, the top list and
//! the sums by owner, by organization and by model.

use std::cmp::Reverse;
use std::collections::BTreeMap;

use unsup_kernel::{Holder, Realm, ResidentId, TICKS_PER_DAY};

use super::state::{Board, Boards, Entry, Group, Record, Standing};

/// Returns the score of one resident on a board, or nothing when the board
/// does not rank the resident.
///
/// A resident is ranked after the first report of its zone. A resident that
/// left the world stays on the boards for things done (`law`, `missions`,
/// `longevity`) until the season ends.
fn score(realm: &Realm, boards: &Boards, board: Board, id: ResidentId, r: &Record) -> Option<i64> {
    if r.reported == 0 || (r.gone != 0 && board.living_only()) {
        return None;
    }
    Some(match board {
        Board::Wealth => {
            // A bank deposit is the money of `Holder::Account(resident id)`
            // in the ledger of the global realm.
            let deposit = realm.ledger().money(Holder::Account(id.0)).cents();
            let sites = boards.site_value.get(&id).copied().unwrap_or(0);
            r.cash.saturating_add(deposit).saturating_add(sites)
        }
        Board::Influence => r.standing,
        Board::Notoriety => r.notoriety,
        Board::Law => r.arrests,
        Board::Missions => r.points,
        Board::Longevity => {
            let end = if r.gone != 0 { r.gone } else { realm.frame() };
            i64::try_from(end.saturating_sub(r.born) / TICKS_PER_DAY).unwrap_or(i64::MAX)
        }
    })
}

/// Returns the score of every resident that a board ranks, ordered by
/// resident.
pub(crate) fn scores(realm: &Realm, boards: &Boards, board: Board) -> Vec<(ResidentId, i64)> {
    boards
        .records
        .iter()
        .filter_map(|(&id, record)| score(realm, boards, board, id, record).map(|s| (id, s)))
        .collect()
}

/// Returns the place of a resident among the scores, from 1, and its score.
/// A higher score is better; among equals the lower id is better.
pub(crate) fn rank_of(scores: &[(ResidentId, i64)], resident: ResidentId) -> Option<(u64, i64)> {
    let &(_, own) = scores.iter().find(|(id, _)| *id == resident)?;
    let better =
        scores.iter().filter(|&&(id, s)| s > own || (s == own && id < resident)).count() as u64;
    Some((better + 1, own))
}

/// Keeps the best `limit` entries of a list, best first. `order` gives the
/// sort key of an entry; a lower key is better.
fn best<T, K: Ord>(mut list: Vec<T>, limit: usize, order: impl Fn(&T) -> K) -> Vec<T> {
    if list.len() > limit && limit > 0 {
        list.select_nth_unstable_by_key(limit - 1, &order);
    }
    list.truncate(limit);
    list.sort_by_key(order);
    list
}

/// Sums the scores by a key of the resident and returns the best groups.
/// With a limit of 0 it does no work.
fn groups<K: Ord + Clone>(
    boards: &Boards,
    scores: &[(ResidentId, i64)],
    limit: usize,
    keys: impl Fn(&Record) -> Vec<K>,
) -> Vec<Group<K>> {
    if limit == 0 {
        return Vec::new();
    }
    let mut sums: BTreeMap<K, (i64, u64)> = BTreeMap::new();
    for &(id, score) in scores {
        let Some(record) = boards.records.get(&id) else {
            continue;
        };
        for key in keys(record) {
            let sum = sums.entry(key).or_default();
            sum.0 = sum.0.saturating_add(score);
            sum.1 += 1;
        }
    }
    let list: Vec<(K, i64, u64)> = sums.into_iter().map(|(k, (sum, n))| (k, sum, n)).collect();
    best(list, limit, |(key, sum, _)| (Reverse(*sum), key.clone()))
        .into_iter()
        .enumerate()
        .map(|(i, (key, sum, n))| Group {
            rank: i as u32 + 1,
            key,
            score: sum,
            residents: n,
            mean: sum.div_euclid(i64::try_from(n).unwrap_or(i64::MAX).max(1)),
        })
        .collect()
}

/// Builds the standing of a board from the scores: the best `top`
/// residents and the best `group_top` owners, organizations and models.
pub(crate) fn standing(
    realm: &Realm,
    boards: &Boards,
    scores: &[(ResidentId, i64)],
    top: usize,
    group_top: usize,
) -> Standing {
    let directory = realm.directory();
    let listed = if top == 0 { Vec::new() } else { scores.to_vec() };
    let entries = best(listed, top, |&(id, score)| (Reverse(score), id))
        .into_iter()
        .enumerate()
        .map(|(i, (id, score))| {
            let record = boards.records.get(&id);
            Entry {
                rank: i as u32 + 1,
                resident: id,
                name: directory.get(id).map(|entry| entry.name.clone()).unwrap_or_default(),
                owner: record.map(|r| r.owner).unwrap_or_default(),
                model: record.and_then(|r| r.model.clone()),
                score,
            }
        })
        .collect();
    Standing {
        residents: scores.len() as u64,
        top: entries,
        by_owner: groups(boards, scores, group_top, |r| vec![r.owner]),
        by_org: groups(boards, scores, group_top, |r| r.orgs.clone()),
        by_model: groups(boards, scores, group_top, |r| vec![r.model.clone()]),
    }
}
