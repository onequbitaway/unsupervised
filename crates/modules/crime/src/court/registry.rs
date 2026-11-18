//! The case registry: every case, custody, trial and bounty of the world.
//! It is state of the global realm and the source of truth for the law.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{CityId, Holder, Pos, ResidentId, Resource, SiteId, ZoneId};

use crate::kinds::CrimeKind;
use crate::msg::{CaseFile, Source};

/// Where a case stands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// The evidence is below the warrant threshold.
    Open,
    /// A warrant names the suspect.
    Wanted,
    /// The suspect is in custody and not charged in this case.
    Held,
    /// A trial runs.
    Trial,
    /// The trial ended with a conviction.
    Convicted,
    /// The trial ended with an acquittal.
    Acquitted,
    /// Nobody was charged before the case got too old.
    Cold,
    /// The perpetrator left the world.
    Moot,
}

impl Status {
    /// Returns the name of the status as verbs and events write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Wanted => "wanted",
            Status::Held => "held",
            Status::Trial => "trial",
            Status::Convicted => "convicted",
            Status::Acquitted => "acquitted",
            Status::Cold => "cold",
            Status::Moot => "moot",
        }
    }

    /// Returns true when nothing more can happen in the case.
    #[must_use]
    pub const fn is_closed(self) -> bool {
        matches!(self, Status::Convicted | Status::Acquitted | Status::Cold | Status::Moot)
    }

    /// Returns true when the registry can name the suspect: a warrant, an
    /// arrest or a verdict did so.
    #[must_use]
    pub const fn names_suspect(self) -> bool {
        matches!(
            self,
            Status::Wanted | Status::Held | Status::Trial | Status::Convicted | Status::Acquitted
        )
    }
}

/// The evidence against the perpetrator of a case, in points by source.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// From victims and witnesses that reported the case.
    pub testimony: i64,
    /// From officers that read the traces at the scene.
    pub investigation: i64,
    /// From a search that found the loot.
    pub search: i64,
    /// From an officer that saw the crime.
    pub caught: i64,
    /// From the city watch.
    pub watch: i64,
}

impl Evidence {
    /// Returns the sum of all sources.
    #[must_use]
    pub fn total(&self) -> i64 {
        [self.testimony, self.investigation, self.search, self.caught, self.watch]
            .into_iter()
            .fold(0i64, i64::saturating_add)
    }

    /// Returns the points of one source.
    pub(crate) fn slot(&mut self, source: Source) -> &mut i64 {
        match source {
            Source::Testimony => &mut self.testimony,
            Source::Investigation => &mut self.investigation,
            Source::Search => &mut self.search,
            Source::Caught => &mut self.caught,
            Source::Watch => &mut self.watch,
        }
    }
}

/// Who decided a trial.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decider {
    /// A resident with `law.judge` (`rule_case`).
    Judge,
    /// The votes of the jury (`jury_vote`).
    Jury,
    /// The engine, from the evidence threshold, at the deadline.
    Evidence,
}

impl Decider {
    /// Returns the name of the decider as events write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Decider::Judge => "judge",
            Decider::Jury => "jury",
            Decider::Evidence => "evidence",
        }
    }
}

/// The end of a trial.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sentence {
    /// True for a conviction.
    pub guilty: bool,
    /// Cents for the city treasury.
    pub fine: i64,
    /// Ticks in jail.
    pub jail_ticks: i64,
    /// Cents for the victim.
    pub restitution: i64,
    /// Who decided.
    pub by: Decider,
    /// The judge, when a judge decided.
    pub judge: Option<ResidentId>,
    /// Frame of the verdict.
    pub frame: u64,
}

/// A juror of a trial and its vote.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Juror {
    pub(crate) resident: ResidentId,
    /// `Some(true)` is a vote for guilty.
    pub(crate) vote: Option<bool>,
}

/// A trial that runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trial {
    /// Frame of the charge.
    pub opened: u64,
    /// Frame at which the engine decides.
    pub deadline: u64,
    /// The officer that charged the suspect. `None` for the engine.
    pub officer: Option<ResidentId>,
    /// True after the engine called a jury.
    pub summoned: bool,
    pub(crate) jury: Vec<Juror>,
}

impl Trial {
    /// Returns the number of jurors.
    #[must_use]
    pub fn jurors(&self) -> usize {
        self.jury.len()
    }

    /// Returns the number of jurors that voted.
    #[must_use]
    pub fn votes_cast(&self) -> usize {
        self.jury.iter().filter(|juror| juror.vote.is_some()).count()
    }

    /// Returns the votes for guilty and the votes for not guilty.
    pub(crate) fn tally(&self) -> (usize, usize) {
        let guilty = self.jury.iter().filter(|juror| juror.vote == Some(true)).count();
        let innocent = self.jury.iter().filter(|juror| juror.vote == Some(false)).count();
        (guilty, innocent)
    }
}

/// One case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Case {
    /// Id of the case.
    pub id: u64,
    /// The kind of crime.
    pub kind: CrimeKind,
    /// The city of the crime.
    pub city: CityId,
    /// The zone of the crime.
    pub zone: ZoneId,
    /// The place of the crime.
    pub pos: Pos,
    /// The site of the crime, if it was in one.
    pub site: Option<SiteId>,
    /// Frame of the crime.
    pub opened: u64,
    /// The resident that did it. It is secret until the status names the
    /// suspect; see [`Case::suspect`].
    pub(crate) perp: ResidentId,
    /// The holder that lost value.
    pub victim: Option<Holder>,
    /// Cents that the victim lost.
    pub value: i64,
    /// True when the crime failed.
    pub attempt: bool,
    /// Residents that noticed the crime.
    pub witnesses: u32,
    /// True after a victim or a witness reported the case.
    pub reported: bool,
    pub(crate) testified: Vec<ResidentId>,
    /// The evidence against the perpetrator.
    pub evidence: Evidence,
    /// Where the case stands.
    pub status: Status,
    /// The trial, while it runs and after it.
    pub trial: Option<Trial>,
    /// The end of the trial.
    pub sentence: Option<Sentence>,
    /// Frame at which the case closed.
    pub closed: Option<u64>,
}

impl Case {
    /// Makes the record of a new case from the file of its zone.
    pub(crate) fn new(file: &CaseFile) -> Case {
        Case {
            id: file.id,
            kind: file.kind,
            city: file.city,
            zone: file.zone,
            pos: file.pos,
            site: file.site,
            opened: file.frame,
            perp: file.perp,
            victim: file.victim,
            value: file.value,
            attempt: file.attempt,
            witnesses: file.witnesses,
            reported: false,
            testified: Vec::new(),
            evidence: Evidence::default(),
            status: Status::Open,
            trial: None,
            sentence: None,
            closed: None,
        }
    }

    /// Returns the suspect, when a warrant, an arrest or a verdict named
    /// it. Before that the perpetrator is secret.
    #[must_use]
    pub fn suspect(&self) -> Option<ResidentId> {
        self.status.names_suspect().then_some(self.perp)
    }

    /// Returns true when residents can see the case on the docket. A
    /// hidden crime (a bribe) is there only after a report or the first
    /// evidence.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        !self.kind.is_hidden() || self.reported || self.evidence.total() > 0
    }
}

/// A resident that is under arrest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Hold {
    /// The officer that made the arrest. `None` for the city watch.
    pub(crate) officer: Option<ResidentId>,
    pub(crate) city: CityId,
    /// Frame of the arrest.
    pub(crate) since: u64,
    /// The cases with a trial that runs.
    pub(crate) trials: Vec<u64>,
}

/// Money that a resident offers for the arrest of another resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounty {
    /// Id of the bounty. The money is in `Holder::Escrow(id)`.
    pub id: u64,
    /// The resident that pays.
    pub poster: ResidentId,
    /// The resident whose arrest earns the money.
    pub target: ResidentId,
    /// Cents in the escrow.
    pub amount: i64,
    /// Frame at which the money goes back to the poster.
    pub expires: u64,
}

/// Something that the registry does at a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Timer {
    /// The time that a suspect can be held without a charge is over.
    Hold { resident: ResidentId, since: u64 },
    /// No judge ruled in time. A jury is called.
    Bench { case: u64 },
    /// The trial ends by the engine.
    TrialEnd { case: u64 },
    /// The time to pay the debts of a case is over.
    FineDue { resident: ResidentId, case: u64 },
    /// A bounty ends.
    BountyEnd { bounty: u64 },
    /// A case without a charge is too old.
    Expire { case: u64 },
    /// A closed case leaves the registry.
    Prune { case: u64 },
}

/// The case registry. A resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cases {
    pub(crate) cases: BTreeMap<u64, Case>,
    /// Perpetrator to its cases that are not closed.
    pub(crate) by_perp: BTreeMap<ResidentId, Vec<u64>>,
    pub(crate) holds: BTreeMap<ResidentId, Hold>,
    pub(crate) bounties: BTreeMap<u64, Bounty>,
    /// Frame to what is due at that frame, in the order of arrival.
    pub(crate) timers: BTreeMap<u64, Vec<Timer>>,
}

impl Resource for Cases {
    const NAME: &'static str = "crime.cases";
}

impl Cases {
    /// Returns one case.
    #[must_use]
    pub fn get(&self, case: u64) -> Option<&Case> {
        self.cases.get(&case)
    }

    /// Returns the number of cases in the registry, open and closed.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cases.len()
    }

    /// Returns true when the registry has no case.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cases.is_empty()
    }

    /// Returns every case, ordered by id.
    pub fn iter(&self) -> impl Iterator<Item = &Case> + '_ {
        self.cases.values()
    }

    /// Returns true when the resident is under arrest.
    #[must_use]
    pub fn in_custody(&self, resident: ResidentId) -> bool {
        self.holds.contains_key(&resident)
    }

    /// Returns the open bounties, ordered by id.
    pub fn bounties(&self) -> impl Iterator<Item = &Bounty> + '_ {
        self.bounties.values()
    }

    /// Plans something for a frame.
    pub(crate) fn at(&mut self, frame: u64, timer: Timer) {
        self.timers.entry(frame).or_default().push(timer);
    }

    /// Takes everything that is due at `frame` or before it, oldest first.
    pub(crate) fn take_due(&mut self, frame: u64) -> Vec<Timer> {
        let mut due = Vec::new();
        while let Some(entry) = self.timers.first_entry() {
            if *entry.key() > frame {
                break;
            }
            due.extend(entry.remove());
        }
        due
    }

    /// Adds a case.
    pub(crate) fn insert(&mut self, case: Case) {
        self.by_perp.entry(case.perp).or_default().push(case.id);
        self.cases.insert(case.id, case);
    }

    /// Closes a case with a status. It leaves the list of its perpetrator.
    pub(crate) fn close(&mut self, id: u64, status: Status, frame: u64) {
        let Some(case) = self.cases.get_mut(&id) else {
            return;
        };
        case.status = status;
        case.closed = Some(frame);
        let perp = case.perp;
        if let Some(list) = self.by_perp.get_mut(&perp) {
            list.retain(|other| *other != id);
            if list.is_empty() {
                self.by_perp.remove(&perp);
            }
        }
    }

    /// Returns the cases of a perpetrator that are not closed.
    pub(crate) fn open_of(&self, perp: ResidentId) -> Vec<u64> {
        self.by_perp.get(&perp).cloned().unwrap_or_default()
    }

    /// Removes a closed case.
    pub(crate) fn prune(&mut self, id: u64) {
        if self.cases.get(&id).is_some_and(|case| case.status.is_closed()) {
            self.cases.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: u64, perp: u64) -> CaseFile {
        CaseFile {
            id,
            frame: 5,
            kind: CrimeKind::Theft,
            city: CityId(0),
            zone: ZoneId(0),
            pos: Pos::new(1, 2),
            site: None,
            perp: ResidentId(perp),
            victim: None,
            value: 10,
            attempt: false,
            witnesses: 0,
            caught: false,
            watch: false,
        }
    }

    #[test]
    fn timers_come_due_in_frame_order_then_arrival_order() {
        let mut cases = Cases::default();
        cases.at(20, Timer::Prune { case: 3 });
        cases.at(10, Timer::Bench { case: 1 });
        cases.at(10, Timer::TrialEnd { case: 2 });
        assert_eq!(cases.take_due(9), []);
        assert_eq!(cases.take_due(15), [Timer::Bench { case: 1 }, Timer::TrialEnd { case: 2 }]);
        assert_eq!(cases.take_due(15), []);
        assert_eq!(cases.take_due(99), [Timer::Prune { case: 3 }]);
        assert!(cases.timers.is_empty());
    }

    #[test]
    fn a_closed_case_leaves_the_list_of_its_perpetrator_and_can_be_pruned() {
        let mut cases = Cases::default();
        cases.insert(Case::new(&file(1, 7)));
        cases.insert(Case::new(&file(2, 7)));
        assert_eq!(cases.open_of(ResidentId(7)), [1, 2]);
        // An open case is never pruned.
        cases.prune(1);
        assert_eq!(cases.len(), 2);
        cases.close(1, Status::Cold, 50);
        assert_eq!(cases.open_of(ResidentId(7)), [2]);
        assert_eq!(cases.get(1).map(|c| (c.status, c.closed)), Some((Status::Cold, Some(50))));
        cases.prune(1);
        assert_eq!(cases.len(), 1);
        cases.close(2, Status::Moot, 60);
        assert!(cases.by_perp.is_empty());
    }

    #[test]
    fn the_suspect_is_secret_until_the_status_names_it() {
        let mut case = Case::new(&file(1, 7));
        assert_eq!(case.suspect(), None);
        case.status = Status::Wanted;
        assert_eq!(case.suspect(), Some(ResidentId(7)));
        case.status = Status::Cold;
        assert_eq!(case.suspect(), None);
        // A bribe is not on the docket before the first evidence.
        case.kind = CrimeKind::Bribery;
        assert!(!case.is_visible());
        case.evidence.investigation = 3;
        assert!(case.is_visible());
        assert_eq!(case.evidence.total(), 3);
    }
}
