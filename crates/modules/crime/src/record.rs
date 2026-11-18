//! The per-resident state of the crime module: the criminal record.
//!
//! The record moves with the resident between zones. The case registry of
//! the global realm is the source of truth for cases, warrants and trials;
//! the record holds what the zone of the resident must know at once: the
//! hot loot, the open warrants, who caught the resident in the act, the
//! crimes that the resident saw, its debts to the court and its custody.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Component, Holder, ItemId, ResidentId, SiteId, Value, ZoneId};

use crate::kinds::CrimeKind;

/// Stolen value that a resident still holds: cash (no item) or units of
/// one good, from one case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotLot {
    /// The case of the theft.
    #[serde(rename = "c")]
    pub case: u64,
    /// The good, or `None` for cash.
    #[serde(rename = "i", default, skip_serializing_if = "Option::is_none")]
    pub item: Option<ItemId>,
    /// Units of the good, or cents of cash.
    #[serde(rename = "q")]
    pub qty: i64,
    /// Frame of the theft.
    #[serde(rename = "f")]
    pub frame: u64,
}

/// A warrant that names the resident.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warrant {
    /// The case.
    #[serde(rename = "c")]
    pub case: u64,
    /// The kind of crime.
    #[serde(rename = "k")]
    pub kind: CrimeKind,
}

/// An officer saw the resident commit a crime. The officer can arrest the
/// resident without a warrant until the frame `until`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Caught {
    #[serde(rename = "c")]
    pub(crate) case: u64,
    #[serde(rename = "o")]
    pub(crate) officer: ResidentId,
    #[serde(rename = "u")]
    pub(crate) until: u64,
}

/// How a resident knows a case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The crime was against the resident or its site.
    Victim,
    /// The resident saw the crime.
    Witness,
    /// The resident got the money of a bribe.
    Receiver,
}

impl Role {
    /// Returns the name of the role as notices write it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Role::Victim => "victim",
            Role::Witness => "witness",
            Role::Receiver => "receiver",
        }
    }

    /// Returns true when the testimony of this role has the weight of the
    /// party that the crime touched (`crime.evidence_victim`).
    pub(crate) const fn is_direct(self) -> bool {
        matches!(self, Role::Victim | Role::Receiver)
    }
}

/// A crime that the resident knows as a victim or a witness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Known {
    /// The case.
    #[serde(rename = "c")]
    pub case: u64,
    /// The kind of crime.
    #[serde(rename = "k")]
    pub kind: CrimeKind,
    /// How the resident knows the case.
    #[serde(rename = "r")]
    pub role: Role,
    /// The perpetrator, when the resident recognized it.
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub suspect: Option<ResidentId>,
    /// True after `report_crime`.
    #[serde(rename = "d", default, skip_serializing_if = "is_false")]
    pub reported: bool,
}

/// Money that a convict owes: a fine to a city or a restitution to a
/// victim.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Debt {
    /// The case of the sentence.
    #[serde(rename = "c")]
    pub case: u64,
    /// The holder that gets the money.
    #[serde(rename = "t")]
    pub to: Holder,
    /// The zone of the holder when it is a site.
    #[serde(rename = "z", default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<ZoneId>,
    /// Cents that are still owed.
    #[serde(rename = "a")]
    pub amount: i64,
    /// Frame at which the rest becomes jail time.
    #[serde(rename = "d")]
    pub due: u64,
    /// True for a fine, false for a restitution.
    #[serde(rename = "f")]
    pub fine: bool,
}

/// The custody of a resident that is under arrest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Custody {
    /// Frame of the arrest.
    #[serde(rename = "f")]
    pub since: u64,
    /// The officer that made the arrest. `None` for the city watch.
    #[serde(rename = "o", default, skip_serializing_if = "Option::is_none")]
    pub officer: Option<ResidentId>,
    /// The jail that holds the resident, when the city has one.
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    pub site: Option<SiteId>,
}

/// A trial or a jury duty that the resident must know of.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Docket {
    /// The case.
    #[serde(rename = "c")]
    pub case: u64,
    /// The kind of crime.
    #[serde(rename = "k")]
    pub kind: CrimeKind,
    /// Frame at which the engine decides the trial.
    #[serde(rename = "d")]
    pub deadline: u64,
}

/// The result of an action that did its work when it started. The
/// action report gets it when the action ends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Stash {
    /// Id of the action.
    #[serde(rename = "a")]
    action: u64,
    /// The result as JSON text.
    #[serde(rename = "r")]
    result: String,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The criminal record of a resident. It moves with the resident between
/// zones.
///
/// Field names are short on the wire because one value is written for each
/// resident in every snapshot. A resident without a record writes an empty
/// map.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    #[serde(rename = "h", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) hot: Vec<HotLot>,
    #[serde(rename = "w", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) warrants: Vec<Warrant>,
    #[serde(rename = "c", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) caught: Vec<Caught>,
    #[serde(rename = "k", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) known: Vec<Known>,
    #[serde(rename = "d", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) debts: Vec<Debt>,
    #[serde(rename = "u", default, skip_serializing_if = "Option::is_none")]
    pub(crate) custody: Option<Custody>,
    #[serde(rename = "t", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) trials: Vec<Docket>,
    #[serde(rename = "j", default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) jury: Vec<Docket>,
    #[serde(rename = "s", default, skip_serializing_if = "Option::is_none")]
    stash: Option<Stash>,
}

impl Component for Record {
    const NAME: &'static str = "crime.record";
}

impl Record {
    /// Returns the stolen value that the record marks as hot. A lot can be
    /// larger than what the resident still holds; a search and `fence`
    /// count only what is held.
    #[must_use]
    pub fn hot(&self) -> &[HotLot] {
        &self.hot
    }

    /// Returns the open warrants that name the resident.
    #[must_use]
    pub fn warrants(&self) -> &[Warrant] {
        &self.warrants
    }

    /// Returns the crimes that the resident knows as a victim or a
    /// witness, oldest first.
    #[must_use]
    pub fn known(&self) -> &[Known] {
        &self.known
    }

    /// Returns the fines and restitutions that the resident owes.
    #[must_use]
    pub fn debts(&self) -> &[Debt] {
        &self.debts
    }

    /// Returns the custody of the resident, when it is under arrest.
    #[must_use]
    pub fn custody(&self) -> Option<Custody> {
        self.custody
    }

    /// Returns the trials in which the resident is the defendant.
    #[must_use]
    pub fn trials(&self) -> &[Docket] {
        &self.trials
    }

    /// Returns the trials in which the resident is a juror that has not
    /// voted.
    #[must_use]
    pub fn jury_duties(&self) -> &[Docket] {
        &self.jury
    }

    /// Returns the cents that the resident owes to the court.
    #[must_use]
    pub fn owed(&self) -> i64 {
        self.debts.iter().fold(0i64, |sum, debt| sum.saturating_add(debt.amount))
    }

    /// Returns the hot units of a good, or the hot cents for `None`, that
    /// the record marks.
    pub(crate) fn hot_of(&self, item: Option<ItemId>) -> i64 {
        self.hot
            .iter()
            .filter(|lot| lot.item == item)
            .fold(0i64, |sum, lot| sum.saturating_add(lot.qty))
    }

    /// Marks stolen value as hot. The record keeps at most `max` lots; the
    /// oldest lot goes first.
    pub(crate) fn add_hot(&mut self, lot: HotLot, max: usize) {
        if lot.qty <= 0 || max == 0 {
            return;
        }
        match self.hot.iter_mut().find(|old| old.case == lot.case && old.item == lot.item) {
            Some(old) => old.qty = old.qty.saturating_add(lot.qty),
            None => self.hot.push(lot),
        }
        if self.hot.len() > max {
            let extra = self.hot.len() - max;
            self.hot.drain(..extra);
        }
    }

    /// Removes the lots that are older than `ttl` frames.
    pub(crate) fn cool(&mut self, frame: u64, ttl: u64) {
        self.hot.retain(|lot| frame.saturating_sub(lot.frame) < ttl);
    }

    /// Lowers the hot amount of one good to what the resident still holds.
    /// The oldest lots shrink first: value that left the hands of the
    /// resident is no longer hot.
    pub(crate) fn clamp_hot(&mut self, item: Option<ItemId>, held: i64) {
        let extra = self.hot_of(item) - held.max(0);
        if extra > 0 {
            self.take_hot(item, extra);
        }
    }

    /// Takes up to `qty` hot units of one good from the record, oldest
    /// lots first. Returns what each case lost.
    pub(crate) fn take_hot(&mut self, item: Option<ItemId>, qty: i64) -> Vec<(u64, i64)> {
        let mut left = qty.max(0);
        let mut taken = Vec::new();
        for lot in self.hot.iter_mut().filter(|lot| lot.item == item) {
            if left == 0 {
                break;
            }
            let part = lot.qty.min(left);
            lot.qty -= part;
            left -= part;
            taken.push((lot.case, part));
        }
        self.hot.retain(|lot| lot.qty > 0);
        taken
    }

    /// Returns the goods and the cash (`None`) that have hot lots, each
    /// one time, in the order of the lots.
    pub(crate) fn hot_kinds(&self) -> Vec<Option<ItemId>> {
        let mut kinds: Vec<Option<ItemId>> = Vec::new();
        for lot in &self.hot {
            if !kinds.contains(&lot.item) {
                kinds.push(lot.item);
            }
        }
        kinds
    }

    /// Adds a warrant unless the record has it.
    pub(crate) fn add_warrant(&mut self, warrant: Warrant) {
        if !self.warrants.iter().any(|w| w.case == warrant.case) {
            self.warrants.push(warrant);
        }
    }

    /// Removes the warrant of a case. Returns true when there was one.
    pub(crate) fn drop_warrant(&mut self, case: u64) -> bool {
        let before = self.warrants.len();
        self.warrants.retain(|w| w.case != case);
        self.warrants.len() != before
    }

    /// Notes that an officer saw the resident in the act. The record keeps
    /// at most `max` such notes.
    pub(crate) fn add_caught(&mut self, caught: Caught, max: usize) {
        if max == 0 {
            return;
        }
        self.caught.retain(|c| !(c.case == caught.case && c.officer == caught.officer));
        self.caught.push(caught);
        if self.caught.len() > max {
            let extra = self.caught.len() - max;
            self.caught.drain(..extra);
        }
    }

    /// Returns the cases in which the officer caught the resident in the
    /// act and can still arrest it for that.
    pub(crate) fn caught_by(&self, officer: ResidentId, frame: u64) -> Vec<u64> {
        self.caught
            .iter()
            .filter(|c| c.officer == officer && c.until >= frame)
            .map(|c| c.case)
            .collect()
    }

    /// Notes a crime that the resident saw. The record keeps at most `max`
    /// such notes; the oldest goes first.
    pub(crate) fn add_known(&mut self, known: Known, max: usize) {
        if max == 0 {
            return;
        }
        if let Some(old) = self.known.iter_mut().find(|k| k.case == known.case) {
            // The victim is also a witness when it was there. Keep the
            // stronger role and what was seen.
            if known.role.is_direct() {
                old.role = known.role;
            }
            old.suspect = old.suspect.or(known.suspect);
            return;
        }
        self.known.push(known);
        if self.known.len() > max {
            let extra = self.known.len() - max;
            self.known.drain(..extra);
        }
    }

    /// Adds a debt.
    pub(crate) fn add_debt(&mut self, debt: Debt) {
        if debt.amount > 0 {
            self.debts.push(debt);
        }
    }

    /// Keeps the result of an action until the action ends. A record has
    /// room for one result: a resident runs one such action at a time.
    pub(crate) fn put_result(&mut self, action: u64, result: &Value) {
        self.stash = Some(Stash { action, result: result.to_string() });
    }

    /// Takes the result that `put_result` kept for an action.
    pub(crate) fn take_result(&mut self, action: u64) -> Option<Value> {
        let stash = self.stash.take_if(|stash| stash.action == action)?;
        serde_json::from_str(&stash.result).ok()
    }

    /// Adds a trial or replaces the one of the same case.
    pub(crate) fn add_trial(&mut self, trial: Docket) {
        self.trials.retain(|t| t.case != trial.case);
        self.trials.push(trial);
    }

    /// Adds a jury duty or replaces the one of the same case.
    pub(crate) fn add_jury(&mut self, duty: Docket) {
        self.jury.retain(|t| t.case != duty.case);
        self.jury.push(duty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lot(case: u64, item: Option<u16>, qty: i64, frame: u64) -> HotLot {
        HotLot { case, item: item.map(ItemId), qty, frame }
    }

    #[test]
    fn hot_lots_merge_shrink_oldest_first_and_cool() {
        let mut record = Record::default();
        record.add_hot(lot(1, None, 500, 10), 4);
        record.add_hot(lot(2, None, 300, 20), 4);
        record.add_hot(lot(1, None, 100, 30), 4);
        record.add_hot(lot(2, Some(7), 3, 20), 4);
        assert_eq!(record.hot_of(None), 900);
        assert_eq!(record.hot_of(Some(ItemId(7))), 3);
        assert_eq!(record.hot.len(), 3);

        // The resident spent cash: 650 are left. The oldest lot shrinks.
        record.clamp_hot(None, 650);
        assert_eq!(record.hot_of(None), 650);
        assert_eq!(record.hot[0], lot(1, None, 350, 10));

        // A search takes 400: all of case 1 and 50 of case 2.
        assert_eq!(record.take_hot(None, 400), [(1, 350), (2, 50)]);
        assert_eq!(record.hot_of(None), 250);
        assert_eq!(record.hot_kinds(), [None, Some(ItemId(7))]);

        // Lots of frame 20 are cold at frame 120 with a life of 100.
        record.cool(119, 100);
        assert_eq!(record.hot.len(), 2);
        record.cool(120, 100);
        assert!(record.hot.is_empty());
    }

    #[test]
    fn the_lists_are_bounded() {
        let mut record = Record::default();
        for case in 1..=5 {
            record.add_hot(lot(case, None, 10, case), 3);
            record.add_known(
                Known {
                    case,
                    kind: CrimeKind::Theft,
                    role: Role::Witness,
                    suspect: None,
                    reported: false,
                },
                2,
            );
            record.add_caught(Caught { case, officer: ResidentId(9), until: 100 + case }, 2);
        }
        assert_eq!(record.hot.iter().map(|l| l.case).collect::<Vec<_>>(), [3, 4, 5]);
        assert_eq!(record.known.iter().map(|k| k.case).collect::<Vec<_>>(), [4, 5]);
        assert_eq!(record.caught_by(ResidentId(9), 105), [5]);
        assert_eq!(record.caught_by(ResidentId(9), 104), [4, 5]);
        assert_eq!(record.caught_by(ResidentId(8), 0), Vec::<u64>::new());
    }

    #[test]
    fn a_victim_that_was_there_keeps_what_it_saw() {
        let mut record = Record::default();
        let seen = Known {
            case: 4,
            kind: CrimeKind::Burglary,
            role: Role::Witness,
            suspect: Some(ResidentId(3)),
            reported: false,
        };
        record.add_known(seen, 4);
        record.add_known(Known { role: Role::Victim, suspect: None, ..seen }, 4);
        assert_eq!(record.known, [Known { role: Role::Victim, ..seen }]);
    }

    #[test]
    fn an_empty_record_is_an_empty_map_on_the_wire() {
        let wire = serde_json::to_value(Record::default()).expect("a record is JSON");
        assert_eq!(wire, serde_json::json!({}));
    }
}
