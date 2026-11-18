//! The record of one election.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::ResidentId;

use crate::model::{Gov, Office};

/// Where an election is in its calendar.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum Phase {
    /// Residents register as candidates.
    Registration,
    /// The candidates campaign.
    Campaign,
    /// The polls are open.
    Voting,
}

impl Phase {
    /// Returns the name of the phase in results, events and published state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Phase::Registration => "registration",
            Phase::Campaign => "campaign",
            Phase::Voting => "voting",
        }
    }
}

/// Why an election is held.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum Cause {
    /// The term ended.
    Regular,
    /// A recall petition had enough signatures.
    Recall,
}

impl Cause {
    /// Returns the name of the cause in events and published state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Cause::Regular => "regular",
            Cause::Recall => "recall",
        }
    }
}

/// One candidate of an election.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Candidate {
    /// The platform text, exactly as the candidate wrote it. It is text of
    /// a resident: it leaves the module only as `Untrusted`.
    pub(crate) platform: String,
    /// Number of the escrow that holds the campaign fund.
    pub(crate) fund: u64,
    /// Cents that donors gave.
    pub(crate) raised: i64,
    /// Cents that the candidate spent on rallies.
    pub(crate) spent: i64,
    /// Votes so far.
    pub(crate) votes: u32,
}

/// One election: one office of one government, from the start of the
/// registration to the count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Election {
    /// Id of the election.
    pub(crate) id: u64,
    /// The government of the office.
    pub(crate) gov: Gov,
    /// The office.
    pub(crate) office: Office,
    /// Why the election is held.
    pub(crate) cause: Cause,
    /// Seats to fill. It is fixed when the election opens.
    pub(crate) seats: u64,
    /// The voting day, as a day of the world from 0.
    pub(crate) vote_day: u64,
    /// Frame at which the registration opened.
    pub(crate) opens: u64,
    /// Frame at which the registration closes and the campaign starts.
    pub(crate) campaign_from: u64,
    /// Frame at which the polls open.
    pub(crate) polls_open: u64,
    /// Frame at which the polls close.
    pub(crate) polls_close: u64,
    /// The phase that the calendar system saw last.
    pub(crate) phase: Phase,
    /// The candidates.
    pub(crate) candidates: BTreeMap<ResidentId, Candidate>,
    /// Each voter with the candidate that it chose.
    pub(crate) voters: BTreeMap<ResidentId, ResidentId>,
    /// The candidate that led when the state was published last.
    pub(crate) leader: Option<ResidentId>,
}

impl Election {
    /// Returns the phase of the election at a frame, or `None` when the
    /// polls are closed.
    pub(crate) fn phase_at(&self, frame: u64) -> Option<Phase> {
        if frame >= self.polls_close {
            None
        } else if frame >= self.polls_open {
            Some(Phase::Voting)
        } else if frame >= self.campaign_from {
            Some(Phase::Campaign)
        } else {
            Some(Phase::Registration)
        }
    }

    /// Returns the frame at which the current phase ends.
    pub(crate) fn phase_ends(&self) -> u64 {
        match self.phase {
            Phase::Registration => self.campaign_from,
            Phase::Campaign => self.polls_open,
            Phase::Voting => self.polls_close,
        }
    }

    /// Returns the candidates with their votes: most votes first, then the
    /// lower id.
    pub(crate) fn standings(&self) -> Vec<(ResidentId, u32)> {
        let mut list: Vec<(ResidentId, u32)> =
            self.candidates.iter().map(|(&id, candidate)| (id, candidate.votes)).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        list
    }

    /// Returns the candidate that has more votes than every other
    /// candidate, if one has.
    pub(crate) fn leading(&self) -> Option<ResidentId> {
        let standings = self.standings();
        let &(first, votes) = standings.first()?;
        let shared = standings.get(1).is_some_and(|&(_, other)| other == votes);
        (votes > 0 && !shared).then_some(first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn election(votes: &[(u64, u32)]) -> Election {
        Election {
            id: 1,
            gov: Gov::Nation,
            office: Office::President,
            cause: Cause::Regular,
            seats: 1,
            vote_day: 4,
            opens: 100,
            campaign_from: 200,
            polls_open: 300,
            polls_close: 400,
            phase: Phase::Registration,
            candidates: votes
                .iter()
                .map(|&(id, votes)| {
                    let candidate =
                        Candidate { platform: String::new(), fund: id, raised: 0, spent: 0, votes };
                    (ResidentId(id), candidate)
                })
                .collect(),
            voters: BTreeMap::new(),
            leader: None,
        }
    }

    #[test]
    fn the_phase_follows_the_windows() {
        let e = election(&[]);
        assert_eq!(e.phase_at(0), Some(Phase::Registration));
        assert_eq!(e.phase_at(199), Some(Phase::Registration));
        assert_eq!(e.phase_at(200), Some(Phase::Campaign));
        assert_eq!(e.phase_at(299), Some(Phase::Campaign));
        assert_eq!(e.phase_at(300), Some(Phase::Voting));
        assert_eq!(e.phase_at(399), Some(Phase::Voting));
        assert_eq!(e.phase_at(400), None);
        assert_eq!(e.phase_ends(), 200);
    }

    #[test]
    fn standings_order_by_votes_then_by_id() {
        let e = election(&[(5, 2), (3, 7), (9, 7), (1, 0)]);
        let ids: Vec<u64> = e.standings().into_iter().map(|(id, _)| id.0).collect();
        assert_eq!(ids, [3, 9, 5, 1]);
    }

    #[test]
    fn a_leader_has_more_votes_than_all_others() {
        assert_eq!(election(&[(5, 2), (3, 7)]).leading(), Some(ResidentId(3)));
        assert_eq!(election(&[(5, 7), (3, 7)]).leading(), None);
        assert_eq!(election(&[(5, 0), (3, 0)]).leading(), None);
        assert_eq!(election(&[(5, 1)]).leading(), Some(ResidentId(5)));
        assert_eq!(election(&[]).leading(), None);
    }
}
