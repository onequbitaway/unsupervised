//! The state of the module. It is one resource of the global realm.

pub(crate) mod bill;
pub(crate) mod election;
pub(crate) mod recall;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use unsup_kernel::{OrgId, ResidentId, Resource};

use self::bill::{Bill, Law};
use self::election::{Cause, Election};
use self::recall::Recall;
use crate::model::{Gov, Office};

/// A rally that a candidate paid for and that is not over yet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Rally {
    /// The candidate.
    pub(crate) resident: ResidentId,
    /// The election.
    pub(crate) election: u64,
    /// The government of the office.
    pub(crate) gov: Gov,
    /// The office.
    pub(crate) office: Office,
    /// The platform of the candidate when the rally started.
    pub(crate) platform: String,
    /// Frame at which the rally ends.
    pub(crate) due: u64,
    /// Cents that the candidate spent.
    pub(crate) spend: i64,
    /// Tiles at which residents hear the rally.
    pub(crate) radius: u32,
}

/// The public state that changed since the last publication.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Dirty {
    /// Governments whose offices, posts or petitions changed.
    pub(crate) governments: BTreeSet<OrgId>,
    /// Elections whose candidates, counts or phase changed.
    pub(crate) elections: BTreeSet<u64>,
    /// Governments whose bills or laws changed.
    pub(crate) bills: BTreeSet<OrgId>,
}

impl Dirty {
    /// Returns true when nothing waits for a publication.
    pub(crate) fn is_empty(&self) -> bool {
        self.governments.is_empty() && self.elections.is_empty() && self.bills.is_empty()
    }
}

/// Elections, bills, laws and petitions. A resource of the global realm.
///
/// The holders of the offices are not here: the registry of the society
/// module has them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Politics {
    /// True after the set-up defined the roles and made the calendar.
    pub(crate) ready: bool,
    /// The elections that are not decided, by id.
    pub(crate) elections: BTreeMap<u64, Election>,
    /// The next regular voting day of each government, by its
    /// organization, as a day of the world from 0.
    pub(crate) next_vote: BTreeMap<OrgId, u64>,
    /// The rallies that are not over, by action id.
    pub(crate) rallies: BTreeMap<u64, Rally>,
    /// The bills that are not closed, by id.
    pub(crate) bills: BTreeMap<u64, Bill>,
    /// The last closed bills of each government, oldest first.
    pub(crate) closed: BTreeMap<OrgId, VecDeque<Bill>>,
    /// The laws in force of each government, by parameter name.
    pub(crate) laws: BTreeMap<OrgId, BTreeMap<String, Law>>,
    /// The open recall petitions, by id.
    pub(crate) recalls: BTreeMap<u64, Recall>,
    /// The last day whose start paid the salaries.
    pub(crate) salary_day: u64,
    /// The last day whose start published all public state again.
    pub(crate) refresh_day: u64,
    /// What changed since the last publication.
    pub(crate) dirty: Dirty,
    /// The first frame at which the next publication can happen.
    pub(crate) next_publish: u64,
}

impl Resource for Politics {
    const NAME: &'static str = "politics.state";
}

impl Politics {
    /// Returns true when a regular election of a government for a voting
    /// day is open.
    pub(crate) fn has_regular(&self, gov: Gov, vote_day: u64) -> bool {
        self.elections
            .values()
            .any(|e| e.gov == gov && e.cause == Cause::Regular && e.vote_day == vote_day)
    }

    /// Returns the open election for an office of a government, if there is
    /// one: the one with the lowest id.
    pub(crate) fn election_for(&self, gov: Gov, office: Office) -> Option<&Election> {
        self.elections.values().find(|e| e.gov == gov && e.office == office)
    }

    /// Returns the open recall petition against an office of a government.
    pub(crate) fn recall_for(&self, gov: Gov, office: Office) -> Option<&Recall> {
        self.recalls.values().find(|r| r.gov == gov && r.office == office)
    }

    /// Returns the open bills of a government, ordered by id.
    pub(crate) fn bills_of(&self, gov: Gov) -> impl Iterator<Item = &Bill> + '_ {
        self.bills.values().filter(move |bill| bill.gov == gov)
    }

    /// Records that the offices, posts or petitions of a government
    /// changed.
    pub(crate) fn touch_government(&mut self, gov: Gov) {
        self.dirty.governments.insert(gov.org());
    }

    /// Records that an election changed.
    pub(crate) fn touch_election(&mut self, election: u64) {
        self.dirty.elections.insert(election);
    }

    /// Records that the bills or the laws of a government changed.
    pub(crate) fn touch_bills(&mut self, gov: Gov) {
        self.dirty.bills.insert(gov.org());
    }

    /// Puts a closed bill in the register of its government and keeps the
    /// last `keep` bills there.
    pub(crate) fn file(&mut self, bill: Bill, keep: usize) {
        let gov = bill.gov;
        let register = self.closed.entry(gov.org()).or_default();
        register.push_back(bill);
        while register.len() > keep {
            register.pop_front();
        }
        self.touch_bills(gov);
    }
}
