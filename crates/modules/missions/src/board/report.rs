//! The zone side of the leaderboards: a report of the residents of the zone
//! and of the sites that residents own there.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::{Dest, Holder, Message, OrgId, OwnerId, Realm, ResidentId, Resource};

use crate::params::{self, Params};

/// Name of the site attribute with the price of a site in cents. The life
/// module writes it. See `docs/design/game-design.md`, section 9.
const SITE_VALUE: &str = "value";

/// What a zone tells the leaderboards about one resident.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Row {
    /// The resident.
    pub(crate) id: ResidentId,
    /// The owner of the resident.
    pub(crate) owner: OwnerId,
    /// The model that the owner declared, if it declared one.
    pub(crate) model: Option<String>,
    /// The frame in which the resident was spawned.
    pub(crate) born: u64,
    /// Cash on the person, in cents.
    pub(crate) cash: i64,
    /// The attribute `rep.standing`.
    pub(crate) standing: i64,
    /// The attribute `rep.notoriety`.
    pub(crate) notoriety: i64,
    /// The organizations of the resident, ordered by id.
    pub(crate) orgs: Vec<OrgId>,
}

/// What one zone tells the leaderboards. The zone sends it to the global
/// realm on a period.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct ZoneReport {
    /// The frame in which the zone made the report.
    pub(crate) frame: u64,
    /// Every resident of the zone.
    pub(crate) rows: Vec<Row>,
    /// The value of the sites of the zone that residents own: owner and
    /// cents, ordered by owner.
    pub(crate) sites: Vec<(ResidentId, i64)>,
}

impl Message for ZoneReport {
    const KIND: &'static str = "missions.zone_report";
}

/// What a zone remembers about its reports. A resource of each zone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Reporter {
    /// The number of the last period with a report: `frame / period`.
    period: u64,
}

impl Resource for Reporter {
    const NAME: &'static str = "missions.reporter";
}

/// Reads the residents and the sites of a zone into a report.
fn build(realm: &Realm, p: &Params) -> ZoneReport {
    let ledger = realm.ledger();
    let standing = realm.attrs(p.standing);
    let notoriety = realm.attrs(p.notoriety);
    let rows = realm
        .indices()
        .map(|idx| {
            let id = realm.id_of(idx);
            let who = realm.identity(idx);
            Row {
                id,
                owner: who.owner,
                model: who.model.clone(),
                born: who.born,
                cash: ledger.money(Holder::Resident(id)).cents(),
                standing: standing[idx.index()],
                notoriety: notoriety[idx.index()],
                orgs: realm.memberships(idx).iter().map(|m| m.org).collect(),
            }
        })
        .collect();
    let mut sites: BTreeMap<ResidentId, i64> = BTreeMap::new();
    for site in realm.sites().iter() {
        if let Some(Holder::Resident(owner)) = site.owner {
            let sum = sites.entry(owner).or_default();
            *sum = sum.saturating_add(site.attr(SITE_VALUE).max(0));
        }
    }
    ZoneReport { frame: realm.frame(), rows, sites: sites.into_iter().collect() }
}

/// Runs in every tick of a zone: sends one report in each period of
/// `missions.report_period` frames, in the first tick of the period that
/// the zone runs. The zones that run every frame thus report in the same
/// frame, and each resident is in exactly one report.
pub(crate) fn tick(realm: &Realm, p: &Params) {
    let period = realm.frame() / params::frames(realm, p.report_period);
    if realm.res::<Reporter>().period == period {
        return;
    }
    realm.res_mut::<Reporter>().period = period;
    realm.send(Dest::Global, &build(realm, p));
}
