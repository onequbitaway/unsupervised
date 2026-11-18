//! The state of the leaderboards: what the global realm knows about each
//! resident, and the form of a standing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use unsup_kernel::schemars::JsonSchema;
use unsup_kernel::serde_json::{Map, json};
use unsup_kernel::{OrgId, OwnerId, ResidentId, Resource, Value};

use super::report::ZoneReport;

/// A leaderboard.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[schemars(crate = "unsup_kernel::schemars")]
#[serde(rename_all = "snake_case")]
pub enum Board {
    /// Cash, bank deposit and the value of owned sites, in cents.
    Wealth,
    /// The attribute `rep.standing`.
    Influence,
    /// The attribute `rep.notoriety`.
    Notoriety,
    /// Arrests that the resident made in this season.
    Law,
    /// Points for completed missions in this season.
    Missions,
    /// Game days that the resident lived.
    Longevity,
}

impl Board {
    /// Every board, in the order of publication.
    pub const ALL: [Board; 6] = [
        Board::Wealth,
        Board::Influence,
        Board::Notoriety,
        Board::Law,
        Board::Missions,
        Board::Longevity,
    ];

    /// Returns the name of the board. It is the `key` of the published
    /// event.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Board::Wealth => "wealth",
            Board::Influence => "influence",
            Board::Notoriety => "notoriety",
            Board::Law => "law",
            Board::Missions => "missions",
            Board::Longevity => "longevity",
        }
    }

    /// Returns the unit of the scores of the board.
    #[must_use]
    pub fn unit(self) -> &'static str {
        match self {
            Board::Wealth => "cents",
            Board::Influence => "standing",
            Board::Notoriety => "notoriety",
            Board::Law => "arrests",
            Board::Missions => "points",
            Board::Longevity => "days",
        }
    }

    /// Returns true when the board ranks only residents that are in the
    /// world. The other boards also rank the residents that left the world
    /// in this season.
    pub(crate) fn living_only(self) -> bool {
        matches!(self, Board::Wealth | Board::Influence | Board::Notoriety)
    }
}

/// What the leaderboards know about one resident.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Record {
    /// The owner of the resident.
    pub(crate) owner: OwnerId,
    /// The model that the owner declared, if it declared one.
    pub(crate) model: Option<String>,
    /// The organizations of the resident, ordered by id.
    pub(crate) orgs: Vec<OrgId>,
    /// The frame in which the resident was spawned.
    pub(crate) born: u64,
    /// Cash on the person, in cents.
    pub(crate) cash: i64,
    /// The attribute `rep.standing`.
    pub(crate) standing: i64,
    /// The attribute `rep.notoriety`.
    pub(crate) notoriety: i64,
    /// The frame of the newest report with the resident. 0: no report yet.
    pub(crate) reported: u64,
    /// The frame in which the resident left the world. 0: it is in the
    /// world.
    pub(crate) gone: u64,
    /// Mission points of this season.
    pub(crate) points: i64,
    /// Arrests that the resident made in this season.
    pub(crate) arrests: i64,
    /// The game day of `points_today`.
    pub(crate) points_day: u64,
    /// Points from missions of residents on that day.
    pub(crate) points_today: i64,
}

/// One resident in the top list of a board.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    /// Place on the board, from 1.
    pub rank: u32,
    /// The resident.
    pub resident: ResidentId,
    /// Display name of the resident.
    pub name: String,
    /// The owner of the resident.
    pub owner: OwnerId,
    /// The model that the owner declared. Nobody checked it.
    pub model: Option<String>,
    /// The score, in the unit of the board.
    pub score: i64,
}

/// One group in a list of a board: an owner, an organization or a model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group<K> {
    /// Place in the list, from 1.
    pub rank: u32,
    /// The owner, the organization or the model.
    pub key: K,
    /// Sum of the scores of the residents of the group.
    pub score: i64,
    /// Number of residents of the group that the board ranks.
    pub residents: u64,
    /// `score / residents`, rounded down.
    pub mean: i64,
}

impl<K: Serialize> Group<K> {
    /// Returns the group as JSON. `name` is the name of the key field:
    /// `owner`, `org` or `model`.
    fn to_value(&self, name: &str) -> Value {
        let mut out = Map::new();
        out.insert("rank".into(), json!(self.rank));
        out.insert(name.into(), json!(self.key));
        out.insert("score".into(), json!(self.score));
        out.insert("residents".into(), json!(self.residents));
        out.insert("mean".into(), json!(self.mean));
        Value::Object(out)
    }
}

/// A board at one moment: the top residents and the best groups.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Standing {
    /// Number of residents that the board ranks.
    pub residents: u64,
    /// The best residents, best first.
    pub top: Vec<Entry>,
    /// The best owners. The score of an owner is the sum of the scores of
    /// its residents.
    pub by_owner: Vec<Group<OwnerId>>,
    /// The best organizations. The score of an organization is the sum of
    /// the scores of its members.
    pub by_org: Vec<Group<OrgId>>,
    /// The best models. The key is the model that the owner declared, or
    /// nothing for the residents without one. Nobody checked it.
    pub by_model: Vec<Group<Option<String>>>,
}

impl Standing {
    /// Returns the top residents as JSON.
    pub(crate) fn top_value(&self) -> Value {
        json!(self.top)
    }

    /// Returns the best owners as JSON.
    pub(crate) fn owners_value(&self) -> Value {
        Value::Array(self.by_owner.iter().map(|group| group.to_value("owner")).collect())
    }

    /// Returns the best organizations as JSON.
    pub(crate) fn orgs_value(&self) -> Value {
        Value::Array(self.by_org.iter().map(|group| group.to_value("org")).collect())
    }

    /// Returns the best models as JSON.
    pub(crate) fn models_value(&self) -> Value {
        Value::Array(self.by_model.iter().map(|group| group.to_value("model")).collect())
    }

    /// Returns the standing of a board as JSON: the data of a publication
    /// without the `key`.
    pub(crate) fn to_value(&self, board: Board) -> Value {
        json!({
            "unit": board.unit(),
            "residents": self.residents,
            // Every `model` is the text that the owner of a resident gave.
            "model_source": "self_reported",
            "top": self.top_value(),
            "by_owner": self.owners_value(),
            "by_org": self.orgs_value(),
            "by_model": self.models_value(),
        })
    }
}

/// The leaderboards. A resource of the global realm.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Boards {
    /// What the boards know about each resident.
    pub(crate) records: BTreeMap<ResidentId, Record>,
    /// The value of the sites that residents own in each zone, as the zone
    /// reported it: by zone, then owner and cents.
    pub(crate) zones: BTreeMap<u32, Vec<(ResidentId, i64)>>,
    /// The value of the sites of each resident in all zones, in cents.
    pub(crate) site_value: BTreeMap<ResidentId, i64>,
}

impl Resource for Boards {
    const NAME: &'static str = "missions.boards";
}

impl Boards {
    /// Returns the number of residents that the boards know.
    #[must_use]
    pub fn residents(&self) -> usize {
        self.records.len()
    }

    /// Returns the mission points of a resident in this season.
    #[must_use]
    pub fn points(&self, resident: ResidentId) -> i64 {
        self.records.get(&resident).map_or(0, |record| record.points)
    }

    /// Returns the arrests that a resident made in this season.
    #[must_use]
    pub fn arrests(&self, resident: ResidentId) -> i64 {
        self.records.get(&resident).map_or(0, |record| record.arrests)
    }

    /// Takes the report of a zone. A report that is older than what the
    /// boards know about a resident does not change that resident: the
    /// resident moved to a zone that reported later.
    pub(crate) fn apply(&mut self, zone: u32, report: ZoneReport) {
        for row in report.rows {
            let record = self.records.entry(row.id).or_default();
            if record.gone != 0 || record.reported > report.frame {
                continue;
            }
            record.owner = row.owner;
            record.model = row.model;
            record.orgs = row.orgs;
            record.born = row.born;
            record.cash = row.cash;
            record.standing = row.standing;
            record.notoriety = row.notoriety;
            record.reported = report.frame;
        }
        let before = self.zones.insert(zone, report.sites.clone()).unwrap_or_default();
        for (owner, cents) in before {
            let left = self.site_value.get(&owner).copied().unwrap_or(0) - cents;
            if left == 0 {
                self.site_value.remove(&owner);
            } else {
                self.site_value.insert(owner, left);
            }
        }
        for (owner, cents) in report.sites {
            let sum = self.site_value.get(&owner).copied().unwrap_or(0).saturating_add(cents);
            if sum == 0 {
                self.site_value.remove(&owner);
            } else {
                self.site_value.insert(owner, sum);
            }
        }
    }

    /// Records that a resident left the world. Its wealth and its
    /// reputation count no more; its points, its arrests and its age stay
    /// until the end of the season.
    pub(crate) fn mark_gone(&mut self, resident: ResidentId, frame: u64) {
        if let Some(record) = self.records.get_mut(&resident)
            && record.gone == 0
        {
            record.gone = frame.max(1);
            record.cash = 0;
            record.standing = 0;
            record.notoriety = 0;
        }
    }

    /// Starts the boards of a new season: the points and the arrests go
    /// back to zero, and the residents that left the world are forgotten.
    pub(crate) fn start_season(&mut self) {
        self.records.retain(|_, record| record.gone == 0);
        for record in self.records.values_mut() {
            record.points = 0;
            record.arrests = 0;
            record.points_day = 0;
            record.points_today = 0;
        }
    }
}
