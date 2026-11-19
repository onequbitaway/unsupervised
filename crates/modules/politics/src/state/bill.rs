//! The record of one bill and of one law.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use unsup_kernel::ResidentId;

use crate::model::Gov;

/// How a bill became law.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum How {
    /// The executive signed it.
    Signed,
    /// The executive did not act before the deadline.
    Unsigned,
    /// The executive office was empty.
    Vacant,
    /// The members overrode a veto.
    Override,
}

impl How {
    /// Returns the name in events and published state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            How::Signed => "signed",
            How::Unsigned => "deadline",
            How::Vacant => "vacant",
            How::Override => "override",
        }
    }
}

/// Why a bill did not become law.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum Why {
    /// Too few members voted.
    NoQuorum,
    /// The votes against were not fewer than the votes in favour.
    VotedDown,
    /// The executive vetoed the bill and the members did not override.
    Vetoed,
    /// No policy parameter has the name any more.
    NoParameter,
}

impl Why {
    /// Returns the name in events and published state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Why::NoQuorum => "no_quorum",
            Why::VotedDown => "voted_down",
            Why::Vetoed => "vetoed",
            Why::NoParameter => "no_parameter",
        }
    }
}

/// Where a bill is on its way.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub(crate) enum Stage {
    /// The members vote.
    Vote,
    /// The bill passed. The executive signs or vetoes.
    Sign,
    /// The executive vetoed. The members vote on the override.
    Override,
    /// The bill is law.
    Enacted(How),
    /// The bill failed.
    Rejected(Why),
}

impl Stage {
    /// Returns the name in results, events and published state.
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Stage::Vote => "vote",
            Stage::Sign => "sign",
            Stage::Override => "override",
            Stage::Enacted(_) => "enacted",
            Stage::Rejected(_) => "rejected",
        }
    }

    /// Returns true while members can vote.
    pub(crate) const fn takes_votes(self) -> bool {
        matches!(self, Stage::Vote | Stage::Override)
    }
}

/// One bill: a policy parameter and a new value for one government.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Bill {
    /// Id of the bill.
    pub(crate) id: u64,
    /// The government whose members decide. A city bill changes the
    /// parameter in that city; a national bill changes the national value.
    pub(crate) gov: Gov,
    /// Name of the policy parameter.
    pub(crate) param: String,
    /// The new value.
    pub(crate) value: i64,
    /// The value in force when the bill was proposed.
    pub(crate) old: i64,
    /// The member that proposed the bill.
    pub(crate) sponsor: ResidentId,
    /// The reason that the sponsor wrote, if it wrote one. It is text of a
    /// resident: it leaves the module only as `Untrusted`.
    pub(crate) reason: Option<String>,
    /// Frame of the proposal.
    pub(crate) proposed: u64,
    /// Where the bill is.
    pub(crate) stage: Stage,
    /// Frame at which the current stage ends, or at which the bill closed.
    pub(crate) deadline: u64,
    /// The electorate of the current stage: the residents that had
    /// `gov.vote` when the stage began. That is the proposal for the stage
    /// `vote` and the veto for the stage `override`. Only they vote in the
    /// stage, and the quorum counts them, also when they left afterwards.
    ///
    /// `None` only in a bill of a snapshot from before this rule. Such a
    /// bill gets the members of its first use; see [`Bill::electorate`].
    #[serde(default)]
    pub(crate) electorate: Option<BTreeSet<ResidentId>>,
    /// Members of the electorate that voted in favour in the current stage.
    /// A vote stays when its member leaves.
    pub(crate) yes: BTreeSet<ResidentId>,
    /// Members of the electorate that voted against in the current stage.
    pub(crate) no: BTreeSet<ResidentId>,
    /// The count of the first vote, after it passed: in favour, against.
    pub(crate) passed: Option<(u32, u32)>,
    /// The executive that vetoed the bill.
    pub(crate) vetoed_by: Option<ResidentId>,
}

impl Bill {
    /// Returns true when a member voted in the current stage.
    pub(crate) fn has_voted(&self, member: ResidentId) -> bool {
        self.yes.contains(&member) || self.no.contains(&member)
    }

    /// Starts a stage in which members vote: nobody has voted yet, and the
    /// electorate is `members`, the residents with `gov.vote` now.
    pub(crate) fn open_vote(&mut self, members: impl IntoIterator<Item = ResidentId>) {
        self.yes.clear();
        self.no.clear();
        self.electorate = Some(members.into_iter().collect());
    }

    /// Returns the electorate of the current stage. A bill of a snapshot
    /// from before the rule has none: it gets `members()`, the residents
    /// with `gov.vote` now, and keeps them.
    pub(crate) fn electorate(
        &mut self,
        members: impl FnOnce() -> Vec<ResidentId>,
    ) -> &BTreeSet<ResidentId> {
        self.electorate.get_or_insert_with(|| members().into_iter().collect())
    }

    /// Returns true when a resident is in the electorate of the current
    /// stage. A bill without an electorate (see [`Bill::electorate`]) does
    /// not say no.
    pub(crate) fn admits(&self, resident: ResidentId) -> bool {
        self.electorate.as_ref().is_none_or(|electorate| electorate.contains(&resident))
    }

    /// Returns the votes of the current stage: in favour, against. Every
    /// vote of the electorate counts, also that of a member that left its
    /// seat afterwards.
    pub(crate) fn count(&self) -> (usize, usize) {
        (self.yes.len(), self.no.len())
    }
}

/// A law in force: the newest enacted bill for one parameter of one
/// government.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Law {
    /// The bill that became this law.
    pub(crate) bill: u64,
    /// The value that the law set.
    pub(crate) value: i64,
    /// The value before the law.
    pub(crate) old: i64,
    /// Frame at which the law took effect.
    pub(crate) enacted: u64,
    /// How the bill became law.
    pub(crate) how: How,
}

/// Returns the number of votes that a decision needs when its electorate
/// has `members` members: `permille` thousandths of them, rounded up, and
/// at least one.
pub(crate) fn quorum(members: usize, permille: i64) -> usize {
    let permille = usize::try_from(permille.clamp(0, 1000)).unwrap_or(1000);
    (members * permille).div_ceil(1000).max(1)
}

/// Returns true when the votes in favour are at least `num / den` of the
/// votes cast, and somebody voted.
pub(crate) fn reaches(yes: usize, cast: usize, num: i64, den: i64) -> bool {
    let num = u128::try_from(num.max(0)).unwrap_or(0);
    let den = u128::try_from(den.max(1)).unwrap_or(1);
    cast > 0 && (yes as u128) * den >= (cast as u128) * num
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_quorum_rounds_up_and_is_at_least_one() {
        assert_eq!(quorum(5, 500), 3);
        assert_eq!(quorum(4, 500), 2);
        assert_eq!(quorum(1, 500), 1);
        assert_eq!(quorum(0, 500), 1);
        assert_eq!(quorum(7, 0), 1);
        assert_eq!(quorum(7, 1000), 7);
        assert_eq!(quorum(7, 5000), 7);
    }

    fn bill() -> Bill {
        Bill {
            id: 1,
            gov: Gov::Nation,
            param: "x".into(),
            value: 1,
            old: 0,
            sponsor: ResidentId(1),
            reason: None,
            proposed: 0,
            stage: Stage::Vote,
            deadline: 10,
            electorate: None,
            yes: BTreeSet::new(),
            no: BTreeSet::new(),
            passed: None,
            vetoed_by: None,
        }
    }

    #[test]
    fn a_bill_without_an_electorate_gets_the_members_of_its_first_use() {
        // A bill of a snapshot from before the rule.
        let mut old = bill();
        assert!(old.admits(ResidentId(9)), "nobody is shut out before the first use");
        let first: Vec<u64> =
            old.electorate(|| vec![ResidentId(2), ResidentId(1)]).iter().map(|r| r.0).collect();
        assert_eq!(first, [1, 2]);
        // The electorate is fixed now: a later list changes nothing.
        let later: Vec<u64> = old.electorate(|| vec![ResidentId(7)]).iter().map(|r| r.0).collect();
        assert_eq!(later, [1, 2]);
        assert!(old.admits(ResidentId(2)));
        assert!(!old.admits(ResidentId(7)));
    }

    #[test]
    fn a_new_stage_has_a_new_electorate_and_no_votes() {
        let mut bill = bill();
        bill.open_vote([ResidentId(1), ResidentId(2)]);
        bill.yes.insert(ResidentId(1));
        bill.no.insert(ResidentId(2));
        assert_eq!(bill.count(), (1, 1));
        bill.open_vote([ResidentId(2), ResidentId(3)]);
        assert_eq!(bill.count(), (0, 0));
        assert!(!bill.admits(ResidentId(1)));
        assert!(bill.admits(ResidentId(3)));
        assert!(!bill.has_voted(ResidentId(2)));
    }

    #[test]
    fn two_thirds_is_exact() {
        assert!(reaches(2, 3, 2, 3));
        assert!(!reaches(1, 3, 2, 3));
        assert!(reaches(4, 6, 2, 3));
        assert!(!reaches(3, 5, 2, 3));
        assert!(reaches(4, 5, 2, 3));
        assert!(!reaches(0, 0, 2, 3));
        assert!(reaches(1, 1, 2, 3));
    }
}
