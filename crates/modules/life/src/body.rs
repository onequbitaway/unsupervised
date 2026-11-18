//! The per-resident state of the life module.

use serde::{Deserialize, Serialize};
use unsup_kernel::{Component, Pos, SiteId, ZoneId};

use crate::care::Care;
use crate::walk::Walk;

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// The place where a resident sleeps: the last home or shelter where it
/// slept, or the shelter where it appeared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bed {
    /// The site.
    #[serde(rename = "s")]
    pub site: SiteId,
    /// The zone of the site.
    #[serde(rename = "z")]
    pub zone: ZoneId,
    /// A tile inside the site.
    #[serde(rename = "p")]
    pub pos: Pos,
    /// Position of the kind of the site in `names::site::ALL`.
    #[serde(rename = "k")]
    pub(crate) kind: u8,
    /// True when the resident sleeps well there: it owns or rents the site.
    #[serde(rename = "g")]
    pub good: bool,
}

/// The state that the life module keeps for each resident. It moves with
/// the resident between zones.
///
/// Field names are short on the wire because one value is written for each
/// resident in every snapshot. A young resident that stands still and does
/// not sleep writes only its bed and the clock of its needs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    /// The walk in progress.
    #[serde(rename = "w", default, skip_serializing_if = "Option::is_none")]
    pub(crate) walk: Option<Box<Walk>>,
    /// The sleep, rest or hospital stay in progress.
    #[serde(rename = "c", default, skip_serializing_if = "Option::is_none")]
    pub(crate) care: Option<Box<Care>>,
    /// The place where the resident sleeps.
    #[serde(rename = "b", default, skip_serializing_if = "Option::is_none")]
    pub(crate) bed: Option<Bed>,
    /// True from the death of the resident until the kernel removes it at
    /// the end of the tick.
    #[serde(rename = "x", default, skip_serializing_if = "is_false")]
    pub(crate) dead: bool,
    /// Tick of the zone up to which the resident paid for its needs: each
    /// hourly change of the needs moves it one hour forward. It can be
    /// below zero in a young zone. While the resident moves between two
    /// zones, it is the number of ticks that are not paid yet.
    #[serde(rename = "n", default)]
    pub(crate) needs_at: i64,
    /// The last calendar day on which the resident had its chance to die
    /// of old age. Days count from the start of the world.
    #[serde(rename = "d", default, skip_serializing_if = "is_zero")]
    pub(crate) rolled_day: u32,
}

impl Body {
    /// Returns the place where the resident sleeps, if it has one.
    #[must_use]
    pub fn bed(&self) -> Option<Bed> {
        self.bed
    }

    /// Returns true when the resident walks.
    #[must_use]
    pub fn is_walking(&self) -> bool {
        self.walk.is_some()
    }
}

impl Component for Body {
    const NAME: &'static str = "life.body";
}
