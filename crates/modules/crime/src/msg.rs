//! The messages of the module.
//!
//! A zone tells the case registry of the global realm what happened
//! ([`Report`]). The registry tells the zone of a resident what to do with
//! that resident ([`Order`]). The zone carries out every order by itself.

use serde::{Deserialize, Serialize};
use unsup_kernel::{CityId, Holder, Message, Pos, ResidentId, SiteId, ZoneId};

use crate::kinds::CrimeKind;

/// Where a piece of evidence comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// A victim or a witness told what it saw (`report_crime`).
    Testimony,
    /// An officer read the traces at the scene (`investigate`).
    Investigation,
    /// A search found the loot (`search`).
    Search,
    /// An officer saw the crime.
    Caught,
    /// The city watch saw the crime.
    Watch,
}

/// A new case, as the zone of the crime writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CaseFile {
    pub(crate) id: u64,
    /// Frame of the crime.
    pub(crate) frame: u64,
    pub(crate) kind: CrimeKind,
    pub(crate) city: CityId,
    pub(crate) zone: ZoneId,
    pub(crate) pos: Pos,
    pub(crate) site: Option<SiteId>,
    /// The resident that did it. The registry keeps it secret.
    pub(crate) perp: ResidentId,
    pub(crate) victim: Option<Holder>,
    /// Cents that the victim lost.
    pub(crate) value: i64,
    /// True when the crime failed.
    pub(crate) attempt: bool,
    /// Residents that noticed the crime.
    pub(crate) witnesses: u32,
    /// True when an officer saw the perpetrator.
    pub(crate) caught: bool,
    /// True when the city watch saw the perpetrator.
    pub(crate) watch: bool,
}

/// What a zone tells the case registry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Report {
    /// A crime happened.
    Opened(CaseFile),
    /// More evidence in a case.
    Evidence { case: u64, source: Source, points: i64 },
    /// A victim or a witness reported a case.
    Testimony { case: u64, witness: ResidentId, direct: bool, identified: bool },
    /// A resident was arrested in the frame `frame` and is in custody now.
    Arrested { target: ResidentId, officer: Option<ResidentId>, city: CityId, frame: u64 },
    /// An officer let a suspect go that was not charged.
    Released { target: ResidentId },
}

impl Message for Report {
    const KIND: &'static str = "crime.report";
}

/// The end of a trial, for the zone of the defendant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Verdict {
    pub(crate) case: u64,
    pub(crate) kind: CrimeKind,
    pub(crate) guilty: bool,
    /// Cents for the city treasury.
    pub(crate) fine: i64,
    /// Ticks in jail.
    pub(crate) jail_ticks: i64,
    /// Cents for the victim.
    pub(crate) restitution: i64,
    pub(crate) victim: Option<Holder>,
    /// The zone of the crime: the zone of a victim that is a site.
    pub(crate) zone: ZoneId,
    pub(crate) city: CityId,
    /// Frame at which what is still owed becomes jail time.
    pub(crate) due: u64,
    /// True when no other trial holds the resident in custody.
    pub(crate) last: bool,
}

/// What the case registry, or another zone, tells the zone of a resident.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Order {
    /// A warrant names the resident (`on`), or the warrant ended.
    Warrant { case: u64, kind: CrimeKind, on: bool },
    /// A crime against a site of the resident, which was not there.
    Heard { case: u64, kind: CrimeKind, site: SiteId, value: i64 },
    /// The resident was charged. A trial runs until `deadline`.
    Charged { case: u64, kind: CrimeKind, deadline: u64 },
    /// A trial of the resident ended.
    Verdict(Verdict),
    /// The time to pay the debts of a case is over.
    Due { case: u64 },
    /// The custody of the resident ends without a sentence.
    Free { reason: String, by: Option<ResidentId> },
    /// The resident is a juror of a trial (`on`), or its duty ended.
    Jury { case: u64, kind: CrimeKind, deadline: u64, on: bool },
    /// Experience in the skill `law` for work at a court.
    Lesson { xp: i64 },
}

impl Message for Order {
    const KIND: &'static str = "crime.order";
}
